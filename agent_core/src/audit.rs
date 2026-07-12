use crate::{
    bash_approval_mode_label, ApiProtocol, ApprovalRequest, BashApprovalMode, MemGuard,
    TurnStopSummary, UsageStats,
};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const AUDIT_SIDECAR_THRESHOLD_BYTES: u64 = 1024 * 1024;
const REPAIR_OUTPUT_RESPONSE_LIMIT_CHARS: usize = 12_000;

pub fn append_audit_event(path: &Path, event: &Value) -> std::io::Result<()> {
    MemGuard::for_audit_file(path)
        .with_write(|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if should_append_audit_sidecar(path) {
                return append_audit_jsonl(&audit_sidecar_path(path), event);
            }
            let mut doc = read_audit_doc(path)?;
            doc["events"]
                .as_array_mut()
                .expect("audit doc events must be an array")
                .push(event.clone());
            let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
            fs::write(path, format!("{text}\n"))
        })
        .map_err(std::io::Error::other)?
}

pub fn append_repair_output_event(api_audit_file: &Path, event: &Value) -> std::io::Result<()> {
    let repair_file = repair_output_file_for_api_audit(api_audit_file);
    MemGuard::for_audit_file(&repair_file)
        .with_write(|| {
            if let Some(parent) = repair_file.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut doc = read_repair_output_doc(&repair_file)?;
            doc["records"]
                .as_array_mut()
                .expect("repair output doc records must be an array")
                .push(event.clone());
            doc["updated_at_ms"] = json!(audit_now_ms());
            let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
            fs::write(&repair_file, format!("{text}\n"))
        })
        .map_err(std::io::Error::other)?
}

fn repair_output_file_for_api_audit(api_audit_file: &Path) -> std::path::PathBuf {
    api_audit_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("api_output_repair.json")
}

fn read_repair_output_doc(path: &Path) -> std::io::Result<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(empty_repair_output_doc());
    };
    if text.trim().is_empty() {
        return Ok(empty_repair_output_doc());
    }
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return Ok(empty_repair_output_doc());
    };
    if value.get("records").and_then(Value::as_array).is_none() {
        value["records"] = json!([]);
    }
    if value.get("version").is_none() {
        value["version"] = json!(1);
    }
    Ok(value)
}

fn empty_repair_output_doc() -> Value {
    json!({
        "version": 1,
        "kind": "timem_realtime_repair_output_log",
        "notes": [
            "Realtime model-output protocol repair diagnostics.",
            "Each record includes the malformed assistant response and the SYSTEM repair message shown to the model.",
            "assistant_response may be capped to avoid unbounded diagnostic growth."
        ],
        "records": []
    })
}

pub fn read_audit_doc(path: &Path) -> std::io::Result<Value> {
    let mut doc = read_audit_doc_single(path)?;
    let sidecar = audit_sidecar_path(path);
    if sidecar != path {
        let sidecar_doc = read_audit_doc_single(&sidecar)?;
        if let (Some(base), Some(extra)) = (
            doc["events"].as_array_mut(),
            sidecar_doc["events"].as_array(),
        ) {
            base.extend(extra.iter().cloned());
        }
    }
    Ok(doc)
}

fn read_audit_doc_single(path: &Path) -> std::io::Result<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(empty_audit_doc());
    };
    if text.trim().is_empty() {
        return Ok(empty_audit_doc());
    }
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if value.get("events").and_then(Value::as_array).is_some() {
            return Ok(value);
        }
    }
    let events = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    Ok(json!({"version": 1, "events": events}))
}

fn should_append_audit_sidecar(path: &Path) -> bool {
    if audit_sidecar_path(path).exists() {
        return true;
    }
    fs::metadata(path)
        .map(|metadata| metadata.len() >= AUDIT_SIDECAR_THRESHOLD_BYTES)
        .unwrap_or(false)
}

fn append_audit_jsonl(path: &Path, event: &Value) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event).map_err(std::io::Error::other)?;
    file.write_all(b"\n")
}

fn audit_sidecar_path(path: &Path) -> std::path::PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.with_extension("jsonl");
    };
    path.with_file_name(format!("{file_name}l"))
}

fn empty_audit_doc() -> Value {
    json!({"version": 1, "events": []})
}

pub fn host_start_audit_event(
    host: &str,
    session: &str,
    space: &str,
    gateway_provider: &str,
    base_url: &str,
    api_protocol: &ApiProtocol,
    model: &str,
    max_llm_input_tokens: u32,
    bash_approval: BashApprovalMode,
) -> Value {
    json!({
        "type": format!("{host}_start"),
        "session": session,
        "space": space,
        "gateway_provider": gateway_provider,
        "provider": gateway_provider,
        "base_url": base_url,
        "api_protocol": api_protocol.label(),
        "model": model,
        "max_llm_input_tokens": max_llm_input_tokens,
        "bash_approval": bash_approval_mode_label(bash_approval),
    })
}

pub fn turn_start_audit_event(session: &str, turn_id: &str, user_input: &str) -> Value {
    json!({
        "type": "turn_start",
        "session": session,
        "turn_id": turn_id,
        "user_input": user_input,
    })
}

pub fn user_supplement_audit_event(session: &str, turn_id: &str, text: &str) -> Value {
    json!({
        "type": "user_supplement",
        "session": session,
        "turn_id": turn_id,
        "text": text,
    })
}

pub fn max_llm_output_increased_audit_event(
    session: &str,
    turn_id: &str,
    max_llm_output_tokens: u32,
) -> Value {
    json!({
        "type": "max_llm_output_increased",
        "session": session,
        "turn_id": turn_id,
        "max_llm_output_tokens": max_llm_output_tokens,
    })
}

pub fn model_repair_request_audit_event(
    session: &str,
    turn_id: &str,
    issue: Option<&str>,
    model: &str,
    usage: &UsageStats,
    truncated: bool,
    repair_calls: u32,
    repair_calls_delta: u32,
) -> Value {
    json!({
        "type": "model_repair_request",
        "session": session,
        "turn_id": turn_id,
        "issue": issue,
        "model": model,
        "usage": usage,
        "truncated": truncated,
        "repair_calls": repair_calls,
        "repair_calls_delta": repair_calls_delta,
    })
}

pub fn model_repair_output_event(
    session: &str,
    turn_id: &str,
    issue: Option<&str>,
    assistant_name: &str,
    assistant_response: &str,
    system_message: &str,
    model: &str,
    usage: &UsageStats,
    truncated: bool,
    repair_calls: u32,
    repair_calls_delta: u32,
) -> Value {
    let (assistant_response, capped) =
        cap_repair_output_text(assistant_response, REPAIR_OUTPUT_RESPONSE_LIMIT_CHARS);
    let time_ms = audit_now_ms();
    let issue_text = issue.unwrap_or("unknown_repair_issue");
    json!({
        "kind": "model_output_repair",
        "time_ms": time_ms,
        "session": session,
        "turn_id": turn_id,
        "issue": issue,
        "assistant_name": assistant_name,
        "assistant_response": assistant_response,
        "assistant_response_capped": capped,
        "system_message": system_message,
        "model": model,
        "usage": usage,
        "truncated": truncated,
        "repair_calls": repair_calls,
        "repair_calls_delta": repair_calls_delta,
        "rendered": format!(
            "---- {} / {} ----\n## assistant:\n{}\n\n## SYSTEM\n{}",
            time_ms, turn_id, assistant_response, system_message
        ),
        "summary": format!("{} repair for {}", issue_text, turn_id),
    })
}

fn cap_repair_output_text(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        return (text.to_string(), false);
    }
    let head_count = limit / 2;
    let tail_count = limit.saturating_sub(head_count);
    let head = text.chars().take(head_count).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (
        format!(
            "{head}\n[TRUNCATED repair output: omitted middle chars; original_chars={}]\n{tail}",
            text.chars().count()
        ),
        true,
    )
}

fn audit_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn turn_error_audit_event(session: &str, turn_id: &str, error: &str) -> Value {
    json!({
        "type": "turn_error",
        "session": session,
        "turn_id": turn_id,
        "error": error,
    })
}

pub fn user_approval_audit_event(
    session: &str,
    turn_id: &str,
    approval: &ApprovalRequest,
    approved: bool,
) -> Value {
    json!({
        "type": "user_approval",
        "session": session,
        "turn_id": turn_id,
        "approval_id": approval.approval_id,
        "action": approval.action,
        "command": approval.command,
        "risk": approval.risk,
        "reason": approval.reason,
        "approved": approved,
    })
}

pub fn round_limit_audit_event(
    session: &str,
    turn_id: &str,
    max_rounds: u32,
    continued: bool,
) -> Value {
    json!({
        "type": "round_limit",
        "session": session,
        "turn_id": turn_id,
        "max_rounds": max_rounds,
        "continued": continued,
    })
}

pub fn stale_context_choice_audit_event(
    session: &str,
    idle: Duration,
    dynamic_context_tokens: u32,
    continue_old_context: bool,
) -> Value {
    json!({
        "type": "stale_context_choice",
        "session": session,
        "idle_secs": idle.as_secs(),
        "dynamic_context_tokens": dynamic_context_tokens,
        "continue_old_context": continue_old_context,
    })
}

pub fn turn_final_audit_event(
    session: &str,
    turn_id: &str,
    assistant_output: &str,
    stats: &UsageStats,
    latest_usage: Option<&UsageStats>,
    repair_issue: Option<&str>,
    stop_summary: Option<&TurnStopSummary>,
    elapsed: Duration,
) -> Value {
    json!({
        "type": "turn_final",
        "session": session,
        "turn_id": turn_id,
        "assistant_output": assistant_output,
        "stats": stats,
        "latest_usage": latest_usage,
        "repair_issue": repair_issue,
        "stop_summary": stop_summary,
        "elapsed_ms": elapsed.as_millis(),
    })
}

pub fn model_retry_audit_event(
    session: &str,
    turn_id: &str,
    attempt: u32,
    max_attempts: u32,
    delay: Duration,
    error: &str,
) -> Value {
    json!({
        "type": "model_retry",
        "session": session,
        "turn_id": turn_id,
        "attempt": attempt,
        "max_attempts": max_attempts,
        "delay_ms": delay.as_millis(),
        "error": error,
    })
}

pub fn model_input_overflow_recovery_audit_event(
    session: &str,
    turn_id: &str,
    removed_delta_id: &str,
    removed_action_output_bytes: usize,
    error: &str,
) -> Value {
    json!({
        "type": "model_input_overflow_recovery",
        "session": session,
        "turn_id": turn_id,
        "removed_delta_id": removed_delta_id,
        "removed_action_output_bytes": removed_action_output_bytes,
        "error": error,
    })
}

#[cfg(test)]
#[path = "../tests/unit/audit_tests.rs"]
mod tests;

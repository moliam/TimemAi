use crate::{
    atomic_write_file, bash_approval_mode_label, ApiProtocol, ApprovalRequest, BashApprovalMode,
    MemGuard, TurnStopSummary, UsageStats,
};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const AUDIT_SIDECAR_THRESHOLD_BYTES: u64 = 1024 * 1024;
const REPAIR_OUTPUT_RESPONSE_LIMIT_CHARS: usize = 12_000;

pub fn append_audit_event(path: &Path, event: &Value) -> std::io::Result<()> {
    MemGuard::for_audit_file(path)
        .with_write(|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let now_ms = audit_now_ms();
            let event = timestamp_audit_event(event, now_ms);
            if should_append_audit_sidecar(path) {
                return append_audit_jsonl(&audit_sidecar_path(path), &event);
            }
            let mut doc = read_audit_doc(path)?;
            doc["events"]
                .as_array_mut()
                .expect("audit doc events must be an array")
                .push(event);
            let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
            atomic_write_file(path, format!("{text}\n").as_bytes())
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
            atomic_write_file(&repair_file, format!("{text}\n").as_bytes())
        })
        .map_err(std::io::Error::other)?
}

fn timestamp_audit_event(event: &Value, now_ms: i64) -> Value {
    let mut event = event.clone();
    if let Some(object) = event.as_object_mut() {
        let valid_time = object
            .get("time_ms")
            .and_then(Value::as_i64)
            .is_some_and(|time_ms| time_ms <= now_ms);
        if !valid_time {
            object.insert("time_ms".into(), json!(now_ms));
        }
        return event;
    }
    json!({"time_ms": now_ms, "event": event})
}

/// Removes API-audit events older than `cutoff_ms` from the JSON document and
/// its JSONL sidecar. Events at the cutoff are retained.
pub fn prune_api_audit_before(path: &Path, cutoff_ms: i64, now_ms: i64) -> std::io::Result<usize> {
    MemGuard::for_audit_file(path)
        .with_write(|| prune_api_audit_before_unlocked(path, cutoff_ms, now_ms))
        .map_err(std::io::Error::other)?
}

fn prune_api_audit_before_unlocked(
    path: &Path,
    cutoff_ms: i64,
    now_ms: i64,
) -> std::io::Result<usize> {
    let mut removed = 0usize;
    if path.exists() {
        let mut doc = read_audit_doc_single(path)?;
        let events = doc["events"]
            .as_array_mut()
            .expect("audit doc events must be an array");
        let before = events.len();
        events.retain(|event| retained_audit_time_ms(event, cutoff_ms, now_ms).is_some());
        removed = removed.saturating_add(before.saturating_sub(events.len()));
        let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
        atomic_write_file(path, format!("{text}\n").as_bytes())?;
    }
    let sidecar = audit_sidecar_path(path);
    if sidecar != path && sidecar.exists() {
        removed = removed.saturating_add(prune_audit_jsonl(&sidecar, cutoff_ms, now_ms)?);
    }
    Ok(removed)
}

fn retained_audit_time_ms(event: &Value, cutoff_ms: i64, now_ms: i64) -> Option<i64> {
    let time_ms = event.get("time_ms").and_then(Value::as_i64)?;
    if !(cutoff_ms..=now_ms).contains(&time_ms) {
        return None;
    }
    Some(time_ms)
}

fn prune_audit_jsonl(path: &Path, cutoff_ms: i64, now_ms: i64) -> std::io::Result<usize> {
    let input = fs::File::open(path)?;
    let temporary = audit_retention_temp_path(path);
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let output = options.open(&temporary)?;
    let mut writer = BufWriter::new(output);
    let mut removed = 0usize;

    let result = (|| -> std::io::Result<()> {
        let mut reader = BufReader::new(input);
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            let Ok(event) = serde_json::from_slice::<Value>(&line) else {
                removed = removed.saturating_add(1);
                continue;
            };
            if retained_audit_time_ms(&event, cutoff_ms, now_ms).is_none() {
                removed = removed.saturating_add(1);
                continue;
            }
            serde_json::to_writer(&mut writer, &event).map_err(std::io::Error::other)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(removed)
}

fn audit_retention_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("api_audit.jsonl");
    path.with_file_name(format!(
        ".{file_name}.retention.tmp-{}-{}",
        std::process::id(),
        audit_now_ms()
    ))
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
            "Each record includes the malformed assistant response and the RUNTIME repair message shown to the model.",
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

#[allow(clippy::too_many_arguments)]
pub fn host_start_audit_event(
    host: &str,
    session: &str,
    space: &str,
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

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::too_many_arguments)]
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
    spec: &crate::response_protocol::PromptBoundarySpec,
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
            "---- {} / {} ----\n## {}:\n{}\n\n## {}\n{}",
            time_ms, turn_id, spec.assistant_role, assistant_response, spec.runtime_role, system_message
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

#[allow(clippy::too_many_arguments)]
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

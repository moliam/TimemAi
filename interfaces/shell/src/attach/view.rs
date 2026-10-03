//! Pure terminal presentation for `timem attach`.
//!
//! This module maps already-authoritative Host/session data into concise
//! terminal copy. It does not own connection state, command admission, or Turn
//! lifecycle decisions.

use super::{AttachError, AttachHostCandidate, AttachSession};
use crate::{local_time_label, ANSI_BOLD, ANSI_BRIGHT_TIMEM, ANSI_DIM, ANSI_RESET, TIMEM_LOGO};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

const ANSI_FAIL: &str = "\x1b[31m";
const ANSI_WARN: &str = "\x1b[33m";
const ANSI_INFO: &str = "\x1b[36m";

pub(super) fn guidance_card(title: &str, detail: &str, next: &str) -> String {
    let detail = detail.trim();
    let detail_line = if detail.is_empty() {
        String::new()
    } else {
        detail
            .lines()
            .map(|line| format!("\n  {line}"))
            .collect::<String>()
    };
    format!("{ANSI_BOLD}{title}{ANSI_RESET}{detail_line}\n  {ANSI_INFO}Next:{ANSI_RESET} {next}")
}

pub(super) fn attach_error_card(error: &AttachError) -> String {
    guidance_card("Unable to attach", &error.message(), error.next_step())
}

pub(super) fn no_sessions_card() -> String {
    guidance_card(
        "No sessions available",
        "The Web Host is running, but it has no session to attach to.",
        "Create or open a session in Timem Web, then run `timem attach` again.",
    )
}

pub(super) fn disconnected_card(detail: Option<&str>) -> String {
    guidance_card(
        "Detached from session",
        detail.unwrap_or("The connection to the Web Host closed."),
        "Run `timem attach` to reconnect. Work already accepted by the Host continues there.",
    )
}

pub(super) fn rejected_command_card(error: &str) -> String {
    let detail = match error {
        "session_restart_cwd_decision_required" => {
            "This session needs a working-directory choice before it can continue."
        }
        "unknown" | "" => "The Host rejected the command without additional detail.",
        other => other,
    };
    guidance_card(
        "Command not accepted",
        detail,
        "Review the current session prompt and try one of the displayed choices.",
    )
}

pub(super) fn host_error_card(error: &str) -> String {
    guidance_card(
        "Host reported a problem",
        error,
        "Check the session state in Timem Web, then retry or reconnect.",
    )
}

pub(super) fn invalid_command_card(command: &str) -> String {
    guidance_card(
        "Unknown attach command",
        command,
        "Use `!c` to cancel the active turn, `!s` to stop the session, or answer a visible choice with its number.",
    )
}

pub(super) fn invalid_restart_choice_card(choice: &str) -> String {
    guidance_card(
        "Invalid working-directory choice",
        &format!("`{choice}` is not available for this prompt."),
        "Choose `1` to keep the session directory or `2` to use the Host directory.",
    )
}

fn pad_display(value: &str, width: usize) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(value));
    format!("{value}{}", " ".repeat(padding))
}

pub(super) fn instance_selector(candidates: &[AttachHostCandidate], selected: usize) -> String {
    let pid_width = candidates
        .iter()
        .map(|candidate| candidate.pid.to_string().len())
        .max()
        .unwrap_or(1);
    let port_width = candidates
        .iter()
        .map(|candidate| candidate.host.port.to_string().len())
        .max()
        .unwrap_or(1);
    let mut out = format!(
        "{ANSI_BOLD}Select a Timem instance{ANSI_RESET} {ANSI_DIM}(↑/↓ move · Enter continue · Esc exit){ANSI_RESET}\r\n"
    );
    for (index, candidate) in candidates.iter().enumerate() {
        let marker = if index == selected {
            format!("{ANSI_BRIGHT_TIMEM}❯{ANSI_RESET}")
        } else {
            " ".to_string()
        };
        out.push_str(&format!(
            "{marker} {memory}  {ANSI_DIM}PID {pid:>pid_width$}  port {port:>port_width$}  started {started}{ANSI_RESET}\r\n",
            memory = candidate.memory_dir.display(),
            pid = candidate.pid,
            port = candidate.host.port,
            started = candidate.started_at_ms,
        ));
    }
    out
}

pub(super) fn session_selector(sessions: &[AttachSession], selected: usize) -> String {
    let ordinal_width = sessions
        .iter()
        .map(|session| session.ordinal.to_string().len())
        .max()
        .unwrap_or(1);
    let name_width = sessions
        .iter()
        .map(|session| UnicodeWidthStr::width(session.display_name.as_str()))
        .max()
        .unwrap_or(1);
    let worker_width = sessions
        .iter()
        .map(|session| session.worker_count.to_string().len())
        .max()
        .unwrap_or(1);

    let mut out = format!(
        "{ANSI_BOLD}Select a session{ANSI_RESET} {ANSI_DIM}(↑/↓ move · Enter attach · Esc exit){ANSI_RESET}\r\n"
    );
    for (index, session) in sessions.iter().enumerate() {
        let marker = if index == selected {
            format!("{ANSI_BRIGHT_TIMEM}❯{ANSI_RESET}")
        } else {
            " ".to_string()
        };
        let ordinal = format!("{:>width$}", session.ordinal, width = ordinal_width);
        let name = pad_display(&session.display_name, name_width);
        let state = if session.working { "working" } else { "idle" };
        let workers = format!("{:>width$}", session.worker_count, width = worker_width);
        let name = if session.working {
            format!("{ANSI_BOLD}{name}{ANSI_RESET}")
        } else {
            name
        };
        out.push_str(&format!(
            "{marker} {ANSI_DIM}{ordinal}{ANSI_RESET}  {name}  {ANSI_DIM}{state:<7}  workers {workers}  {path}{ANSI_RESET}\r\n",
            path = session.current_dir,
        ));
    }
    out
}

pub(super) fn connected_intro(session: &AttachSession, working: bool) -> String {
    let state = if working { "Working" } else { "Idle" };
    format!(
        "{ANSI_BRIGHT_TIMEM}{TIMEM_LOGO}{ANSI_RESET} {ANSI_BOLD}attach{ANSI_RESET}\n  {ANSI_BOLD}{}{ANSI_RESET}  {ANSI_DIM}{}{ANSI_RESET}\n  State: {state} · Workers: {}\n  Directory: {}\n\n{ANSI_DIM}Input while idle starts a new turn; input while working adds a supplement.\nChoices use 1/2(/3).  !c cancels the active turn · !s stops the session · Ctrl+C detaches only.{ANSI_RESET}",
        session.display_name, session.session_id, session.worker_count, session.current_dir,
    )
}

pub(super) fn decision_request_prompt(payload: &Value) -> Option<String> {
    let topic = payload.get("topic")?;
    if topic
        .get("attributes")
        .and_then(|attributes| attributes.get("expects_reply"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let topic_name = topic
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let request = payload.get("payload")?.get("request")?;
    let (detail, allow_always) = match topic_name {
        "core.user.approval.request" => {
            let action = request
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("run");
            let command = request.get("command").and_then(Value::as_str).unwrap_or("");
            let risk = request
                .get("risk")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            (format!("Approve {action}: {command}\nRisk: {risk}"), true)
        }
        "core.user.round_limit.request" => {
            let rounds = request
                .get("recharge_rounds")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (format!("Continue for {rounds} more rounds?"), false)
        }
        "core.user.output_expand.request" => {
            let tokens = request
                .get("increment_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (format!("Expand output by {tokens} tokens?"), false)
        }
        "core.user.stale_context.request" => {
            let idle_ms = request.get("idle_ms").and_then(Value::as_u64).unwrap_or(0);
            (
                format!("Continue with context idle for {idle_ms} ms?"),
                false,
            )
        }
        "core.work_instruction_load" => {
            let directory = request
                .get("directory")
                .and_then(Value::as_str)
                .unwrap_or("unknown directory");
            (format!("Load work instructions from {directory}?"), false)
        }
        "core.user.long_running_command.request" => {
            let action = request
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("command");
            let command = request.get("command").and_then(Value::as_str).unwrap_or("");
            let elapsed_ms = request
                .get("elapsed_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (
                format!("{action} is still running after {elapsed_ms} ms: {command}"),
                false,
            )
        }
        _ => (
            "The Host needs a decision before work can continue.".to_string(),
            false,
        ),
    };
    let third = if allow_always {
        "\n  3  Always allow"
    } else {
        ""
    };
    let aliases = if allow_always {
        "!y / !n / !a"
    } else {
        "!y / !n"
    };
    Some(format!(
        "\n{ANSI_WARN}{ANSI_BOLD}Action required{ANSI_RESET}\n  {}\n\n  {ANSI_BOLD}1{ANSI_RESET}  Accept\n  {ANSI_BOLD}2{ANSI_RESET}  Decline{third}\n  {ANSI_DIM}Aliases: {aliases}{ANSI_RESET}",
        detail.replace('\n', "\n  "),
    ))
}

pub(super) fn restart_cwd_prompt(decision: &Value) -> Option<String> {
    let runtime_cwd = decision.get("runtime_cwd")?.as_str()?;
    let session_cwd = decision
        .get("session_cwd")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let available = decision
        .get("session_cwd_available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let keep_note = if available { "" } else { "  (unavailable)" };
    let aliases = if available { "!k / !r" } else { "!r" };
    Some(format!(
        "\n{ANSI_WARN}{ANSI_BOLD}Action required{ANSI_RESET}\n  Choose the working directory for this restored session.\n\n  Session: {session_cwd}\n  Host:    {runtime_cwd}\n\n  {ANSI_BOLD}1{ANSI_RESET}  Keep session directory{keep_note}\n  {ANSI_BOLD}2{ANSI_RESET}  Use host directory\n  {ANSI_DIM}Aliases: {aliases}{ANSI_RESET}\n\n{ANSI_BRIGHT_TIMEM}{ANSI_BOLD}choice ❯❯{ANSI_RESET} "
    ))
}

pub(super) fn format_user_echo(text: &str) -> String {
    format!(
        "\x1b[94;1m[{}] You ❯❯{ANSI_RESET} {text}",
        local_time_label()
    )
}

pub(super) fn truncate_line(text: &str, max_chars: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max_chars {
        return one_line;
    }
    let truncated: String = one_line.chars().take(max_chars).collect();
    format!("{truncated}...")
}

fn humanize(value: &str) -> String {
    let mut words = value
        .split(['_', '.'])
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(first) = words.first_mut() {
        if let Some(initial) = first.get_mut(0..1) {
            initial.make_ascii_uppercase();
        }
    }
    words.join(" ")
}

fn action_name(action: &str) -> String {
    match action {
        "run_bash" => "Run command".to_string(),
        "run_powershell" => "Run PowerShell".to_string(),
        "readfile" => "Read file".to_string(),
        "memmgr" => "Memory".to_string(),
        "memo" => "Work memo".to_string(),
        "self_tool" => "Runtime info".to_string(),
        "context_compress" => "Compact context".to_string(),
        other => humanize(other),
    }
}

pub(super) fn action_detail(payload: &Value) -> Option<String> {
    let action = payload.get("action").and_then(Value::as_str)?;
    if matches!(action, "task_finished" | "turn_finished") {
        return None;
    }
    let event = payload.get("event").and_then(Value::as_str).unwrap_or("");
    let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
    let input = payload.get("input");
    let detail = match action {
        "run_bash" | "run_powershell" => input
            .and_then(|value| {
                value
                    .get("cmd")
                    .or_else(|| value.get("loop_cmd"))
                    .and_then(Value::as_str)
            })
            .map(|command| format!("`{}`", truncate_line(command, 120)))
            .unwrap_or_default(),
        "readfile" => input
            .and_then(|value| value.get("path"))
            .and_then(Value::as_str)
            .map(|path| truncate_line(path, 120))
            .unwrap_or_default(),
        "memmgr" => input
            .and_then(|value| value.get("op"))
            .and_then(Value::as_str)
            .map(humanize)
            .unwrap_or_default(),
        "memo" => input
            .and_then(|value| value.get("op"))
            .and_then(Value::as_str)
            .map(humanize)
            .unwrap_or_default(),
        "self_tool" => input
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            .map(humanize)
            .unwrap_or_default(),
        _ => String::new(),
    };
    let failed = status.contains("error") || status.contains("fail");
    let (symbol, symbol_style) = if failed {
        ("×", ANSI_FAIL)
    } else if event == "finish" {
        ("✓", ANSI_BRIGHT_TIMEM)
    } else {
        ("›", ANSI_INFO)
    };
    let status_label = if status.is_empty() || status == "running" {
        String::new()
    } else {
        format!(" · {}", humanize(status))
    };
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" · {detail}")
    };
    Some(format!(
        "{symbol_style}{symbol}{ANSI_RESET} {ANSI_BOLD}{}{ANSI_RESET}{detail}{status_label}",
        action_name(action),
    ))
}

pub(super) fn topic_summary(payload: &Value) -> Option<String> {
    if decision_request_prompt(payload).is_some() {
        return None;
    }
    let topic = payload
        .get("topic")
        .and_then(|topic| topic.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let data = payload.get("payload").unwrap_or(payload);
    match topic {
        "core.action" => action_detail(data),
        "core.model.preview" | "core.lifecycle" => None,
        "core.model.response" => {
            let final_answer = data
                .get("final_answer")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !final_answer.trim().is_empty() {
                return None;
            }
            let detail = ["free_talk", "progress"]
                .iter()
                .filter_map(|field| data.get(*field).and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            (!detail.is_empty()).then(|| {
                format!(
                    "{ANSI_INFO}Thinking{ANSI_RESET} · {}",
                    truncate_line(&detail, 180)
                )
            })
        }
        "core.model.repair" => {
            let reason = data
                .get("reason")
                .or_else(|| data.get("issue"))
                .and_then(Value::as_str)
                .unwrap_or("The model response did not match the protocol.");
            let attempt = data.get("attempt").and_then(Value::as_u64);
            let max = data.get("max_attempts").and_then(Value::as_u64);
            let progress = match (attempt, max) {
                (Some(attempt), Some(max)) => format!(" · attempt {attempt}/{max}"),
                _ => String::new(),
            };
            Some(format!(
                "{ANSI_WARN}Model response repair{ANSI_RESET}{progress} · {}",
                truncate_line(reason, 160)
            ))
        }
        "core.context.compress" => Some(format!("{ANSI_INFO}Compressing context{ANSI_RESET}")),
        "core.memo" => {
            let op = data.get("op").and_then(Value::as_str).unwrap_or("updated");
            Some(format!(
                "{ANSI_DIM}Work memo {}{ANSI_RESET}",
                humanize(op).to_lowercase()
            ))
        }
        _ => None,
    }
}

pub(super) fn activity_summary(payload: &Value) -> Option<String> {
    let kind = payload.get("kind").and_then(Value::as_str)?;
    if matches!(kind, "model_request" | "model_response" | "lifecycle") {
        return None;
    }
    let detail = payload
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let detail = truncate_line(detail, 160);
    Some(if detail.is_empty() {
        humanize(kind)
    } else {
        format!("{} · {detail}", humanize(kind))
    })
}

pub(super) fn worker_event_summary(payload: &Value) -> Option<String> {
    let kind = payload.get("kind").and_then(Value::as_str)?;
    match kind {
        "model_request"
        | "model_response"
        | "subworker_turn_projection"
        | "topic_scope_mismatch" => None,
        "model_retry" => {
            let attempt = payload.get("attempt").and_then(Value::as_u64);
            let max = payload.get("max_attempts").and_then(Value::as_u64);
            let delay_ms = payload.get("delay_ms").and_then(Value::as_u64);
            let error = payload.get("error").and_then(Value::as_str).unwrap_or("");
            let progress = match (attempt, max) {
                (Some(attempt), Some(max)) => format!(" {attempt}/{max}"),
                _ => String::new(),
            };
            let delay = delay_ms
                .map(|ms| format!(" · next attempt in {:.1}s", ms as f64 / 1000.0))
                .unwrap_or_default();
            let error = if error.is_empty() {
                String::new()
            } else {
                format!(" · {}", truncate_line(error, 140))
            };
            Some(format!(
                "{ANSI_WARN}Retrying model{progress}{ANSI_RESET}{delay}{error}"
            ))
        }
        "model_error" => {
            let error = payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Unknown model error");
            Some(format!(
                "{ANSI_FAIL}Model error{ANSI_RESET} · {}",
                truncate_line(error, 160)
            ))
        }
        "reasoning_upgrade" => {
            let from = payload
                .get("from")
                .and_then(Value::as_str)
                .unwrap_or("default");
            let to = payload
                .get("to")
                .and_then(Value::as_str)
                .unwrap_or("enhanced");
            Some(format!(
                "{ANSI_INFO}Reasoning upgraded{ANSI_RESET} · {from} → {to}"
            ))
        }
        "subworker_turn_finished" => {
            let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
            let detail = truncate_line(text, 160);
            Some(if detail.is_empty() {
                format!("{ANSI_BRIGHT_TIMEM}Worker completed{ANSI_RESET}")
            } else {
                format!("{ANSI_BRIGHT_TIMEM}Worker completed{ANSI_RESET} · {detail}")
            })
        }
        "worker_stopped" => Some(format!("{ANSI_WARN}Worker stopped{ANSI_RESET}")),
        "unconsumed_supplements" => Some(format!(
            "{ANSI_WARN}A late supplement will continue in the next turn{ANSI_RESET}"
        )),
        "unconsumed_supplements_resubmit_failed" => Some(format!(
            "{ANSI_FAIL}Could not continue a late supplement automatically{ANSI_RESET}"
        )),
        "work_instruction_request_timeout" => Some(format!(
            "{ANSI_WARN}Work-instruction request timed out{ANSI_RESET}"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::HostEndpoint;
    use super::*;
    use serde_json::json;

    fn session(
        ordinal: u32,
        name: &str,
        working: bool,
        workers: usize,
        path: &str,
    ) -> AttachSession {
        AttachSession {
            session_id: format!("session-{ordinal}"),
            display_name: name.to_string(),
            ordinal,
            state: if working { "working" } else { "ready" }.to_string(),
            working,
            active_turn_id: None,
            current_dir: path.to_string(),
            restart_cwd_decision: None,
            worker_count: workers,
        }
    }

    fn strip_ansi(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(character) = chars.next() {
            if character == '\x1b' {
                for character in chars.by_ref() {
                    if character.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(character);
            }
        }
        out
    }

    #[test]
    fn instance_selector_shows_identity_without_exposing_tokens() {
        let candidates = vec![
            AttachHostCandidate {
                memory_dir: std::path::PathBuf::from("/tmp/中文-memory"),
                pid: 7,
                started_at_ms: 1_700_000_000_000,
                host: HostEndpoint {
                    port: 8080,
                    token: Some("never-render-this-token".to_string()),
                },
            },
            AttachHostCandidate {
                memory_dir: std::path::PathBuf::from("/tmp/longer-memory"),
                pid: 12345,
                started_at_ms: 1_700_000_000_001,
                host: HostEndpoint {
                    port: 18080,
                    token: Some("another-secret".to_string()),
                },
            },
        ];
        let plain = strip_ansi(&instance_selector(&candidates, 1));
        let lines = plain.lines().collect::<Vec<_>>();
        assert_eq!(
            lines[0],
            "Select a Timem instance (↑/↓ move · Enter continue · Esc exit)"
        );
        assert!(lines[1].contains("/tmp/中文-memory"));
        assert!(lines[1].contains("PID     7"));
        assert!(lines[1].contains("port  8080"));
        assert!(lines[2].contains("❯ /tmp/longer-memory"));
        assert!(lines[2].contains("PID 12345"));
        assert!(lines[2].contains("port 18080"));
        assert!(lines[2].contains("started 1700000000001"));
        assert!(!plain.contains("never-render-this-token"));
        assert!(!plain.contains("another-secret"));
    }

    #[test]
    fn selector_aligns_unicode_names_and_columns() {
        let sessions = vec![
            session(1, "Alpha", false, 1, "/repo/a"),
            session(12, "中文会话", true, 9, "/repo/b"),
        ];
        let plain = strip_ansi(&session_selector(&sessions, 1));
        let lines = plain.lines().collect::<Vec<_>>();
        assert_eq!(
            lines[0],
            "Select a session (↑/↓ move · Enter attach · Esc exit)"
        );
        assert!(lines[1].contains(" 1  Alpha"));
        assert!(lines[1].contains("idle"));
        assert!(lines[2].contains("❯ 12  中文会话"));
        assert!(lines[2].contains("working"));
        let display_column = |line: &str, needle: &str| {
            let byte_index = line.find(needle).expect("column marker");
            UnicodeWidthStr::width(&line[..byte_index])
        };
        assert_eq!(
            display_column(lines[1], "workers"),
            display_column(lines[2], "workers")
        );
        assert_eq!(
            display_column(lines[1], "/repo"),
            display_column(lines[2], "/repo")
        );
    }

    #[test]
    fn connected_intro_explains_state_and_safe_controls() {
        let intro = strip_ansi(&connected_intro(
            &session(3, "Build", true, 2, "/repo/project"),
            true,
        ));
        assert!(intro.contains("Build  session-3"));
        assert!(intro.contains("State: Working · Workers: 2"));
        assert!(intro.contains("input while working adds a supplement"));
        assert!(intro.contains("Ctrl+C detaches only"));
    }

    #[test]
    fn action_projection_humanizes_command_status_and_truncates_detail() {
        let running = action_detail(&json!({
            "action": "run_bash",
            "event": "start",
            "status": "running",
            "input": {"cmd": "cargo build --release"},
        }))
        .expect("action detail");
        let plain = strip_ansi(&running);
        assert!(plain.contains("Run command"));
        assert!(plain.contains("`cargo build --release`"));

        let failed = action_detail(&json!({
            "action": "run_bash",
            "event": "finish",
            "status": "error",
            "input": {"cmd": "x".repeat(200)},
        }))
        .expect("failed action detail");
        assert!(failed.contains(ANSI_FAIL));
        assert!(failed.contains("..."));
        assert!(failed.chars().count() < 200);
    }

    #[test]
    fn completion_control_actions_are_not_presented_as_tools() {
        for action in ["task_finished", "turn_finished"] {
            assert!(action_detail(&json!({
                "action": action,
                "event": "finish",
                "status": "completed",
            }))
            .is_none());
        }
    }

    #[test]
    fn topic_projection_hides_noise_and_keeps_useful_progress() {
        assert!(topic_summary(&json!({
            "topic": {"name": "core.model.preview"},
            "payload": {"delta": "raw"},
        }))
        .is_none());
        assert!(topic_summary(&json!({
            "topic": {"name": "core.lifecycle"},
            "payload": {"event": "working"},
        }))
        .is_none());
        let thought = topic_summary(&json!({
            "topic": {"name": "core.model.response"},
            "payload": {"free_talk": "Checking the build", "continue_work": true},
        }))
        .expect("continuing thought");
        assert!(strip_ansi(&thought).contains("Thinking · Checking the build"));
        assert!(topic_summary(&json!({
            "topic": {"name": "core.model.response"},
            "payload": {"final_answer": "Rendered elsewhere"},
        }))
        .is_none());
    }

    #[test]
    fn worker_projection_hides_request_response_and_formats_failures() {
        assert!(worker_event_summary(&json!({"kind": "model_request"})).is_none());
        assert!(worker_event_summary(&json!({"kind": "model_response"})).is_none());
        let retry = worker_event_summary(&json!({
            "kind": "model_retry",
            "attempt": 2,
            "max_attempts": 4,
            "delay_ms": 1500,
            "error": "service unavailable",
        }))
        .expect("retry summary");
        let plain = strip_ansi(&retry);
        assert!(plain.contains("Retrying model 2/4"));
        assert!(plain.contains("1.5s"));
        assert!(plain.contains("service unavailable"));
    }

    #[test]
    fn decision_and_restart_cards_put_numbered_choices_first() {
        let approval = json!({
            "topic": {
                "name": "core.user.approval.request",
                "attributes": {"expects_reply": true},
            },
            "payload": {"request": {
                "action": "run",
                "command": "cargo test",
                "risk": "medium",
            }},
        });
        let plain = strip_ansi(&decision_request_prompt(&approval).expect("approval prompt"));
        assert!(plain.contains("Action required"));
        assert!(plain.contains("1  Accept\n  2  Decline\n  3  Always allow"));
        assert!(plain.contains("Aliases: !y / !n / !a"));

        let restart = strip_ansi(
            &restart_cwd_prompt(&json!({
                "runtime_cwd": "/host",
                "session_cwd": "/session",
                "session_cwd_available": false,
            }))
            .expect("restart prompt"),
        );
        assert!(restart.contains("1  Keep session directory  (unavailable)"));
        assert!(restart.contains("2  Use host directory"));
        assert!(restart.contains("Aliases: !r"));
        assert!(!restart.contains("!k / !r"));
    }

    #[test]
    fn guidance_cards_always_include_a_next_step() {
        for card in [
            no_sessions_card(),
            disconnected_card(None),
            rejected_command_card("unknown"),
            host_error_card("boom"),
            invalid_command_card("!wat"),
            invalid_restart_choice_card("3"),
        ] {
            assert!(strip_ansi(&card).contains("Next:"));
        }
    }
}

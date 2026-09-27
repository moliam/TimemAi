//! `timem attach`: attach a terminal to a session owned by a running Web Host.
//!
//! The shell is a plain client here: it discovers the Host through the
//! workspace `web_instance.json` lease, lists sessions over the read-only
//! attach endpoint, and then renders authoritative Host events over the
//! existing WebSocket command transport. It never owns domain state.

use crate::{
    dim_line, local_time_label, render_final_answer_markdown, ANSI_BOLD, ANSI_BRIGHT_TIMEM,
    ANSI_DIM, ANSI_RESET, TIMEM_LOGO,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const ANSI_FAIL: &str = "\x1b[31m";

/// Whether a progress dot line is open. Event lines close it first so live
/// output and progress never interleave on one terminal row.
static DOTS_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn close_dots() {
    if DOTS_OPEN.swap(false, std::sync::atomic::Ordering::Relaxed) {
        println!();
    }
}
const ATTACH_PATH: &str = "/api/attach/sessions";

#[derive(Debug, Deserialize)]
struct WebInstanceFile {
    pid: u32,
    port: Option<u16>,
    token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttachSession {
    pub session_id: String,
    pub display_name: String,
    pub ordinal: u32,
    pub state: String,
    pub working: bool,
    pub active_turn_id: Option<String>,
    pub current_dir: String,
    pub restart_cwd_decision: Option<Value>,
    pub worker_count: usize,
}

enum AttachError {
    NoHost,
    HostUnreachable(String),
    Http(String),
    Protocol(String),
}

impl AttachError {
    fn message(&self) -> String {
        match self {
            Self::NoHost => "no running timem web host found".to_string(),
            Self::HostUnreachable(detail) => format!("host unreachable: {detail}"),
            Self::Http(detail) => format!("attach endpoint failed: {detail}"),
            Self::Protocol(detail) => format!("attach protocol error: {detail}"),
        }
    }
}

/// Entry point for `timem attach`. `space` selects the workspace whose Host
/// lease should be discovered.
pub fn run_attach(space: Option<&str>) {
    let memory_dir = match crate::resolve_memory_dir(space) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[attach_error] {error}");
            std::process::exit(2);
        }
    };
    let instance_path = memory_dir.join("web_instance.json");
    let host = match discover_host(&instance_path) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("[attach_error] {}", error.message());
            std::process::exit(2);
        }
    };
    let sessions = match fetch_attach_sessions(&host) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("[attach_error] {}", error.message());
            std::process::exit(2);
        }
    };
    if sessions.is_empty() {
        eprintln!("[attach_error] the host has no sessions");
        std::process::exit(2);
    }
    let selected = match select_session(&sessions) {
        Some(session) => session,
        None => return,
    };
    if let Err(error) = attach_session(&host, &selected) {
        eprintln!("[attach_error] {}", error.message());
        std::process::exit(2);
    }
}

struct HostEndpoint {
    port: u16,
    token: Option<String>,
}

fn discover_host(instance_path: &std::path::Path) -> Result<HostEndpoint, AttachError> {
    let raw = std::fs::read(instance_path).map_err(|_| AttachError::NoHost)?;
    let info: WebInstanceFile = serde_json::from_slice(&raw).map_err(|_| AttachError::NoHost)?;
    let port = info.port.ok_or(AttachError::NoHost)?;
    // A stale lease for a dead process must not pass as a live Host.
    let alive = std::path::Path::new("/proc")
        .join(info.pid.to_string())
        .exists();
    if !alive && !host_responds(port, info.token.as_deref()) {
        return Err(AttachError::NoHost);
    }
    Ok(HostEndpoint {
        port,
        token: info.token,
    })
}

fn host_responds(port: u16, token: Option<&str>) -> bool {
    http_get_json(port, "/api/health", token, 512).is_ok()
}

fn http_get_json(
    port: u16,
    path: &str,
    token: Option<&str>,
    max_bytes: usize,
) -> Result<Value, AttachError> {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| AttachError::HostUnreachable(e.to_string()))?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT)).ok();
    stream.set_write_timeout(Some(HTTP_TIMEOUT)).ok();
    let mut stream = stream;
    let token_query = token.map(|t| format!("?token={t}")).unwrap_or_default();
    let request = format!(
        "GET {path}{token_query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| AttachError::Http(e.to_string()))?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|e| AttachError::Http(e.to_string()))?;
    if !status_line.contains("200") {
        return Err(AttachError::Http(format!(
            "unexpected status: {}",
            status_line.trim()
        )));
    }
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| AttachError::Http(e.to_string()))?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().ok();
        }
        if trimmed
            .to_ascii_lowercase()
            .starts_with("transfer-encoding:")
        {
            chunked = true;
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            let mut size_line = String::new();
            reader
                .read_line(&mut size_line)
                .map_err(|e| AttachError::Http(e.to_string()))?;
            let size = usize::from_str_radix(size_line.trim(), 16)
                .map_err(|_| AttachError::Protocol("bad chunk size".to_string()))?;
            if size == 0 {
                break;
            }
            if body.len() + size > max_bytes {
                return Err(AttachError::Protocol("response too large".to_string()));
            }
            let mut chunk = vec![0u8; size];
            reader
                .read_exact(&mut chunk)
                .map_err(|e| AttachError::Http(e.to_string()))?;
            body.extend_from_slice(&chunk);
            let mut crlf = [0u8; 2];
            reader
                .read_exact(&mut crlf)
                .map_err(|e| AttachError::Http(e.to_string()))?;
        }
    } else {
        let limit = content_length.unwrap_or(max_bytes).min(max_bytes);
        reader
            .take(limit as u64)
            .read_to_end(&mut body)
            .map_err(|e| AttachError::Http(e.to_string()))?;
    }
    serde_json::from_slice(&body).map_err(|e| AttachError::Protocol(e.to_string()))
}

fn fetch_attach_sessions(host: &HostEndpoint) -> Result<Vec<AttachSession>, AttachError> {
    let payload = http_get_json(host.port, ATTACH_PATH, host.token.as_deref(), 256 * 1024)?;
    let sessions = payload
        .get("sessions")
        .cloned()
        .ok_or_else(|| AttachError::Protocol("missing sessions field".to_string()))?;
    serde_json::from_value(sessions).map_err(|e| AttachError::Protocol(e.to_string()))
}

/// Renders the interactive up/down selector. Returns the chosen session or
/// None when the user aborted.
fn select_session(sessions: &[AttachSession]) -> Option<AttachSession> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    println!("{ANSI_BRIGHT_TIMEM}{TIMEM_LOGO}{ANSI_RESET} {ANSI_DIM}attach{ANSI_RESET}");
    let mut selected = 0usize;
    let _ = enable_raw_mode();
    let line_count = sessions.len() + 1;
    let mut rendered_once = false;
    let render = |selected: usize, rendered_once: bool| {
        use crossterm::cursor::MoveUp;
        use crossterm::queue;
        use crossterm::terminal::{Clear, ClearType};
        let mut stdout = std::io::stdout();
        if rendered_once {
            // Move above the previously drawn block and clear downward, so the
            // list updates in place instead of scrolling the whole screen.
            let _ = queue!(
                stdout,
                MoveUp(line_count as u16),
                Clear(ClearType::FromCursorDown)
            );
        }
        let mut out = String::new();
        out.push_str(&format!(
            "{ANSI_DIM}选择一个 session（↑/↓ 移动，Enter attach，Esc 退出）{ANSI_RESET}\r\n"
        ));
        for (index, session) in sessions.iter().enumerate() {
            let marker = if index == selected {
                format!("{ANSI_BRIGHT_TIMEM}❯{ANSI_RESET}")
            } else {
                " ".to_string()
            };
            let name = if session.working {
                format!("{ANSI_BOLD}{}{ANSI_RESET}", session.display_name)
            } else {
                session.display_name.clone()
            };
            let work = if session.working { " working" } else { "" };
            let line = format!(
                "{marker} {ANSI_DIM}{}.{}{ANSI_RESET} {name}{ANSI_DIM}{work} · workers: {}, {}{ANSI_RESET}\r\n",
                index + 1,
                " ",
                session.worker_count,
                session.current_dir,
            );
            out.push_str(&line);
        }
        print!("{out}");
        let _ = stdout.flush();
    };
    render(selected, rendered_once);
    rendered_once = true;
    let result = loop {
        match crossterm::event::read() {
            Ok(Event::Key(KeyEvent {
                code, modifiers, ..
            })) => match (code, modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => break None,
                (KeyCode::Esc, _) => break None,
                (KeyCode::Up, _) if selected > 0 => {
                    selected -= 1;
                    render(selected, rendered_once);
                }
                (KeyCode::Down, _) if selected + 1 < sessions.len() => {
                    selected += 1;
                    render(selected, rendered_once);
                }
                (KeyCode::Enter, _) => break Some(sessions[selected].clone()),
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    let _ = disable_raw_mode();
    println!();
    result
}

/// A Host decision request (approval / continue / expand ...) awaiting a
/// reply. Entering `!y` / `!n` / `!a` in the attach prompt answers it.
#[derive(Debug, Clone)]
struct PendingDecision {
    request_id: Option<String>,
    worker_id: Option<String>,
    topic_name: String,
}

fn decision_request_prompt(payload: &Value) -> Option<String> {
    let topic = payload.get("topic")?;
    if topic
        .get("attributes")
        .and_then(|a| a.get("expects_reply"))
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
    let line = match topic_name {
        "core.user.approval.request" => {
            let action = request
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("run");
            let command = request.get("command").and_then(Value::as_str).unwrap_or("");
            let risk = request.get("risk").and_then(Value::as_str).unwrap_or("?");
            format!("approve {action}: {command} (risk: {risk})")
        }
        "core.user.round_limit.request" => {
            let recharge = request
                .get("recharge_rounds")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            format!("continue for {recharge} more rounds past the round limit")
        }
        "core.user.output_expand.request" => {
            let increment = request
                .get("increment_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            format!("expand output by +{increment} tokens")
        }
        "core.user.stale_context.request" => {
            let idle = request.get("idle_ms").and_then(Value::as_u64).unwrap_or(0);
            format!("continue after {}ms idle (keeps dynamic context)", idle)
        }
        "core.work_instruction_load" => {
            let directory = request
                .get("directory")
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("load work instructions from {directory}")
        }
        "core.user.long_running_command.request" => {
            let action = request
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("command");
            let command = request.get("command").and_then(Value::as_str).unwrap_or("");
            let elapsed = request
                .get("elapsed_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            format!("{action} still running: {command} ({elapsed}ms)")
        }
        _ => format!("host decision request on {topic_name}"),
    };
    Some(format!("\n[needs reply] {line}\n  输入 1 或 2 后回车:  1 = 接受   2 = 拒绝   3 = 总是允许(仅审批)  (等效 !y / !n / !a)"))
}

fn register_decision_request(
    payload: &Value,
    pending: &mut Vec<PendingDecision>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Some(prompt) = decision_request_prompt(payload) else {
        return;
    };
    close_dots();
    let payload_payload = payload.get("payload").cloned().unwrap_or(Value::Null);
    let request_id = payload_payload
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let dedupe_key = request_id.clone().unwrap_or_else(|| prompt.clone());
    if !seen.insert(dedupe_key) {
        return;
    }
    println!("{prompt}");
    pending.push(PendingDecision {
        request_id,
        worker_id: payload
            .get("worker_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        topic_name: payload
            .get("topic")
            .and_then(|topic| topic.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    });
}

struct AttachTurnView {
    printed_event_ids: std::collections::HashSet<String>,
    printed_user_entries: usize,
    final_answer_printed: bool,
    seen_decision_ids: std::collections::HashSet<String>,
}

impl AttachTurnView {
    /// Renders the authoritative final answer exactly once per turn.
    fn render_final_answer_once(&mut self, text: &str) {
        if text.is_empty() || self.final_answer_printed {
            return;
        }
        self.final_answer_printed = true;
        println!("\n{}", render_final_answer_markdown(text));
    }

    fn new() -> Self {
        Self {
            printed_event_ids: std::collections::HashSet::new(),
            printed_user_entries: 0,
            final_answer_printed: false,
            seen_decision_ids: std::collections::HashSet::new(),
        }
    }

    fn render_turn(&mut self, turn: &Value, pending: &mut Vec<PendingDecision>) {
        close_dots();
        if let Some(entries) = turn.get("user_entries").and_then(Value::as_array) {
            while self.printed_user_entries < entries.len() {
                let entry = &entries[self.printed_user_entries];
                self.printed_user_entries += 1;
                let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("user");
                let text = entry.get("text").and_then(Value::as_str).unwrap_or("");
                if !text.is_empty() {
                    println!("\n[{kind}] {text}");
                }
            }
        }
        if let Some(events) = turn.get("events").and_then(Value::as_array) {
            for event in events {
                let event_id = event
                    .get("event_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if event_id.is_empty() || !self.printed_event_ids.insert(event_id.clone()) {
                    continue;
                }
                register_decision_request(event, pending, &mut self.seen_decision_ids);
                print_turn_event(event);
            }
        }
        let final_answer = turn
            .get("final_answer")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty());
        if let Some(answer) = final_answer {
            self.render_final_answer_once(answer);
        }
    }
}

fn print_turn_event(event: &Value) {
    let source = event.get("source").and_then(Value::as_str).unwrap_or("");
    let payload = event.get("payload").cloned().unwrap_or(Value::Null);
    let line = match source {
        "ui_activity" => activity_summary(&payload),
        "core_topic" => topic_summary(&payload),
        _ => worker_event_summary(&payload),
    };
    if let Some(line) = line {
        if !line.is_empty() {
            println!("  {line}");
        }
    }
}

fn activity_summary(payload: &Value) -> Option<String> {
    let kind = payload.get("kind").and_then(Value::as_str)?;
    let detail = payload
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(format!("- {kind} {detail}").trim_end().to_string())
}

fn topic_summary(payload: &Value) -> Option<String> {
    let topic = payload
        .get("topic")
        .and_then(|topic| topic.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("core.topic");
    if topic == "core.action" {
        return action_detail(payload.get("payload")?);
    }
    if topic == "core.model.preview" {
        // Streaming text deltas: keep them silent. Progress is already
        // conveyed by the projection dots; per-delta lines would flood
        // the terminal.
        return None;
    }
    let event_name = payload.get("event").and_then(Value::as_str).unwrap_or("");
    Some(format!("- [{topic}] {event_name}").trim_end().to_string())
}

/// Renders live tool execution detail: command lines, file reads, tool status.
fn action_detail(payload: &Value) -> Option<String> {
    let action = payload.get("action").and_then(Value::as_str)?;
    let event = payload.get("event").and_then(Value::as_str).unwrap_or("");
    let status = payload.get("status").and_then(Value::as_str).unwrap_or("");
    let input = payload.get("input");
    let describe_input = |input: Option<&Value>| -> String {
        let Some(input) = input else {
            return String::new();
        };
        match action {
            "run_bash" => input
                .get("cmd")
                .and_then(Value::as_str)
                .map(|cmd| format!("`{}`", truncate_line(cmd, 120)))
                .unwrap_or_default(),
            "readfile" => input
                .get("path")
                .and_then(Value::as_str)
                .map(|path| truncate_line(path, 120))
                .unwrap_or_default(),
            _ => String::new(),
        }
    };
    let detail = describe_input(input);
    let prefix = match event {
        "start" => format!("{ANSI_BRIGHT_TIMEM}>{ANSI_RESET}"),
        _ => format!("{ANSI_DIM}<{ANSI_RESET}"),
    };
    let status_style = if status.contains("error") || status.contains("fail") {
        ANSI_FAIL
    } else {
        ANSI_DIM
    };
    let status_label = if status.is_empty() {
        String::new()
    } else {
        format!(" {status_style}{status}{ANSI_RESET}")
    };
    Some(
        format!("{prefix} {ANSI_BOLD}{action}{ANSI_RESET} {detail}{status_label}")
            .trim_end()
            .to_string(),
    )
}

/// Shell-style attach prompt, shown while the session is idle.
fn print_attach_prompt() {
    close_dots();
    print!(
        "\x1b[94;1m[{}] {TIMEM_LOGO} attach ❯❯{ANSI_RESET} ",
        local_time_label()
    );
    let _ = std::io::stdout().flush();
}

/// Renders a pending restart-cwd decision (Web shows the same gate as a modal).
fn restart_cwd_prompt(decision: &Value) -> Option<String> {
    let runtime_cwd = decision.get("runtime_cwd")?.as_str()?;
    let session_cwd = decision
        .get("session_cwd")
        .and_then(Value::as_str)
        .unwrap_or("");
    let available = decision
        .get("session_cwd_available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let keep_num_hint = if available {
        ""
    } else {
        "  (session 目录不可用，只能选 2)"
    };
    Some(format!(
        "\n[needs reply] restart cwd mismatch\n  host dir:   {runtime_cwd}\n  session dir: {session_cwd}\n  输入 1 或 2 后回车:  1 = 保持 session 目录{keep_num_hint}   2 = 切到 host 目录  (等效 !k / !r)"
    ))
}

/// Echoes an accepted user line in the same style as the interactive shell.
fn format_user_echo(text: &str) -> String {
    format!(
        "\x1b[94;1m[{}] You ❯❯{ANSI_RESET} {text}",
        local_time_label()
    )
}

fn truncate_line(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{truncated}...")
}

fn worker_event_summary(payload: &Value) -> Option<String> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("worker_event");
    Some(format!("- {kind}").to_string())
}

/// Connects to the Host WebSocket and runs the attach loop: stream-render
/// authoritative events for the chosen session and submit every entered line
/// as a forced supplement (no message queueing).
fn attach_session(host: &HostEndpoint, session: &AttachSession) -> Result<(), AttachError> {
    println!(
        "{ANSI_BRIGHT_TIMEM}{TIMEM_LOGO}{ANSI_RESET} {ANSI_DIM}attach ·{ANSI_RESET} {} {ANSI_DIM}({}){ANSI_RESET}",
        session.display_name, session.session_id
    );
    let initial_restart_decision = session.restart_cwd_decision.clone();
    println!(
        "{ANSI_DIM}空闲时输入即提交新 turn；working 时作为 supplement 注入。命令：提示出现时按数字 1/2(/3) 回车即可选择；!c 取消正在跑的 turn，!s 停止 session；Ctrl+C 仅断开 attach。{ANSI_RESET}"
    );
    print_attach_prompt();
    let token_query = host
        .token
        .as_deref()
        .map(|t| format!("?token={t}"))
        .unwrap_or_default();
    let url = format!("ws://127.0.0.1:{}/ws{}", host.port, token_query);
    let request = url
        .into_client_request()
        .map_err(|e| AttachError::Protocol(e.to_string()))?;
    // The Host hello snapshot carries full session history and can exceed the
    // tungstenite default 16MiB single-frame limit for long-running sessions
    // (observed ~19MB). Raise both frame and message limits; the Host is a
    // trusted local endpoint bounded by MAX_BROWSER_COMMAND/snapshot sizes.
    let ws_config = tungstenite::protocol::WebSocketConfig {
        max_frame_size: Some(256 << 20),
        max_message_size: Some(256 << 20),
        ..Default::default()
    };
    let (mut ws, _response) = tungstenite::client::connect_with_config(request, Some(ws_config), 3)
        .map_err(|e| AttachError::HostUnreachable(e.to_string()))?;

    // Terminal input on a worker thread; the main loop multiplexes stdin and
    // the socket. The underlying stream uses a short read timeout so an idle
    // session (no events flowing) still delivers user input promptly instead
    // of blocking forever inside ws.read().
    let (input_tx, input_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            if stdin.read_line(&mut line).is_err() {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
            if input_tx.send(trimmed).is_err() {
                break;
            }
        }
    });
    // tungstenite surfaces a read timeout as a transient WouldBlock error;
    // the next read continues normally, giving a poll-friendly loop.
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = ws.get_ref() {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    }

    let mut turn_views: std::collections::HashMap<String, AttachTurnView> =
        std::collections::HashMap::new();
    let mut pending_decisions: Vec<PendingDecision> = Vec::new();
    let mut restart_cwd_decision: Option<Value> = initial_restart_decision;
    if let Some(decision) = restart_cwd_decision.as_ref() {
        // The gate blocks all turns; surface it immediately so the user is
        // never stuck without knowing the valid replies.
        if let Some(prompt) = restart_cwd_prompt(decision) {
            println!("{prompt}");
        }
    }
    let session_id = session.session_id.clone();
    let mut command_sequence: u64 = 0;

    loop {
        // Drain pending user input first so lines are never delayed by
        // streaming traffic.
        while let Ok(text) = input_rx.try_recv() {
            if text.is_empty() {
                continue;
            }
            command_sequence += 1;
            let command_id = format!("attach_{}_{}", std::process::id(), command_sequence);
            // Numbered choices: render options when a prompt appears and let
            // the user pick by number, so no command memorization is needed.
            let numeric_choice = match text.as_str() {
                "1" | "2" | "3" => Some(text.as_str()),
                _ => None,
            };
            if let (Some(choice), Some(_)) = (numeric_choice, &restart_cwd_decision) {
                // Gate menu: 1 = keep session dir, 2 = use host dir.
                let decision = if choice == "1" {
                    "keep_session"
                } else {
                    "use_runtime"
                };
                command_sequence += 0; // counted above
                let message = json!({
                    "command_id": format!("attach_{}_{}", std::process::id(), command_sequence),
                    "type": "session_restart_cwd_resolve",
                    "session_id": session_id,
                    "decision": decision,
                });
                restart_cwd_decision = None;
                let send_result = match ws.write(Message::Text(message.to_string())) {
                    Ok(()) => ws.flush(),
                    Err(error) => Err(error),
                };
                if let Err(error) = send_result {
                    return Err(AttachError::HostUnreachable(error.to_string()));
                }
                println!("{}", format_user_echo(&text));
                print_attach_prompt();
                continue;
            }
            let numeric_decision = match numeric_choice {
                Some("1") => Some("accept"),
                Some("2") => Some("decline"),
                Some("3") => Some("always_allow"),
                _ => None,
            };
            // Decision replies answer the newest pending host request instead
            // of submitting a new turn.
            let decision = match text.as_str() {
                "!y" => Some("accept"),
                "!n" => Some("decline"),
                "!a" => Some("always_allow"),
                _ => numeric_decision,
            };
            // Restart-cwd gate: mirrors the Web RestartCwdGate modal.
            let restart_choice = match text.as_str() {
                "!r" => Some("use_runtime"),
                "!k" => Some("keep_session"),
                _ => None,
            };
            // Send restart-cwd replies even without a locally known pending
            // gate: the Host is authoritative and the local snapshot can be
            // stale (e.g. rejection arrived before any gate payload). An
            // unprompted reply is rejected harmlessly instead of trapping
            // the user with no escape hatch.
            let message = if let Some(choice) = &restart_choice {
                restart_cwd_decision = None;
                json!({
                    "command_id": command_id,
                    "type": "session_restart_cwd_resolve",
                    "session_id": session_id,
                    "decision": choice,
                })
            } else if let (Some(decision), Some(pending)) = (decision, pending_decisions.last()) {
                let mut reply = json!({
                    "command_id": command_id,
                    "type": "topic_reply",
                    "session_id": session_id,
                    "topic_name": pending.topic_name,
                    "decision": decision,
                });
                if let Some(request_id) = pending.request_id.as_deref() {
                    reply["request_id"] = json!(request_id);
                }
                if let Some(worker_id) = pending.worker_id.as_deref() {
                    reply["worker_id"] = json!(worker_id);
                }
                pending_decisions.pop();
                reply
            } else if text == "!c" {
                json!({
                    "command_id": command_id,
                    "type": "turn_cancel",
                    "session_id": session_id,
                })
            } else if text == "!s" {
                // Full stop: shuts down all workers of the session and blocks
                // queue continuation (mirrors the Web "stop session" action).
                json!({
                    "command_id": command_id,
                    "type": "session_stop",
                    "session_id": session_id,
                })
            } else {
                if text.starts_with('!') {
                    println!(
                            "[no pending request; ignored {text}. valid: !y !n !a (decisions), !r !k (restart cwd), !c (cancel running turn), !s (stop session)]"
                        );
                    continue;
                }
                json!({
                    "command_id": command_id,
                    "type": "turn_supplement",
                    "session_id": session_id,
                    "text": text,
                })
            };
            // The Host falls back to submitting a fresh turn when the session
            // is idle, so every entered line produces visible work.
            // `write` only buffers the frame locally; flush it to the socket so
            // the command actually reaches the Host before reporting it sent.
            let send_result = match ws.write(Message::Text(message.to_string())) {
                Ok(()) => ws.flush(),
                Err(error) => Err(error),
            };
            if let Err(error) = send_result {
                return Err(AttachError::HostUnreachable(error.to_string()));
            }
            println!("{}", format_user_echo(&text));
            print_attach_prompt();
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if !handle_wire_event(
                    host,
                    &text,
                    &session_id,
                    &mut turn_views,
                    &mut pending_decisions,
                    &mut restart_cwd_decision,
                ) {
                    return Ok(());
                }
            }
            Ok(Message::Close(_)) => {
                println!("{}", dim_line("[attach disconnected]"));
                return Ok(());
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(ref io_error))
                if io_error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                // Read timeout: loop back to poll input again.
                thread::sleep(Duration::from_millis(20));
            }
            Err(other) => {
                // A quiet socket after detach input ends the loop naturally.
                if input_rx.try_recv().is_err() && !ws.can_read() {
                    println!("{}", dim_line("[attach disconnected]"));
                    return Ok(());
                }
                // Capacity/protocol errors never recover on retry; surface
                // them instead of spinning silently behind the prompt.
                if matches!(
                    other,
                    tungstenite::Error::Capacity(_)
                        | tungstenite::Error::Protocol(_)
                        | tungstenite::Error::Utf8
                ) {
                    println!("[attach error] {other}");
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn handle_wire_event(
    host: &HostEndpoint,
    text: &str,
    session_id: &str,
    turn_views: &mut std::collections::HashMap<String, AttachTurnView>,
    pending_decisions: &mut Vec<PendingDecision>,
    restart_cwd_decision: &mut Option<Value>,
) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return true;
    };
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "hello" | "semantic_event" => {
            let inner = if event_type == "semantic_event" {
                value.get("event").cloned().unwrap_or(Value::Null)
            } else {
                value.clone()
            };
            handle_wire_event_inner(
                &inner,
                session_id,
                turn_views,
                pending_decisions,
                restart_cwd_decision,
            )
        }
        "turn_projection" => {
            // Streaming projections render as progress hints; the authoritative
            // turn snapshot arrives via turn_updated.
            if let Some(status) = value
                .get("projection")
                .and_then(|p| p.get("projection"))
                .and_then(|p| p.get("status"))
                .and_then(Value::as_str)
            {
                if status == "active" {
                    DOTS_OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
                    print!(".");
                    let _ = std::io::stdout().flush();
                }
            }
            true
        }
        "turn_finished" => {
            if value.get("session_id").and_then(Value::as_str) == Some(session_id) {
                handle_turn_finished(&value, turn_views);
            }
            true
        }
        "core_topic" | "worker_activity" => {
            if value.get("session_id").and_then(Value::as_str) != Some(session_id) {
                return true;
            }
            close_dots();
            let payload = value.get("event").cloned().unwrap_or(Value::Null);
            if event_type == "core_topic" {
                register_live_decision_request(&payload, pending_decisions);
                if let Some(line) = live_topic_summary(&payload) {
                    close_dots();
                    println!("  {line}");
                }
            }
            true
        }
        "command_ack" => {
            let status = value.get("status").and_then(Value::as_str).unwrap_or("");
            if status == "rejected" {
                close_dots();
                let error = value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                println!("[rejected] {error}");
                if error == "session_restart_cwd_decision_required"
                    && restart_cwd_decision.is_none()
                {
                    // The hello snapshot may predate the gate (e.g. the
                    // mismatch appeared after attach). Recover the
                    // authoritative gate from the read-only attach API.
                    let host_port = host.port;
                    let token = host.token.clone();
                    let fetched =
                        http_get_json(host_port, ATTACH_PATH, token.as_deref(), 256 * 1024)
                            .ok()
                            .and_then(|payload| {
                                payload
                                    .get("sessions")?
                                    .as_array()?
                                    .iter()
                                    .find_map(|entry| {
                                        (entry.get("session_id").and_then(Value::as_str)
                                            == Some(session_id))
                                        .then(|| entry.get("restart_cwd_decision").cloned())
                                        .flatten()
                                    })
                            });
                    if let Some(decision) = fetched {
                        if let Some(prompt) = restart_cwd_prompt(&decision) {
                            println!("{prompt}");
                        }
                        *restart_cwd_decision = Some(decision);
                    } else {
                        println!(
                            "{}",
                            dim_line("[hint] reply !r (switch session to host dir) or !k (keep session dir)")
                        );
                    }
                }
            }
            true
        }
        "host_error" => {
            close_dots();
            let error = value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !error.is_empty() {
                println!("[host error] {error}");
            }
            if error == "session_restart_cwd_decision_required" && restart_cwd_decision.is_none() {
                // The gate state lives in the authoritative snapshot; refetch
                // is not available here, so tell the user the valid replies.
                println!(
                    "{}",
                    dim_line("[hint] restart cwd decision pending; reply !r (use host dir) or !k (keep session dir)")
                );
            }
            true
        }
        _ => true,
    }
}

fn register_live_decision_request(payload: &Value, pending_decisions: &mut Vec<PendingDecision>) {
    let mut seen = std::collections::HashSet::new();
    register_decision_request(payload, pending_decisions, &mut seen);
}

fn live_topic_summary(payload: &Value) -> Option<String> {
    if decision_request_prompt(payload).is_some() {
        // Decision requests print their interactive prompt instead.
        return None;
    }
    topic_summary(payload)
}

/// A finished turn never re-broadcasts a full `turn_updated` snapshot with the
/// final answer; the authoritative text rides on `turn_finished.outcome`.
fn handle_turn_finished(
    value: &Value,
    turn_views: &mut std::collections::HashMap<String, AttachTurnView>,
) {
    let turn_id = value
        .get("turn_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let text = value
        .get("outcome")
        .and_then(|outcome| outcome.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let view = turn_views
        .entry(turn_id)
        .or_insert_with(AttachTurnView::new);
    view.render_final_answer_once(text);
    close_dots();
    println!(
        "{}",
        dim_line(&format!("──── turn finished · {} ────", local_time_label()))
    );
    print_attach_prompt();
}

#[allow(clippy::type_complexity)]
fn handle_wire_event_inner(
    inner: &Value,
    session_id: &str,
    turn_views: &mut std::collections::HashMap<String, AttachTurnView>,
    pending_decisions: &mut Vec<PendingDecision>,
    restart_cwd_decision: &mut Option<Value>,
) -> bool {
    let event_type = inner.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "hello" => {
            // Initial snapshot: render the latest turn tail for orientation.
            let Some(snapshot) = inner.get("snapshot") else {
                return true;
            };
            let Some(sessions) = snapshot.get("sessions").and_then(Value::as_array) else {
                return true;
            };
            let Some(target) = sessions
                .iter()
                .find(|s| s.get("session_id").and_then(Value::as_str) == Some(session_id))
            else {
                return true;
            };
            let working = target.get("state").and_then(Value::as_str) == Some("working");
            if let Some(decision) = target.get("restart_cwd_decision") {
                let decision = decision.clone();
                if let Some(prompt) = restart_cwd_prompt(&decision) {
                    println!("{prompt}");
                }
                *restart_cwd_decision = Some(decision);
            }
            println!(
                "{}",
                dim_line(if working {
                    "[attached] session is working"
                } else {
                    "[attached] session is idle"
                })
            );
            print_attach_prompt();
            if let Some(turns) = target.get("turns").and_then(Value::as_array) {
                if let Some(latest) = turns.last() {
                    let turn_id = latest
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let view = turn_views
                        .entry(turn_id)
                        .or_insert_with(AttachTurnView::new);
                    view.render_turn(latest, pending_decisions);
                }
            }
            true
        }
        "session_restart_cwd_resolved" => {
            if inner
                .get("session")
                .and_then(|s| s.get("session_id"))
                .and_then(Value::as_str)
                == Some(session_id)
                && restart_cwd_decision.take().is_some()
            {
                println!("{}", dim_line("[restart cwd resolved]"));
            }
            true
        }
        "turn_updated" => {
            if inner.get("session_id").and_then(Value::as_str) != Some(session_id) {
                return true;
            }
            let Some(turn) = inner.get("turn") else {
                return true;
            };
            let turn_id = turn
                .get("turn_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let view = turn_views
                .entry(turn_id)
                .or_insert_with(AttachTurnView::new);
            view.render_turn(turn, pending_decisions);
            true
        }
        "turn_finished" => {
            if inner.get("session_id").and_then(Value::as_str) == Some(session_id) {
                handle_turn_finished(inner, turn_views);
            }
            true
        }
        "host_error" => {
            let error = inner.get("error").and_then(Value::as_str).unwrap_or("");
            println!("[host error] {error}");
            true
        }
        // semantic_event-wrapped streaming traffic: mid-turn Core actions and
        // worker activity must render live, not only after turn_updated.
        "core_topic" | "worker_activity" => {
            if inner.get("session_id").and_then(Value::as_str) != Some(session_id) {
                return true;
            }
            close_dots();
            let payload = inner.get("event").cloned().unwrap_or(Value::Null);
            if event_type == "core_topic" {
                register_live_decision_request(&payload, pending_decisions);
                if let Some(line) = live_topic_summary(&payload) {
                    close_dots();
                    println!("  {line}");
                }
            } else if let Some(line) = worker_event_summary(&payload) {
                println!("  {line}");
            }
            true
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision_payload(topic: &str, request: Value) -> Value {
        json!({
            "worker_id": "worker-1",
            "topic": {
                "name": topic,
                "attributes": {"expects_reply": true},
            },
            "payload": {
                "request_id": "req-1",
                "request": request,
            },
        })
    }

    #[test]
    fn decision_request_prompt_renders_approval() {
        let payload = decision_payload(
            "core.user.approval.request",
            json!({
                "action": "run",
                "command": "cargo test",
                "risk": "medium",
            }),
        );
        let prompt = decision_request_prompt(&payload).expect("approval prompt");
        assert!(prompt.contains("approve run: cargo test"));
        assert!(prompt.contains("risk: medium"));
        assert!(prompt.contains("!y"));
        assert!(prompt.contains("!n"));
        assert!(prompt.contains("!a"));
    }

    #[test]
    fn decision_request_prompt_ignores_non_expecting_topics() {
        let mut payload = decision_payload(
            "core.user.approval.request",
            json!({"action": "run", "command": "ls"}),
        );
        payload["topic"]["attributes"]["expects_reply"] = json!(false);
        assert!(decision_request_prompt(&payload).is_none());
    }

    #[test]
    fn decision_request_prompt_renders_round_limit() {
        let payload = decision_payload(
            "core.user.round_limit.request",
            json!({"recharge_rounds": 8}),
        );
        let prompt = decision_request_prompt(&payload).expect("round limit prompt");
        assert!(prompt.contains("8 more rounds"));
    }

    #[test]
    fn register_decision_request_dedupes_and_records() {
        let payload = decision_payload(
            "core.user.approval.request",
            json!({"action": "run", "command": "make"}),
        );
        let mut pending = Vec::new();
        let mut seen = std::collections::HashSet::new();
        register_decision_request(&payload, &mut pending, &mut seen);
        register_decision_request(&payload, &mut pending, &mut seen);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_id.as_deref(), Some("req-1"));
        assert_eq!(pending[0].worker_id.as_deref(), Some("worker-1"));
        assert_eq!(pending[0].topic_name, "core.user.approval.request");

        // A different request_id is registered separately.
        let mut other = payload.clone();
        other["payload"]["request_id"] = json!("req-2");
        register_decision_request(&other, &mut pending, &mut seen);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn action_detail_shows_command_and_status() {
        let payload = json!({
            "action": "run_bash",
            "event": "start",
            "status": "running",
            "input": {"cmd": "cargo build --release"},
        });
        let line = action_detail(&payload).expect("action detail");
        let plain: String = strip_ansi(&line);
        assert_eq!(plain, "> run_bash `cargo build --release` running");
    }

    #[test]
    fn action_detail_marks_failed_status_in_red() {
        let payload = json!({
            "action": "run_bash",
            "event": "finish",
            "status": "error",
            "input": {"cmd": "false"},
        });
        let line = action_detail(&payload).expect("action detail");
        assert!(line.contains(ANSI_FAIL));
    }

    #[test]
    fn restart_cwd_prompt_lists_both_dirs_and_replies() {
        let decision = json!({
            "runtime_cwd": "/host/dir",
            "session_cwd": "/session/dir",
            "session_cwd_available": true,
        });
        let prompt = restart_cwd_prompt(&decision).expect("prompt");
        assert!(prompt.contains("/host/dir"));
        assert!(prompt.contains("/session/dir"));
        assert!(prompt.contains("!r"));
        assert!(prompt.contains("!k"));
    }

    #[test]
    fn user_echo_matches_shell_style() {
        let echo = format_user_echo("你好");
        assert!(echo.contains("You ❯❯"));
        assert!(echo.ends_with("你好"));
    }

    fn strip_ansi(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn action_detail_truncates_long_commands() {
        let long_cmd = "x".repeat(200);
        let payload = json!({
            "action": "run_bash",
            "event": "finish",
            "status": "ok",
            "input": {"cmd": long_cmd},
        });
        let line = action_detail(&payload).expect("action detail");
        assert!(line.contains("..."));
        assert!(line.chars().count() < 200);
    }

    #[test]
    fn topic_summary_passes_through_non_action_topics() {
        let payload = json!({
            "topic": {"name": "core.model_health"},
            "event": "degraded",
        });
        assert_eq!(
            topic_summary(&payload),
            Some("- [core.model_health] degraded".to_string())
        );
    }

    #[test]
    fn final_answer_printed_once() {
        let mut view = AttachTurnView::new();
        view.render_final_answer_once("answer one");
        view.render_final_answer_once("answer two");
        view.render_final_answer_once("");
        assert!(view.final_answer_printed);
    }
}

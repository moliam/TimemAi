//! `timem attach`: attach a terminal to a session owned by a running Web Host.
//!
//! The shell is a plain client here: it discovers the Host through the
//! workspace `web_instance.json` lease, lists sessions over the read-only
//! attach endpoint, and then renders authoritative Host events over the
//! existing WebSocket command transport. It never owns domain state.

use crate::{
    dim_line, local_time_label, render_final_answer_markdown, ANSI_BRIGHT_TIMEM, ANSI_DIM,
    ANSI_RESET, TIMEM_LOGO,
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

mod view;

use view::{
    activity_summary, attach_error_card, connected_intro, decision_request_prompt,
    disconnected_card, format_user_echo, guidance_card, host_error_card, invalid_command_card,
    invalid_restart_choice_card, no_sessions_card, rejected_command_card, restart_cwd_prompt,
    session_selector, topic_summary, worker_event_summary,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ATTACH_TURN_VIEWS: usize = 200;
const MAX_ATTACH_PENDING_DECISIONS: usize = 200;

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
            Self::NoHost => "No running Web Host was found for this workspace.".to_string(),
            Self::HostUnreachable(detail) => format!("The Web Host could not be reached: {detail}"),
            Self::Http(detail) => format!("The attach endpoint failed: {detail}"),
            Self::Protocol(detail) => {
                format!("The Host returned an invalid attach response: {detail}")
            }
        }
    }

    fn next_step(&self) -> &'static str {
        match self {
            Self::NoHost => "Start Timem in this workspace, then run `timem attach` again.",
            Self::HostUnreachable(_) => "Check that the Web Host is still running, then reconnect.",
            Self::Http(_) | Self::Protocol(_) => {
                "Restart the Web Host if the problem persists, then reconnect."
            }
        }
    }
}

/// Entry point for `timem attach`. `space` selects the workspace whose Host
/// lease should be discovered.
pub fn run_attach(space: Option<&str>) {
    let memory_dir = match crate::resolve_memory_dir(space) {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "{}",
                guidance_card(
                    "Unable to attach",
                    &error.to_string(),
                    "Check the workspace path and try again."
                )
            );
            std::process::exit(2);
        }
    };
    let instance_path = memory_dir.join("web_instance.json");
    let host = match discover_host(&instance_path) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("{}", attach_error_card(&error));
            std::process::exit(2);
        }
    };
    let sessions = match fetch_attach_sessions(&host) {
        Ok(sessions) => sessions,
        Err(error) => {
            eprintln!("{}", attach_error_card(&error));
            std::process::exit(2);
        }
    };
    if sessions.is_empty() {
        eprintln!("{}", no_sessions_card());
        std::process::exit(2);
    }
    let selected = match select_session(&sessions) {
        Some(session) => session,
        None => return,
    };
    if let Err(error) = attach_session(&host, &selected) {
        eprintln!("{}", attach_error_card(&error));
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
        print!("{}", session_selector(sessions, selected));
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
    let worker_id = payload
        .get("worker_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let topic_name = payload
        .get("topic")
        .and_then(|topic| topic.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let dedupe_key = request_id
        .clone()
        .unwrap_or_else(|| format!("{topic_name}|{}", worker_id.as_deref().unwrap_or_default()));
    if !seen.insert(dedupe_key)
        || pending.iter().any(|existing| {
            request_id
                .as_ref()
                .is_some_and(|id| existing.request_id.as_ref() == Some(id))
                || (request_id.is_none()
                    && existing.request_id.is_none()
                    && existing.worker_id == worker_id
                    && existing.topic_name == topic_name)
        })
    {
        return;
    }
    println!("{prompt}");
    if pending.len() >= MAX_ATTACH_PENDING_DECISIONS {
        pending.remove(0);
    }
    pending.push(PendingDecision {
        request_id,
        worker_id,
        topic_name,
    });
}

#[derive(Default)]
struct AttachTurnViews {
    views: std::collections::HashMap<String, AttachTurnView>,
    order: std::collections::VecDeque<String>,
}

impl AttachTurnViews {
    fn view_mut(&mut self, turn_id: String) -> &mut AttachTurnView {
        if !self.views.contains_key(&turn_id) {
            while self.views.len() >= MAX_ATTACH_TURN_VIEWS {
                let Some(oldest) = self.order.pop_front() else {
                    break;
                };
                self.views.remove(&oldest);
            }
            self.order.push_back(turn_id.clone());
            self.views.insert(turn_id.clone(), AttachTurnView::new());
        }
        self.views
            .get_mut(&turn_id)
            .expect("inserted attach turn view must exist")
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.views.len()
    }

    #[cfg(test)]
    fn contains(&self, turn_id: &str) -> bool {
        self.views.contains_key(turn_id)
    }
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

/// Shell-style attach prompt, shown while the session is idle.
fn print_attach_prompt() {
    close_dots();
    print!(
        "\x1b[94;1m[{}] {TIMEM_LOGO} attach ❯❯{ANSI_RESET} ",
        local_time_label()
    );
    let _ = std::io::stdout().flush();
}

fn restart_cwd_numeric_decision(
    text: &str,
    gate_pending: bool,
) -> Option<Result<&'static str, ()>> {
    if !gate_pending {
        return None;
    }
    match text {
        "1" => Some(Ok("keep_session")),
        "2" => Some(Ok("use_runtime")),
        "3" => Some(Err(())),
        _ => None,
    }
}

/// Connects to the Host WebSocket and runs the attach loop: stream-render
/// authoritative events for the chosen session and submit every entered line
/// as a forced supplement (no message queueing).
fn attach_session(host: &HostEndpoint, session: &AttachSession) -> Result<(), AttachError> {
    let initial_restart_decision = session.restart_cwd_decision.clone();
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

    let mut turn_views = AttachTurnViews::default();
    let mut pending_decisions: Vec<PendingDecision> = Vec::new();
    let mut restart_cwd_decision: Option<Value> = initial_restart_decision;
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
            if let Some(decision) =
                restart_cwd_numeric_decision(&text, restart_cwd_decision.is_some())
            {
                let Ok(decision) = decision else {
                    close_dots();
                    println!("{}", invalid_restart_choice_card(&text));
                    print_attach_prompt();
                    continue;
                };
                let message = json!({
                    "command_id": command_id,
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
                    close_dots();
                    println!("{}", invalid_command_card(&text));
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
                    session,
                    &mut turn_views,
                    &mut pending_decisions,
                    &mut restart_cwd_decision,
                ) {
                    return Ok(());
                }
            }
            Ok(Message::Close(_)) => {
                close_dots();
                println!("{}", disconnected_card(None));
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
                    close_dots();
                    println!("{}", disconnected_card(None));
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
                    close_dots();
                    println!(
                        "{}",
                        disconnected_card(Some(&format!("WebSocket protocol error: {other}")))
                    );
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
    session: &AttachSession,
    turn_views: &mut AttachTurnViews,
    pending_decisions: &mut Vec<PendingDecision>,
    restart_cwd_decision: &mut Option<Value>,
) -> bool {
    let session_id = session.session_id.as_str();
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
                session,
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
            } else if let Some(line) = worker_event_summary(&payload) {
                println!("  {line}");
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
                println!("{}", rejected_command_card(error));
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
                            guidance_card(
                                "Working-directory choice required",
                                "The authoritative choice could not be reloaded.",
                                "Reply `!r` to use the Host directory or `!k` to keep the session directory.",
                            )
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
                println!("{}", host_error_card(error));
            }
            if error == "session_restart_cwd_decision_required" && restart_cwd_decision.is_none() {
                // The gate state lives in the authoritative snapshot; refetch
                // is not available here, so tell the user the valid replies.
                println!(
                    "{}",
                    guidance_card(
                        "Working-directory choice required",
                        "The Host is waiting for a restored-session directory choice.",
                        "Reply `!r` to use the Host directory or `!k` to keep the session directory.",
                    )
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
fn handle_turn_finished(value: &Value, turn_views: &mut AttachTurnViews) {
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
    let view = turn_views.view_mut(turn_id);
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
    session: &AttachSession,
    turn_views: &mut AttachTurnViews,
    pending_decisions: &mut Vec<PendingDecision>,
    restart_cwd_decision: &mut Option<Value>,
) -> bool {
    let session_id = session.session_id.as_str();
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
            close_dots();
            println!("{}", connected_intro(session, working));
            if let Some(decision) = target.get("restart_cwd_decision") {
                let decision = decision.clone();
                if let Some(prompt) = restart_cwd_prompt(&decision) {
                    println!("{prompt}");
                }
                *restart_cwd_decision = Some(decision);
            } else {
                *restart_cwd_decision = None;
            }
            if let Some(turns) = target.get("turns").and_then(Value::as_array) {
                if let Some(latest) = turns.last() {
                    let turn_id = latest
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let view = turn_views.view_mut(turn_id);
                    view.render_turn(latest, pending_decisions);
                }
            }
            print_attach_prompt();
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
                println!("{}", dim_line("Working directory updated."));
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
            let view = turn_views.view_mut(turn_id);
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
            if !error.is_empty() {
                println!("{}", host_error_card(error));
            }
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
    fn pending_decisions_dedupe_live_replays_and_evict_oldest_at_limit() {
        let mut pending = Vec::new();
        let mut first = decision_payload(
            "core.user.approval.request",
            json!({"action": "run", "command": "first"}),
        );
        register_live_decision_request(&first, &mut pending);
        register_live_decision_request(&first, &mut pending);
        assert_eq!(
            pending.len(),
            1,
            "live replay must not duplicate request_id"
        );

        for index in 2..=(MAX_ATTACH_PENDING_DECISIONS + 5) {
            first["payload"]["request_id"] = json!(format!("req-{index}"));
            register_live_decision_request(&first, &mut pending);
        }
        assert_eq!(pending.len(), MAX_ATTACH_PENDING_DECISIONS);
        assert!(
            pending
                .iter()
                .all(|item| item.request_id.as_deref() != Some("req-1")),
            "oldest pending decision must be evicted"
        );
        assert_eq!(
            pending.last().and_then(|item| item.request_id.as_deref()),
            Some("req-205"),
            "newest decision remains the reply target"
        );
    }

    #[test]
    fn restart_cwd_gate_accepts_only_its_two_numbered_choices() {
        assert_eq!(
            restart_cwd_numeric_decision("1", true),
            Some(Ok("keep_session"))
        );
        assert_eq!(
            restart_cwd_numeric_decision("2", true),
            Some(Ok("use_runtime"))
        );
        assert_eq!(restart_cwd_numeric_decision("3", true), Some(Err(())));
        assert_eq!(restart_cwd_numeric_decision("3", false), None);
    }

    #[test]
    fn attach_turn_views_evict_oldest_state_at_host_turn_limit() {
        let mut views = AttachTurnViews::default();
        for index in 0..(MAX_ATTACH_TURN_VIEWS + 5) {
            views.view_mut(format!("turn-{index}"));
        }
        assert_eq!(views.len(), MAX_ATTACH_TURN_VIEWS);
        assert!(!views.contains("turn-0"));
        assert!(!views.contains("turn-4"));
        assert!(views.contains("turn-5"));
        assert!(views.contains(&format!("turn-{}", MAX_ATTACH_TURN_VIEWS + 4)));
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

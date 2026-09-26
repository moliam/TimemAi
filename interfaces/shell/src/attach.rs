//! `timem attach`: attach a terminal to a session owned by a running Web Host.
//!
//! The shell is a plain client here: it discovers the Host through the
//! workspace `web_instance.json` lease, lists sessions over the read-only
//! attach endpoint, and then renders authoritative Host events over the
//! existing WebSocket command transport. It never owns domain state.

use crate::render_final_answer_markdown;
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
    println!("Select a session to attach (up/down to move, Enter to attach, Esc to quit):");
    let mut selected = 0usize;
    let _ = enable_raw_mode();
    let render = |selected: usize| {
        let mut out = String::from("\r\n");
        for (index, session) in sessions.iter().enumerate() {
            let marker = if index == selected { ">" } else { " " };
            let work = if session.working { " [working]" } else { "" };
            let line = format!(
                "{marker} {}. {}{} (workers: {}, dir: {})",
                index + 1,
                session.display_name,
                work,
                session.worker_count,
                session.current_dir
            );
            out.push_str(&line);
            out.push_str("\r\n");
        }
        print!("{out}");
        let _ = std::io::stdout().flush();
    };
    render(selected);
    let result = loop {
        match crossterm::event::read() {
            Ok(Event::Key(KeyEvent {
                code, modifiers, ..
            })) => match (code, modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => break None,
                (KeyCode::Esc, _) => break None,
                (KeyCode::Up, _) if selected > 0 => {
                    selected -= 1;
                    render(selected);
                }
                (KeyCode::Down, _) if selected + 1 < sessions.len() => {
                    selected += 1;
                    render(selected);
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

struct AttachTurnView {
    printed_event_ids: std::collections::HashSet<String>,
    printed_user_entries: usize,
    printed_sub_answers: usize,
    final_answer_printed: bool,
}

impl AttachTurnView {
    fn new() -> Self {
        Self {
            printed_event_ids: std::collections::HashSet::new(),
            printed_user_entries: 0,
            printed_sub_answers: 0,
            final_answer_printed: false,
        }
    }

    fn render_turn(&mut self, turn: &Value) {
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
                print_turn_event(event);
            }
        }
        if let Some(sub_answers) = turn.get("sub_answers").and_then(Value::as_array) {
            while self.printed_sub_answers < sub_answers.len() {
                let sub = &sub_answers[self.printed_sub_answers];
                self.printed_sub_answers += 1;
                let task = sub.get("task").and_then(Value::as_str).unwrap_or("");
                let answer = sub.get("answer").and_then(Value::as_str).unwrap_or("");
                println!("\n[sub-answer: {task}]\n{answer}");
            }
        }
        let final_answer = turn
            .get("final_answer")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty());
        if let Some(answer) = final_answer {
            if !self.final_answer_printed {
                self.final_answer_printed = true;
                println!("\n{}", render_final_answer_markdown(answer));
            }
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
    let event_name = payload.get("event").and_then(Value::as_str).unwrap_or("");
    Some(format!("- [{topic}] {event_name}").trim_end().to_string())
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
        "Attaching to session \"{}\" ({}). Lines you enter are sent as forced supplements; Ctrl+C to detach.",
        session.display_name, session.session_id
    );
    let token_query = host
        .token
        .as_deref()
        .map(|t| format!("?token={t}"))
        .unwrap_or_default();
    let url = format!("ws://127.0.0.1:{}/ws{}", host.port, token_query);
    let request = url
        .into_client_request()
        .map_err(|e| AttachError::Protocol(e.to_string()))?;
    let (mut ws, _response) =
        tungstenite::connect(request).map_err(|e| AttachError::HostUnreachable(e.to_string()))?;

    // Terminal input on a worker thread; WebSocket reads block the main
    // loop so streaming output keeps flowing while the user types.
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

    let mut turn_views: std::collections::HashMap<String, AttachTurnView> =
        std::collections::HashMap::new();
    let session_id = session.session_id.clone();
    let mut command_sequence: u64 = 0;

    loop {
        // Drain pending user input first so supplements are not delayed by
        // streaming traffic.
        while let Ok(text) = input_rx.try_recv() {
            if text.is_empty() {
                continue;
            }
            command_sequence += 1;
            let command_id = format!("attach_{}_{}", std::process::id(), command_sequence);
            let message = json!({
                "command_id": command_id,
                "type": "turn_supplement",
                "session_id": session_id,
                "text": text,
            });
            ws.write(Message::Text(message.to_string()))
                .map_err(|e| AttachError::HostUnreachable(e.to_string()))?;
            println!("[sent as forced supplement]");
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if !handle_wire_event(&text, &session_id, &mut turn_views) {
                    return Ok(());
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(_) => {
                // A quiet socket after detach input ends the loop naturally.
                if input_rx.try_recv().is_err() && !ws.can_read() {
                    return Ok(());
                }
            }
        }
    }
}

fn handle_wire_event(
    text: &str,
    session_id: &str,
    turn_views: &mut std::collections::HashMap<String, AttachTurnView>,
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
            handle_wire_event_inner(&inner, session_id, turn_views)
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
                    print!(".");
                    let _ = std::io::stdout().flush();
                }
            }
            true
        }
        "turn_finished" => {
            if value.get("session_id").and_then(Value::as_str) == Some(session_id) {
                println!("\n[turn finished]");
            }
            true
        }
        "host_error" => {
            let error = value.get("error").and_then(Value::as_str).unwrap_or("");
            println!("[host error] {error}");
            true
        }
        _ => true,
    }
}

fn handle_wire_event_inner(
    inner: &Value,
    session_id: &str,
    turn_views: &mut std::collections::HashMap<String, AttachTurnView>,
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
            println!(
                "[attached] {}",
                if working {
                    "session is working"
                } else {
                    "session is idle"
                }
            );
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
                    view.render_turn(latest);
                }
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
            view.render_turn(turn);
            true
        }
        "turn_finished" => {
            if inner.get("session_id").and_then(Value::as_str) == Some(session_id) {
                println!("\n[turn finished]");
            }
            true
        }
        "host_error" => {
            let error = inner.get("error").and_then(Value::as_str).unwrap_or("");
            println!("[host error] {error}");
            true
        }
        _ => true,
    }
}

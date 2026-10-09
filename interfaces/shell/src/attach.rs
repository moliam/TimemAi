//! `timem attach`: attach a terminal to a session owned by a running Web Host.
//!
//! The shell is a plain client here: it discovers the Host through the
//! workspace `web_instance.json` lease, lists sessions over the read-only
//! attach endpoint, and then renders authoritative Host events over the
//! existing WebSocket command transport. It never owns domain state.

use crate::{
    dim_line, local_time_label, render_final_answer_markdown, render_stream_preview,
    render_stream_tool_fold, resolve_ui_mode, select_ui_mode, ShellUiMode, StreamPreviewState,
    StreamPreviewUpdate, StreamToolFoldState, UiModeResolution, ANSI_BRIGHT_TIMEM, ANSI_DIM,
    ANSI_RESET, CORE_TOPIC_ACTION, TIMEM_LOGO, UI_MODE_ENV,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::net::TcpStream;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;
use unicode_width::UnicodeWidthStr;

mod view;

use view::{
    activity_summary, attach_error_card, connected_intro, decision_request_prompt,
    disconnected_card, format_user_echo, guidance_card, host_error_card, instance_selector,
    invalid_command_card, invalid_restart_choice_card, invalid_restart_recovery_input_card,
    no_sessions_card, rejected_command_card, restart_cwd_prompt, restart_cwd_recovery_prompt,
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
    #[serde(default)]
    started_at_ms: Option<u128>,
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

#[derive(Debug)]
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

/// Entry point for `timem attach`. An explicit `--space` keeps the
/// directed single-MEM contract. Without it, the user-level registry is only
/// an index: every candidate is revalidated against its authoritative lease
/// and health endpoint before any token is used.
pub fn run_attach(space: Option<&str>, explicit_ui_mode: Option<&str>) {
    let host = match resolve_attach_host(space) {
        Ok(Some(host)) => host,
        Ok(None) => return,
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
    let env_ui_mode = std::env::var(UI_MODE_ENV).ok();
    let interactive_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let ui_mode = match resolve_ui_mode(
        explicit_ui_mode,
        env_ui_mode.as_deref(),
        interactive_tty,
        false,
    ) {
        Ok(UiModeResolution::Resolved(mode)) => mode,
        Ok(UiModeResolution::Select) => match select_ui_mode() {
            Some(mode) => mode,
            None => return,
        },
        Err(error) => {
            eprintln!("[config_error] {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = attach_session(&host, &selected, ui_mode) {
        eprintln!("{}", attach_error_card(&error));
        std::process::exit(2);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostEndpoint {
    port: u16,
    token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AttachHostCandidate {
    pub memory_dir: PathBuf,
    pub pid: u32,
    pub started_at_ms: u128,
    host: HostEndpoint,
}

fn resolve_attach_host(space: Option<&str>) -> Result<Option<HostEndpoint>, AttachError> {
    if let Some(space) = space {
        let memory_dir = crate::resolve_memory_dir(Some(space)).map_err(AttachError::Protocol)?;
        return discover_host(&memory_dir.join("web_instance.json")).map(Some);
    }

    let candidates = timem_in_process::agent_api::web_instance_registry_dir()
        .ok()
        .map(|registry_dir| discover_registered_hosts(&registry_dir))
        .unwrap_or_default();
    match registered_host_route(candidates) {
        RegisteredHostRoute::DefaultMemory => {
            let memory_dir = crate::resolve_memory_dir(None).map_err(AttachError::Protocol)?;
            discover_host(&memory_dir.join("web_instance.json")).map(Some)
        }
        RegisteredHostRoute::Direct(host) => Ok(Some(host)),
        RegisteredHostRoute::Select(candidates) => {
            Ok(select_host_instance(&candidates).map(|candidate| candidate.host))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RegisteredHostRoute {
    DefaultMemory,
    Direct(HostEndpoint),
    Select(Vec<AttachHostCandidate>),
}

fn registered_host_route(candidates: Vec<AttachHostCandidate>) -> RegisteredHostRoute {
    match candidates.len() {
        0 => RegisteredHostRoute::DefaultMemory,
        1 => RegisteredHostRoute::Direct(
            candidates
                .into_iter()
                .next()
                .expect("single candidate must exist")
                .host,
        ),
        _ => RegisteredHostRoute::Select(candidates),
    }
}

fn discover_host(instance_path: &Path) -> Result<HostEndpoint, AttachError> {
    discover_host_with(instance_path, |host| {
        host_responds(host.port, host.token.as_deref())
    })
}

fn discover_host_with(
    instance_path: &Path,
    mut healthy: impl FnMut(&HostEndpoint) -> bool,
) -> Result<HostEndpoint, AttachError> {
    let raw = std::fs::read(instance_path).map_err(|_| AttachError::NoHost)?;
    let info: WebInstanceFile = serde_json::from_slice(&raw).map_err(|_| AttachError::NoHost)?;
    let port = info.port.ok_or(AttachError::NoHost)?;
    let host = HostEndpoint {
        port,
        token: info.token,
    };
    if !healthy(&host) {
        return Err(AttachError::NoHost);
    }
    Ok(host)
}

fn discover_registered_hosts(registry_dir: &Path) -> Vec<AttachHostCandidate> {
    discover_registered_hosts_with(registry_dir, |host| {
        host_responds(host.port, host.token.as_deref())
    })
}

fn discover_registered_hosts_with(
    registry_dir: &Path,
    mut healthy: impl FnMut(&HostEndpoint) -> bool,
) -> Vec<AttachHostCandidate> {
    let Ok(entries) = std::fs::read_dir(registry_dir) else {
        return Vec::new();
    };
    let mut by_memory = BTreeMap::<PathBuf, AttachHostCandidate>::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let Ok(record) =
            serde_json::from_slice::<timem_in_process::agent_api::WebInstanceRegistryRecord>(&raw)
        else {
            continue;
        };
        if path.file_name().and_then(|name| name.to_str())
            != Some(format!("{}.json", record.registration_id).as_str())
            || !record.memory_dir.is_absolute()
        {
            continue;
        }
        let Ok(lease_raw) = std::fs::read(record.memory_dir.join("web_instance.json")) else {
            continue;
        };
        let Ok(lease) = serde_json::from_slice::<WebInstanceFile>(&lease_raw) else {
            continue;
        };
        if lease.pid != record.pid || lease.started_at_ms != Some(record.started_at_ms) {
            continue;
        }
        let Some(port) = lease.port else {
            continue;
        };
        let host = HostEndpoint {
            port,
            token: lease.token,
        };
        if !healthy(&host) {
            continue;
        }
        let normalized_memory = normalize_memory_path(&record.memory_dir);
        let candidate = AttachHostCandidate {
            memory_dir: normalized_memory.clone(),
            pid: record.pid,
            started_at_ms: record.started_at_ms,
            host,
        };
        match by_memory.get(&normalized_memory) {
            Some(current)
                if (current.started_at_ms, current.pid, current.host.port)
                    >= (candidate.started_at_ms, candidate.pid, candidate.host.port) => {}
            _ => {
                by_memory.insert(normalized_memory, candidate);
            }
        }
    }
    let mut candidates = by_memory.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.memory_dir
            .to_string_lossy()
            .cmp(&right.memory_dir.to_string_lossy())
            .then_with(|| left.pid.cmp(&right.pid))
            .then_with(|| left.started_at_ms.cmp(&right.started_at_ms))
            .then_with(|| left.host.port.cmp(&right.host.port))
    });
    candidates
}

fn normalize_memory_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn select_host_instance(candidates: &[AttachHostCandidate]) -> Option<AttachHostCandidate> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    println!("{ANSI_BRIGHT_TIMEM}{TIMEM_LOGO}{ANSI_RESET} {ANSI_DIM}attach{ANSI_RESET}");
    let mut selected = 0usize;
    let _ = enable_raw_mode();
    let line_count = candidates.len() + 1;
    let mut rendered_once = false;
    let render = |selected: usize, rendered_once: bool| {
        use crossterm::cursor::MoveUp;
        use crossterm::queue;
        use crossterm::terminal::{Clear, ClearType};
        let mut stdout = std::io::stdout();
        if rendered_once {
            let _ = queue!(
                stdout,
                MoveUp(line_count as u16),
                Clear(ClearType::FromCursorDown)
            );
        }
        print!("{}", instance_selector(candidates, selected));
        let _ = stdout.flush();
    };
    render(selected, rendered_once);
    rendered_once = true;
    let result = loop {
        match crossterm::event::read() {
            Ok(Event::Key(KeyEvent {
                code, modifiers, ..
            })) => match (code, modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Esc, _) => break None,
                (KeyCode::Up, _) if selected > 0 => {
                    selected -= 1;
                    render(selected, rendered_once);
                }
                (KeyCode::Down, _) if selected + 1 < candidates.len() => {
                    selected += 1;
                    render(selected, rendered_once);
                }
                (KeyCode::Enter, _) => break Some(candidates[selected].clone()),
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

    fn render_turn(
        &mut self,
        turn: &Value,
        pending: &mut Vec<PendingDecision>,
        stream_region: &mut AttachStreamRegion,
    ) {
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
                let turn_id = turn
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !stream_region.consume_snapshot_tool_event(turn_id, event) {
                    print_turn_event(event);
                }
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

fn attach_prompt_text() -> String {
    format!(
        "\x1b[94;1m[{}] {TIMEM_LOGO} attach ❯❯{ANSI_RESET} ",
        local_time_label()
    )
}

fn attach_strip_ansi(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn attach_terminal_width() -> usize {
    crossterm::terminal::size()
        .ok()
        .map(|(width, _)| usize::from(width))
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

fn attach_wrapped_rows(text: &str, terminal_width: usize) -> usize {
    let width = terminal_width.max(1);
    text.split('\n')
        .map(|line| {
            UnicodeWidthStr::width(attach_strip_ansi(line).as_str())
                .max(1)
                .div_ceil(width)
        })
        .sum::<usize>()
        .max(1)
}

fn attach_input_clear_rows(
    rendered_rows: usize,
    prompt: &str,
    input: &str,
    terminal_width: usize,
) -> usize {
    let prompt_rows = attach_wrapped_rows(prompt, terminal_width);
    let input_rows = attach_wrapped_rows(&format!("{prompt}{input}"), terminal_width);
    // stdin line mode echoes Enter and leaves the cursor one row below the
    // final wrapped input row. `rendered_rows` already includes the prompt;
    // add only wrapping introduced by the user's text.
    rendered_rows.saturating_add(input_rows.saturating_sub(prompt_rows))
}

fn attach_snapshot_orientation_turn<'a>(
    turns: &'a [Value],
    active_turn_id: Option<&str>,
) -> Option<&'a Value> {
    match active_turn_id {
        Some(active_turn_id) => turns
            .iter()
            .find(|turn| turn.get("turn_id").and_then(Value::as_str) == Some(active_turn_id)),
        None => turns.last(),
    }
}

#[derive(Clone, Debug)]
enum RestartCwdGate {
    Decision(Value),
    ReloadRequired,
}

struct AttachStreamRegion {
    mode: ShellUiMode,
    active_turn_id: Option<String>,
    preview: StreamPreviewState,
    tools: StreamToolFoldState,
    rendered_rows: usize,
    prompt_visible: bool,
    restart_cwd_gate: Option<RestartCwdGate>,
}

impl AttachStreamRegion {
    fn new(mode: ShellUiMode, active_turn_id: Option<String>) -> Self {
        Self {
            mode,
            active_turn_id,
            preview: StreamPreviewState::default(),
            tools: StreamToolFoldState::default(),
            rendered_rows: 0,
            prompt_visible: false,
            restart_cwd_gate: None,
        }
    }

    fn content(&self) -> String {
        if self.mode != ShellUiMode::Stream {
            return String::new();
        }
        let mut output = render_stream_tool_fold(&self.tools);
        if let Some(snapshot) = self
            .preview
            .snapshot()
            .filter(|snapshot| self.active_turn_id.as_deref() == Some(snapshot.turn_id.as_str()))
        {
            let preview = render_stream_preview(snapshot);
            if !preview.is_empty() {
                output.push_str(&preview);
                output.push('\n');
            }
        }
        if self.restart_cwd_gate.is_some() || self.prompt_visible {
            output.push_str(&self.visible_prompt_text());
        }
        output
    }

    fn set_restart_cwd_gate(&mut self, gate: Option<RestartCwdGate>) {
        self.restart_cwd_gate = gate;
    }

    fn visible_prompt_text(&self) -> String {
        match self.restart_cwd_gate.as_ref() {
            Some(RestartCwdGate::Decision(decision)) => {
                restart_cwd_prompt(decision).unwrap_or_else(attach_prompt_text)
            }
            Some(RestartCwdGate::ReloadRequired) => restart_cwd_recovery_prompt(),
            None => attach_prompt_text(),
        }
    }

    fn set_active_turn(&mut self, turn_id: Option<String>) {
        if self.mode != ShellUiMode::Stream || self.active_turn_id == turn_id {
            return;
        }
        self.active_turn_id = turn_id;
        self.preview.clear();
        self.tools.clear();
    }

    fn finish_turn(&mut self, turn_id: &str) -> String {
        if self.mode != ShellUiMode::Stream || self.active_turn_id.as_deref() != Some(turn_id) {
            return String::new();
        }
        self.active_turn_id = None;
        self.preview.clear();
        let summary = render_stream_tool_fold(&self.tools);
        self.tools.clear();
        summary
    }

    fn consume_snapshot_tool_event(&mut self, turn_id: &str, event: &Value) -> bool {
        let payload = event.get("payload").unwrap_or(&Value::Null);
        self.consume_tool_topic(turn_id, payload)
    }

    fn consume_tool_topic(&mut self, turn_id: &str, event: &Value) -> bool {
        if self.mode != ShellUiMode::Stream
            || self.active_turn_id.as_deref() != Some(turn_id)
            || event
                .get("topic")
                .and_then(|topic| topic.get("name"))
                .and_then(Value::as_str)
                != Some(CORE_TOPIC_ACTION)
        {
            return false;
        }
        let _ = self.tools.apply_json_topic(event);
        true
    }

    fn apply_preview_payload(&mut self, payload: &Value) -> StreamPreviewUpdate {
        if self.mode != ShellUiMode::Stream
            || payload.get("turn_id").and_then(Value::as_str) != self.active_turn_id.as_deref()
        {
            return StreamPreviewUpdate::Ignored;
        }
        self.preview.apply_payload(payload)
    }

    fn apply_preview_topic(&mut self, event: &Value) -> StreamPreviewUpdate {
        let Some(payload) = event.get("payload") else {
            return StreamPreviewUpdate::Ignored;
        };
        let Some(topic_name) = event
            .get("topic")
            .and_then(|topic| topic.get("name"))
            .and_then(Value::as_str)
        else {
            return StreamPreviewUpdate::Ignored;
        };
        if topic_name != crate::CORE_TOPIC_MODEL_PREVIEW {
            return StreamPreviewUpdate::Ignored;
        }
        self.apply_preview_payload(payload)
    }

    #[cfg(test)]
    fn rendered_row_count(&self, terminal_width: usize) -> usize {
        let content = self.content();
        if content.is_empty() {
            0
        } else {
            attach_wrapped_rows(&content, terminal_width)
        }
    }

    fn clear_into(&mut self, output: &mut impl Write) -> std::io::Result<()> {
        if self.mode != ShellUiMode::Stream || self.rendered_rows == 0 {
            return Ok(());
        }
        use crossterm::cursor::{MoveToColumn, MoveUp};
        use crossterm::queue;
        use crossterm::terminal::{Clear, ClearType};

        queue!(output, MoveToColumn(0))?;
        let mut rows_up = self.rendered_rows.saturating_sub(1);
        while rows_up > 0 {
            let step = rows_up.min(usize::from(u16::MAX)) as u16;
            queue!(output, MoveUp(step))?;
            rows_up -= usize::from(step);
        }
        queue!(output, Clear(ClearType::FromCursorDown))?;
        self.rendered_rows = 0;
        Ok(())
    }

    fn clear_for_input_into(
        &mut self,
        output: &mut impl Write,
        input: &str,
        terminal_width: usize,
    ) -> std::io::Result<()> {
        if self.mode != ShellUiMode::Stream || self.rendered_rows == 0 {
            return Ok(());
        }
        use crossterm::cursor::{MoveToColumn, MoveUp};
        use crossterm::queue;
        use crossterm::terminal::{Clear, ClearType};

        let prompt = self.visible_prompt_text();
        let mut rows_up =
            attach_input_clear_rows(self.rendered_rows, &prompt, input, terminal_width);
        queue!(output, MoveToColumn(0))?;
        while rows_up > 0 {
            let step = rows_up.min(usize::from(u16::MAX)) as u16;
            queue!(output, MoveUp(step))?;
            rows_up -= usize::from(step);
        }
        queue!(output, Clear(ClearType::FromCursorDown))?;
        self.rendered_rows = 0;
        self.prompt_visible = false;
        Ok(())
    }

    fn render_into(
        &mut self,
        output: &mut impl Write,
        terminal_width: usize,
    ) -> std::io::Result<()> {
        if self.mode != ShellUiMode::Stream {
            return Ok(());
        }
        self.clear_into(output)?;
        let content = self.content();
        if content.is_empty() {
            output.flush()?;
            return Ok(());
        }
        output.write_all(content.as_bytes())?;
        output.flush()?;
        self.rendered_rows = attach_wrapped_rows(&content, terminal_width);
        Ok(())
    }

    fn show_prompt(&mut self) {
        self.prompt_visible = true;
        if self.mode == ShellUiMode::Ordinary {
            if self.restart_cwd_gate.is_some() {
                close_dots();
                print!("{}", self.visible_prompt_text());
                let _ = std::io::stdout().flush();
            } else {
                print_attach_prompt();
            }
        }
    }
}

/// Shell-style attach prompt, shown while the session is idle.
fn print_attach_prompt() {
    close_dots();
    print!("{}", attach_prompt_text());
    let _ = std::io::stdout().flush();
}

#[derive(Debug, PartialEq, Eq)]
enum RestartCwdInput {
    Resolve(&'static str),
    Retry,
    Detach,
    Invalid,
    NotPending,
}

fn restart_cwd_input(text: &str, gate: Option<&RestartCwdGate>) -> RestartCwdInput {
    let Some(gate) = gate else {
        return RestartCwdInput::NotPending;
    };
    match gate {
        RestartCwdGate::Decision(decision) => {
            let session_cwd_available = decision
                .get("session_cwd_available")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match text {
                "1" | "!k" if session_cwd_available => RestartCwdInput::Resolve("keep_session"),
                "2" | "!r" => RestartCwdInput::Resolve("use_runtime"),
                _ => RestartCwdInput::Invalid,
            }
        }
        RestartCwdGate::ReloadRequired => match text {
            "" | "1" | "!retry" => RestartCwdInput::Retry,
            "2" | "!q" => RestartCwdInput::Detach,
            _ => RestartCwdInput::Invalid,
        },
    }
}

fn set_restart_cwd_gate(
    gate: &mut Option<RestartCwdGate>,
    stream_region: &mut AttachStreamRegion,
    next: Option<RestartCwdGate>,
) {
    *gate = next.clone();
    stream_region.set_restart_cwd_gate(next);
}

/// Connects to the Host WebSocket and runs the attach loop: stream-render
/// authoritative events for the chosen session and submit every entered line
/// as a forced supplement (no message queueing).
fn attach_session(
    host: &HostEndpoint,
    session: &AttachSession,
    ui_mode: ShellUiMode,
) -> Result<(), AttachError> {
    let initial_restart_gate = session
        .restart_cwd_decision
        .clone()
        .map(RestartCwdGate::Decision);
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
    let mut restart_cwd_gate = initial_restart_gate.clone();
    let session_id = session.session_id.clone();
    let mut stream_region = AttachStreamRegion::new(ui_mode, session.active_turn_id.clone());
    stream_region.set_restart_cwd_gate(initial_restart_gate);
    let mut command_sequence: u64 = 0;

    loop {
        // Drain pending user input first so lines are never delayed by
        // streaming traffic.
        while let Ok(text) = input_rx.try_recv() {
            if let Err(error) = stream_region.clear_for_input_into(
                &mut std::io::stdout(),
                &text,
                attach_terminal_width(),
            ) {
                return Err(AttachError::Protocol(error.to_string()));
            }
            if text.is_empty()
                && !matches!(
                    restart_cwd_gate.as_ref(),
                    Some(RestartCwdGate::ReloadRequired)
                )
            {
                stream_region.show_prompt();
                stream_region
                    .render_into(&mut std::io::stdout(), attach_terminal_width())
                    .map_err(|error| AttachError::Protocol(error.to_string()))?;
                continue;
            }
            match restart_cwd_input(&text, restart_cwd_gate.as_ref()) {
                RestartCwdInput::Resolve(decision) => {
                    command_sequence += 1;
                    let command_id = format!("attach_{}_{}", std::process::id(), command_sequence);
                    let message = json!({
                        "command_id": command_id,
                        "type": "session_restart_cwd_resolve",
                        "session_id": session_id,
                        "decision": decision,
                    });
                    let send_result = match ws.write(Message::Text(message.to_string())) {
                        Ok(()) => ws.flush(),
                        Err(error) => Err(error),
                    };
                    if let Err(error) = send_result {
                        return Err(AttachError::HostUnreachable(error.to_string()));
                    }
                    println!("{}", format_user_echo(&text));
                    stream_region.show_prompt();
                    stream_region
                        .render_into(&mut std::io::stdout(), attach_terminal_width())
                        .map_err(|error| AttachError::Protocol(error.to_string()))?;
                    continue;
                }
                RestartCwdInput::Retry => {
                    close_dots();
                    match restore_restart_cwd_gate(
                        host,
                        &session_id,
                        &mut restart_cwd_gate,
                        &mut stream_region,
                    ) {
                        RestartCwdReload::ChoicesLoaded => {
                            println!("{}", dim_line("Directory choices reloaded."));
                        }
                        RestartCwdReload::NoLongerPending => {
                            println!(
                                "{}",
                                dim_line("The working-directory choice is no longer pending.")
                            );
                        }
                        RestartCwdReload::Unavailable => {
                            println!(
                                "{}",
                                guidance_card(
                                    "Directory choices still unavailable",
                                    "Nothing was sent. The Host's current choices could not be loaded yet.",
                                    "Press Enter to retry, choose `1` to retry, or choose `2` to detach safely.",
                                )
                            );
                        }
                    }
                    stream_region.show_prompt();
                    stream_region
                        .render_into(&mut std::io::stdout(), attach_terminal_width())
                        .map_err(|error| AttachError::Protocol(error.to_string()))?;
                    continue;
                }
                RestartCwdInput::Detach => {
                    close_dots();
                    println!(
                        "{}",
                        guidance_card(
                            "Detached safely",
                            "No directory choice or message was sent. The session remains owned by the Host.",
                            "Run `timem attach` whenever you are ready to load the choices again.",
                        )
                    );
                    return Ok(());
                }
                RestartCwdInput::Invalid => {
                    close_dots();
                    let card = match restart_cwd_gate.as_ref() {
                        Some(RestartCwdGate::Decision(decision)) => {
                            let available = decision
                                .get("session_cwd_available")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            invalid_restart_choice_card(&text, available)
                        }
                        Some(RestartCwdGate::ReloadRequired) => {
                            invalid_restart_recovery_input_card(&text)
                        }
                        None => unreachable!("restart input cannot be invalid without a gate"),
                    };
                    println!("{card}");
                    stream_region.show_prompt();
                    stream_region
                        .render_into(&mut std::io::stdout(), attach_terminal_width())
                        .map_err(|error| AttachError::Protocol(error.to_string()))?;
                    continue;
                }
                RestartCwdInput::NotPending => {}
            }
            command_sequence += 1;
            let command_id = format!("attach_{}_{}", std::process::id(), command_sequence);
            // Numbered choices: render options when a prompt appears and let
            // the user pick by number, so no command memorization is needed.
            let numeric_choice = match text.as_str() {
                "1" | "2" | "3" => Some(text.as_str()),
                _ => None,
            };
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
            let message =
                if let (Some(decision), Some(pending)) = (decision, pending_decisions.last()) {
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
                        stream_region.show_prompt();
                        stream_region
                            .render_into(&mut std::io::stdout(), attach_terminal_width())
                            .map_err(|error| AttachError::Protocol(error.to_string()))?;
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
            stream_region.show_prompt();
            stream_region
                .render_into(&mut std::io::stdout(), attach_terminal_width())
                .map_err(|error| AttachError::Protocol(error.to_string()))?;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                stream_region
                    .clear_into(&mut std::io::stdout())
                    .map_err(|error| AttachError::Protocol(error.to_string()))?;
                if !handle_wire_event(
                    host,
                    &text,
                    session,
                    &mut turn_views,
                    &mut pending_decisions,
                    &mut restart_cwd_gate,
                    &mut stream_region,
                ) {
                    return Ok(());
                }
                stream_region
                    .render_into(&mut std::io::stdout(), attach_terminal_width())
                    .map_err(|error| AttachError::Protocol(error.to_string()))?;
            }
            Ok(Message::Close(_)) => {
                let _ = stream_region.clear_into(&mut std::io::stdout());
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
                    let _ = stream_region.clear_into(&mut std::io::stdout());
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
                    let _ = stream_region.clear_into(&mut std::io::stdout());
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

fn fetch_restart_cwd_decision(host: &HostEndpoint, session_id: &str) -> Result<Option<Value>, ()> {
    let payload =
        http_get_json(host.port, ATTACH_PATH, host.token.as_deref(), 256 * 1024).map_err(|_| ())?;
    let session = payload
        .get("sessions")
        .and_then(Value::as_array)
        .and_then(|sessions| {
            sessions
                .iter()
                .find(|entry| entry.get("session_id").and_then(Value::as_str) == Some(session_id))
        })
        .ok_or(())?;
    Ok(session
        .get("restart_cwd_decision")
        .filter(|decision| !decision.is_null())
        .cloned())
}

#[derive(Debug, PartialEq, Eq)]
enum RestartCwdReload {
    ChoicesLoaded,
    NoLongerPending,
    Unavailable,
}

fn restore_restart_cwd_gate(
    host: &HostEndpoint,
    session_id: &str,
    restart_cwd_gate: &mut Option<RestartCwdGate>,
    stream_region: &mut AttachStreamRegion,
) -> RestartCwdReload {
    match fetch_restart_cwd_decision(host, session_id) {
        Ok(Some(decision)) => {
            set_restart_cwd_gate(
                restart_cwd_gate,
                stream_region,
                Some(RestartCwdGate::Decision(decision)),
            );
            RestartCwdReload::ChoicesLoaded
        }
        Ok(None) => {
            set_restart_cwd_gate(restart_cwd_gate, stream_region, None);
            RestartCwdReload::NoLongerPending
        }
        Err(()) => {
            set_restart_cwd_gate(
                restart_cwd_gate,
                stream_region,
                Some(RestartCwdGate::ReloadRequired),
            );
            RestartCwdReload::Unavailable
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
    restart_cwd_gate: &mut Option<RestartCwdGate>,
    stream_region: &mut AttachStreamRegion,
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
                restart_cwd_gate,
                stream_region,
            )
        }
        "turn_projection" => {
            if value.get("session_id").and_then(Value::as_str) != Some(session_id) {
                return true;
            }
            let projection = value
                .get("projection")
                .and_then(|versioned| versioned.get("projection"));
            let state = projection
                .and_then(|projection| projection.get("state"))
                .and_then(Value::as_str);
            let turn_id = projection
                .and_then(|projection| projection.get("token"))
                .and_then(|token| token.get("turn_id"))
                .and_then(Value::as_str);
            match (state, turn_id) {
                (Some("active"), Some(turn_id)) => {
                    stream_region.set_active_turn(Some(turn_id.to_string()));
                    if stream_region.mode == ShellUiMode::Ordinary {
                        DOTS_OPEN.store(true, std::sync::atomic::Ordering::Relaxed);
                        print!(".");
                        let _ = std::io::stdout().flush();
                    }
                }
                (Some("finished"), Some(turn_id)) => {
                    print_stream_tool_summary(stream_region.finish_turn(turn_id));
                }
                _ => {}
            }
            true
        }
        "turn_finished" => {
            if value.get("session_id").and_then(Value::as_str) == Some(session_id) {
                handle_turn_finished(&value, turn_views, stream_region);
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
                stream_region.apply_preview_topic(&payload);
                register_live_decision_request(&payload, pending_decisions);
                let turn_id = value
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !stream_region.consume_tool_topic(turn_id, &payload) {
                    if let Some(line) = live_topic_summary(&payload) {
                        close_dots();
                        println!("  {line}");
                    }
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
                if error == "session_restart_cwd_decision_required" && restart_cwd_gate.is_none() {
                    let _ =
                        restore_restart_cwd_gate(host, session_id, restart_cwd_gate, stream_region);
                }
                if restart_cwd_gate.is_some() {
                    stream_region.show_prompt();
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
            if error == "session_restart_cwd_decision_required" && restart_cwd_gate.is_none() {
                let _ = restore_restart_cwd_gate(host, session_id, restart_cwd_gate, stream_region);
            }
            if restart_cwd_gate.is_some() {
                stream_region.show_prompt();
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
fn print_stream_tool_summary(summary: String) {
    if summary.is_empty() {
        return;
    }
    close_dots();
    print!("{summary}");
    let _ = std::io::stdout().flush();
}

fn handle_turn_finished(
    value: &Value,
    turn_views: &mut AttachTurnViews,
    stream_region: &mut AttachStreamRegion,
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
    print_stream_tool_summary(stream_region.finish_turn(&turn_id));
    let view = turn_views.view_mut(turn_id);
    view.render_final_answer_once(text);
    close_dots();
    println!(
        "{}",
        dim_line(&format!("──── turn finished · {} ────", local_time_label()))
    );
    stream_region.show_prompt();
}

#[allow(clippy::type_complexity)]
fn handle_wire_event_inner(
    inner: &Value,
    session: &AttachSession,
    turn_views: &mut AttachTurnViews,
    pending_decisions: &mut Vec<PendingDecision>,
    restart_cwd_gate: &mut Option<RestartCwdGate>,
    stream_region: &mut AttachStreamRegion,
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
            let active_turn_id = target.get("active_turn_id").and_then(Value::as_str);
            stream_region.set_active_turn(active_turn_id.map(str::to_string));
            close_dots();
            println!("{}", connected_intro(session, working));
            let gate = target
                .get("restart_cwd_decision")
                .filter(|decision| !decision.is_null())
                .cloned()
                .map(RestartCwdGate::Decision);
            set_restart_cwd_gate(restart_cwd_gate, stream_region, gate);
            if let Some(turns) = target.get("turns").and_then(Value::as_array) {
                if let Some(orientation_turn) =
                    attach_snapshot_orientation_turn(turns, active_turn_id)
                {
                    let turn_id = orientation_turn
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(preview) = orientation_turn.get("preview") {
                        stream_region.apply_preview_payload(preview);
                    }
                    let view = turn_views.view_mut(turn_id);
                    view.render_turn(orientation_turn, pending_decisions, stream_region);
                }
            }
            stream_region.show_prompt();
            true
        }
        "session_restart_cwd_resolved" => {
            if inner
                .get("session")
                .and_then(|s| s.get("session_id"))
                .and_then(Value::as_str)
                == Some(session_id)
                && restart_cwd_gate.is_some()
            {
                set_restart_cwd_gate(restart_cwd_gate, stream_region, None);
                println!("{}", dim_line("Working directory updated."));
                stream_region.show_prompt();
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
            let state = turn
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(state, "pending" | "working" | "restored") {
                stream_region.set_active_turn(Some(turn_id.clone()));
                if let Some(preview) = turn.get("preview") {
                    stream_region.apply_preview_payload(preview);
                }
            } else {
                print_stream_tool_summary(stream_region.finish_turn(&turn_id));
            }
            let view = turn_views.view_mut(turn_id);
            view.render_turn(turn, pending_decisions, stream_region);
            true
        }
        "turn_finished" => {
            if inner.get("session_id").and_then(Value::as_str) == Some(session_id) {
                handle_turn_finished(inner, turn_views, stream_region);
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
                stream_region.apply_preview_topic(&payload);
                register_live_decision_request(&payload, pending_decisions);
                let turn_id = inner
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !stream_region.consume_tool_topic(turn_id, &payload) {
                    if let Some(line) = live_topic_summary(&payload) {
                        close_dots();
                        println!("  {line}");
                    }
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
    use std::sync::atomic::{AtomicU64, Ordering};

    static ATTACH_TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn attach_test_root(label: &str) -> PathBuf {
        let sequence = ATTACH_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "timem-attach-{label}-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_json(path: &Path, value: &impl serde::Serialize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn registry_record(
        registration_id: &str,
        memory_dir: &Path,
        pid: u32,
        started_at_ms: u128,
    ) -> timem_in_process::agent_api::WebInstanceRegistryRecord {
        timem_in_process::agent_api::WebInstanceRegistryRecord {
            registration_id: registration_id.to_string(),
            memory_dir: memory_dir.to_path_buf(),
            pid,
            started_at_ms,
        }
    }

    fn lease(pid: u32, port: Option<u16>, token: &str, started_at_ms: u128) -> Value {
        json!({
            "pid": pid,
            "port": port,
            "token": token,
            "started_at_ms": started_at_ms,
        })
    }

    fn candidate(memory: &str, pid: u32, port: u16) -> AttachHostCandidate {
        AttachHostCandidate {
            memory_dir: PathBuf::from(memory),
            pid,
            started_at_ms: u128::from(pid),
            host: HostEndpoint {
                port,
                token: Some(format!("token-{pid}")),
            },
        }
    }

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
    fn registered_host_route_falls_back_directly_or_selects_by_candidate_count() {
        assert_eq!(
            registered_host_route(Vec::new()),
            RegisteredHostRoute::DefaultMemory
        );
        assert_eq!(
            registered_host_route(vec![candidate("/mem/one", 1, 4101)]),
            RegisteredHostRoute::Direct(HostEndpoint {
                port: 4101,
                token: Some("token-1".to_string()),
            })
        );
        let multiple = vec![candidate("/mem/a", 2, 4102), candidate("/mem/b", 3, 4103)];
        assert_eq!(
            registered_host_route(multiple.clone()),
            RegisteredHostRoute::Select(multiple)
        );
    }

    #[test]
    fn directed_host_discovery_requires_health_even_when_pid_is_live() {
        let root = attach_test_root("directed-health");
        let instance = root.join("web_instance.json");
        write_json(
            &instance,
            &lease(std::process::id(), Some(4201), "lease-token", 10),
        );

        assert!(matches!(
            discover_host_with(&instance, |_| false),
            Err(AttachError::NoHost)
        ));
        let host = discover_host_with(&instance, |host| host.port == 4201).unwrap();
        assert_eq!(host.token.as_deref(), Some("lease-token"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_discovery_revalidates_lease_health_identity_and_deduplicates_memories() {
        let root = attach_test_root("registry");
        let registry = root.join("registry");
        std::fs::create_dir_all(&registry).unwrap();

        let memory_a = root.join("内存-a");
        let memory_b = root.join("memory-b");
        let memory_stale = root.join("memory-stale");
        let memory_no_port = root.join("memory-no-port");
        let memory_unhealthy = root.join("memory-unhealthy");
        for memory in [
            &memory_a,
            &memory_b,
            &memory_stale,
            &memory_no_port,
            &memory_unhealthy,
        ] {
            std::fs::create_dir_all(memory).unwrap();
        }
        write_json(
            &memory_a.join("web_instance.json"),
            &lease(101, Some(4301), "lease-a", 1001),
        );
        write_json(
            &memory_b.join("web_instance.json"),
            &lease(202, Some(4302), "lease-b", 2002),
        );
        write_json(
            &memory_stale.join("web_instance.json"),
            &lease(303, Some(4303), "stale", 3004),
        );
        write_json(
            &memory_no_port.join("web_instance.json"),
            &lease(404, None, "no-port", 4004),
        );
        write_json(
            &memory_unhealthy.join("web_instance.json"),
            &lease(505, Some(4305), "unhealthy", 5005),
        );

        write_json(
            &registry.join("a.json"),
            &registry_record("a", &memory_a, 101, 1001),
        );
        write_json(
            &registry.join("b.json"),
            &registry_record("b", &memory_b, 202, 2002),
        );
        write_json(
            &registry.join("a-alias.json"),
            &registry_record("a-alias", &memory_a.join("."), 101, 1001),
        );
        write_json(
            &registry.join("stale.json"),
            &registry_record("stale", &memory_stale, 303, 3003),
        );
        write_json(
            &registry.join("no-port.json"),
            &registry_record("no-port", &memory_no_port, 404, 4004),
        );
        write_json(
            &registry.join("unhealthy.json"),
            &registry_record("unhealthy", &memory_unhealthy, 505, 5005),
        );
        write_json(
            &registry.join("wrong-name.json"),
            &registry_record("different-id", &memory_a, 101, 1001),
        );
        write_json(
            &registry.join("relative.json"),
            &registry_record("relative", Path::new("relative/mem"), 606, 6006),
        );
        std::fs::write(registry.join("malformed.json"), b"not-json").unwrap();

        let candidates = discover_registered_hosts_with(&registry, |host| host.port != 4305);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.windows(2).all(|pair| {
            pair[0].memory_dir.to_string_lossy() <= pair[1].memory_dir.to_string_lossy()
        }));
        let candidate_a = candidates
            .iter()
            .find(|candidate| candidate.host.port == 4301)
            .expect("healthy memory A candidate");
        assert_eq!(candidate_a.memory_dir, memory_a.canonicalize().unwrap());
        assert_eq!(candidate_a.host.token.as_deref(), Some("lease-a"));
        let candidate_b = candidates
            .iter()
            .find(|candidate| candidate.host.port == 4302)
            .expect("healthy memory B candidate");
        assert_eq!(candidate_b.memory_dir, memory_b.canonicalize().unwrap());
        assert_eq!(candidate_b.host.token.as_deref(), Some("lease-b"));

        assert!(registry.join("malformed.json").exists());
        assert!(registry.join("stale.json").exists());
        assert!(registry.join("unhealthy.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_registry_is_an_empty_candidate_set() {
        let root = attach_test_root("missing-registry");
        let missing = root.join("missing");
        assert!(discover_registered_hosts_with(&missing, |_| true).is_empty());
        std::fs::remove_dir_all(root).unwrap();
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

    fn preview_payload(turn_id: &str, revision: u64, text: Option<&str>, status: &str) -> Value {
        json!({
            "turn_id": turn_id,
            "attempt": 1,
            "revision": revision,
            "interruption": null,
            "response": text.map(|text| json!({
                "attempt": 1,
                "revision": revision,
                "text": text,
                "status": status,
            })),
        })
    }

    #[test]
    fn attach_snapshot_orientation_prefers_the_active_turn_over_array_order() {
        let turns = vec![
            json!({"turn_id": "turn-active", "preview": preview_payload("turn-active", 1, Some("active preview"), "streaming")}),
            json!({"turn_id": "turn-history", "preview": preview_payload("turn-history", 9, Some("history preview"), "final")}),
        ];

        assert_eq!(
            attach_snapshot_orientation_turn(&turns, Some("turn-active"))
                .and_then(|turn| turn.get("turn_id"))
                .and_then(Value::as_str),
            Some("turn-active")
        );
        assert!(
            attach_snapshot_orientation_turn(&turns, Some("turn-missing")).is_none(),
            "a missing active turn must not fall back to unrelated history"
        );
        assert_eq!(
            attach_snapshot_orientation_turn(&turns, None)
                .and_then(|turn| turn.get("turn_id"))
                .and_then(Value::as_str),
            Some("turn-history"),
            "without an active turn the latest historical turn provides orientation"
        );
    }

    #[test]
    fn attach_input_clear_rows_include_echo_newline_and_only_added_wrapping() {
        let prompt = "attach> ";
        assert_eq!(attach_input_clear_rows(3, prompt, "", 80), 3);
        assert_eq!(attach_input_clear_rows(3, prompt, "x", 80), 3);
        assert_eq!(attach_input_clear_rows(3, prompt, "123456", 10), 4);
        assert_eq!(attach_input_clear_rows(3, prompt, "你好你好", 10), 4);
    }

    #[test]
    fn attach_stream_region_filters_turns_and_uses_one_snapshot_live_reducer() {
        let mut region =
            AttachStreamRegion::new(ShellUiMode::Stream, Some("turn-active".to_string()));
        region.prompt_visible = true;

        assert_eq!(
            region.apply_preview_payload(&preview_payload(
                "turn-old",
                99,
                Some("wrong turn"),
                "streaming",
            )),
            StreamPreviewUpdate::Ignored
        );
        assert!(!region.content().contains("wrong turn"));

        assert_eq!(
            region.apply_preview_payload(&preview_payload(
                "turn-active",
                2,
                Some("snapshot text"),
                "streaming",
            )),
            StreamPreviewUpdate::Changed
        );
        assert!(region.content().contains("snapshot text"));

        let live = json!({
            "topic": {"name": "core.model.preview"},
            "payload": preview_payload("turn-active", 3, Some("live replacement"), "intermediate"),
        });
        assert_eq!(
            region.apply_preview_topic(&live),
            StreamPreviewUpdate::Changed
        );
        assert!(region.content().contains("live replacement"));
        assert!(!region.content().contains("snapshot text"));

        assert_eq!(
            region.apply_preview_payload(&preview_payload(
                "turn-active",
                2,
                Some("stale"),
                "streaming",
            )),
            StreamPreviewUpdate::Ignored
        );
        assert!(!region.content().contains("stale"));

        assert_eq!(
            region.apply_preview_payload(&preview_payload("turn-active", 4, None, "streaming",)),
            StreamPreviewUpdate::Retracted
        );
        assert!(!region.content().contains("live replacement"));
    }

    fn attach_tool_topic(action_id: &str, event: &str, status: &str, active: bool) -> Value {
        json!({
            "topic": {"name": "core.action"},
            "payload": {
                "action": "readfile",
                "action_id": action_id,
                "input": {"path": "src/main.rs"},
                "event": event,
                "status": status,
                "active": active,
            }
        })
    }

    #[test]
    fn attach_stream_tools_filter_active_turn_and_fold_snapshot_live_lifecycle_once() {
        let mut region =
            AttachStreamRegion::new(ShellUiMode::Stream, Some("turn-active".to_string()));
        let running = attach_tool_topic("tool-1", "execution_start", "running", true);
        assert!(!region.consume_tool_topic("turn-old", &running));
        assert!(region.tools.is_empty());
        assert!(region.consume_tool_topic("turn-active", &running));
        assert!(region.content().contains("Read file"));
        assert!(region.content().contains("src/main.rs"));

        let snapshot_duplicate = json!({
            "event_id": "event-running",
            "source": "core_topic",
            "payload": running,
        });
        assert!(region.consume_snapshot_tool_event("turn-active", &snapshot_duplicate));
        assert_eq!(region.tools.active_count(), 1);

        let completed = attach_tool_topic("tool-1", "finish", "completed", false);
        assert!(region.consume_tool_topic("turn-active", &completed));
        assert!(region.consume_tool_topic("turn-active", &completed));
        assert_eq!(region.tools.completed_counts(), (1, 0));
        assert!(!region.content().contains("Read file"));
        assert!(region.content().contains("Tools folded"));
        assert!(region.content().contains("✓ 1"));
    }

    #[test]
    fn attach_stream_turn_switch_and_finish_clear_tool_projection_exactly_once() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, Some("turn-1".to_string()));
        assert!(region.consume_tool_topic(
            "turn-1",
            &attach_tool_topic("tool-1", "finish", "timeout", false),
        ));
        let summary = region.finish_turn("turn-1");
        assert!(summary.contains("Tools folded"));
        assert!(summary.contains("× 1"));
        assert!(region.finish_turn("turn-1").is_empty());
        assert!(region.tools.is_empty());

        region.set_active_turn(Some("turn-2".to_string()));
        assert!(region.consume_tool_topic(
            "turn-2",
            &attach_tool_topic("tool-2", "execution_start", "running", true),
        ));
        region.set_active_turn(Some("turn-3".to_string()));
        assert!(region.tools.is_empty());
        assert!(!region.content().contains("Read file"));
    }

    #[test]
    fn ordinary_attach_never_consumes_structured_tool_topics() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Ordinary, Some("turn-1".to_string()));
        assert!(!region.consume_tool_topic(
            "turn-1",
            &attach_tool_topic("tool-1", "execution_start", "running", true),
        ));
        assert!(region.tools.is_empty());
        assert!(region.finish_turn("turn-1").is_empty());
    }

    #[test]
    fn final_preview_remains_provisional_until_authoritative_turn_finish() {
        let mut region =
            AttachStreamRegion::new(ShellUiMode::Stream, Some("turn-active".to_string()));
        region.apply_preview_payload(&preview_payload(
            "turn-active",
            7,
            Some("provisional final"),
            "final",
        ));

        assert_eq!(region.active_turn_id.as_deref(), Some("turn-active"));
        assert!(region.content().contains("provisional final"));

        region.finish_turn("turn-other");
        assert!(region.content().contains("provisional final"));
        region.finish_turn("turn-active");
        assert!(region.active_turn_id.is_none());
        assert!(region.preview.snapshot().is_none());
        assert!(!region.content().contains("provisional final"));
    }

    #[test]
    fn changing_active_turn_clears_prior_preview_even_when_revision_was_higher() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, Some("turn-old".to_string()));
        region.apply_preview_payload(&preview_payload(
            "turn-old",
            90,
            Some("old preview"),
            "streaming",
        ));
        region.set_active_turn(Some("turn-new".to_string()));
        assert!(region.preview.snapshot().is_none());
        assert_eq!(
            region.apply_preview_payload(&preview_payload(
                "turn-new",
                1,
                Some("new preview"),
                "streaming",
            )),
            StreamPreviewUpdate::Changed
        );
        assert!(region.content().contains("new preview"));
        assert!(!region.content().contains("old preview"));
    }

    #[test]
    fn ordinary_attach_stream_region_writes_no_terminal_control_bytes() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Ordinary, Some("turn-1".to_string()));
        region.rendered_rows = 4;
        region.prompt_visible = true;
        assert_eq!(
            region.apply_preview_payload(&preview_payload(
                "turn-1",
                1,
                Some("ignored"),
                "streaming",
            )),
            StreamPreviewUpdate::Ignored
        );
        let mut output = Vec::new();
        region.clear_into(&mut output).unwrap();
        region
            .clear_for_input_into(&mut output, "wrapped input", 8)
            .unwrap();
        region.render_into(&mut output, 8).unwrap();
        assert!(output.is_empty());
        assert!(region.content().is_empty());
    }

    #[test]
    fn attach_stream_region_counts_ansi_cjk_and_clears_wrapped_input_rows() {
        assert_eq!(attach_wrapped_rows("\x1b[31m12345\x1b[0m\n你好", 4), 3);

        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, Some("turn-1".to_string()));
        region.prompt_visible = true;
        region.apply_preview_payload(&preview_payload(
            "turn-1",
            1,
            Some("第一行 preview\nsecond line"),
            "streaming",
        ));
        let expected_rows = region.rendered_row_count(20);
        assert!(expected_rows >= 3);

        let mut output = Vec::new();
        region.render_into(&mut output, 20).unwrap();
        assert_eq!(region.rendered_rows, expected_rows);
        let rendered_len = output.len();
        region
            .clear_for_input_into(&mut output, "很长的用户输入 wrapped user input", 20)
            .unwrap();
        let clear_bytes = &output[rendered_len..];
        let prompt = attach_prompt_text();
        let expected_up = attach_input_clear_rows(
            expected_rows,
            &prompt,
            "很长的用户输入 wrapped user input",
            20,
        );
        assert_eq!(
            clear_bytes,
            format!("\x1b[1G\x1b[{expected_up}A\x1b[J").as_bytes(),
            "clear sequence must return from the echoed input line to the first dynamic row"
        );
        assert_eq!(region.rendered_rows, 0);
        assert!(!region.prompt_visible);
    }

    #[test]
    fn blank_attach_input_clears_echoed_line_and_restores_stream_prompt() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, None);
        region.show_prompt();
        let mut output = Vec::new();
        region.render_into(&mut output, 120).unwrap();
        assert_eq!(region.rendered_rows, 1);
        let rendered_len = output.len();

        region.clear_for_input_into(&mut output, "", 120).unwrap();
        assert_eq!(
            &output[rendered_len..],
            b"\x1b[1G\x1b[1A\x1b[J",
            "blank Enter leaves the cursor one row below the prompt"
        );
        region.show_prompt();
        let before_rerender = output.len();
        region.render_into(&mut output, 120).unwrap();
        let rerendered = String::from_utf8_lossy(&output[before_rerender..]);
        assert!(rerendered.contains("attach"));
        assert!(rerendered.contains("❯❯"));
        assert_eq!(region.rendered_rows, 1);
        assert!(region.prompt_visible);
    }

    fn restart_cwd_fixture(available: bool) -> Value {
        json!({
            "runtime_cwd": "/host/work",
            "session_cwd": "/session/work",
            "session_cwd_available": available,
        })
    }

    #[test]
    fn restart_cwd_gate_replaces_stream_prompt_until_host_resolution() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, None);
        region.show_prompt();
        assert!(region.content().contains("attach"));

        region.set_restart_cwd_gate(Some(RestartCwdGate::Decision(restart_cwd_fixture(false))));
        region.show_prompt();
        let gated = region.content();
        assert!(gated.contains("Choose the working directory"));
        assert!(gated.contains("/session/work"));
        assert!(gated.contains("/host/work"));
        assert!(gated.contains("(unavailable)"));
        assert!(gated.contains("choice ❯❯"));
        assert!(!gated.contains("attach ❯❯"));

        region.set_restart_cwd_gate(None);
        region.show_prompt();
        assert!(region.content().contains("attach"));
        assert!(!region.content().contains("Choose the working directory"));
    }

    #[test]
    fn restart_cwd_gate_clear_rows_use_the_rendered_choice_card() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, None);
        region.set_restart_cwd_gate(Some(RestartCwdGate::Decision(restart_cwd_fixture(true))));
        region.show_prompt();
        let prompt = region.visible_prompt_text();
        let expected_rows = region.rendered_row_count(24);
        let mut output = Vec::new();
        region.render_into(&mut output, 24).unwrap();
        let rendered_len = output.len();
        region.clear_for_input_into(&mut output, "2", 24).unwrap();
        let expected_up = attach_input_clear_rows(expected_rows, &prompt, "2", 24);
        assert_eq!(
            &output[rendered_len..],
            format!("\x1b[1G\x1b[{expected_up}A\x1b[J").as_bytes()
        );
    }

    #[test]
    fn hello_gate_persists_and_resolved_event_restores_stream_prompt() {
        let session = AttachSession {
            session_id: "session-1".to_string(),
            display_name: "Session".to_string(),
            ordinal: 1,
            state: "ready".to_string(),
            working: false,
            active_turn_id: None,
            current_dir: "/session/work".to_string(),
            restart_cwd_decision: None,
            worker_count: 0,
        };
        let mut views = AttachTurnViews::default();
        let mut pending = Vec::new();
        let mut gate = None;
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, None);
        let hello = json!({
            "type": "hello",
            "snapshot": {"sessions": [{
                "session_id": "session-1",
                "state": "ready",
                "active_turn_id": null,
                "restart_cwd_decision": restart_cwd_fixture(true),
                "turns": [],
            }]},
        });
        assert!(handle_wire_event_inner(
            &hello,
            &session,
            &mut views,
            &mut pending,
            &mut gate,
            &mut region,
        ));
        assert!(gate.is_some());
        assert!(region.content().contains("choice ❯❯"));
        assert!(!region.content().contains("attach ❯❯"));

        let resolved = json!({
            "type": "session_restart_cwd_resolved",
            "session": {"session_id": "session-1"},
        });
        assert!(handle_wire_event_inner(
            &resolved,
            &session,
            &mut views,
            &mut pending,
            &mut gate,
            &mut region,
        ));
        assert!(gate.is_none());
        assert!(region.content().contains("attach"));
        assert!(!region.content().contains("choice ❯❯"));
    }

    #[test]
    fn restart_cwd_recovery_gate_is_actionable_and_blocks_unrelated_input() {
        let recovery = RestartCwdGate::ReloadRequired;
        assert_eq!(
            restart_cwd_input("", Some(&recovery)),
            RestartCwdInput::Retry
        );
        assert_eq!(
            restart_cwd_input("1", Some(&recovery)),
            RestartCwdInput::Retry
        );
        assert_eq!(
            restart_cwd_input("!retry", Some(&recovery)),
            RestartCwdInput::Retry
        );
        assert_eq!(
            restart_cwd_input("2", Some(&recovery)),
            RestartCwdInput::Detach
        );
        assert_eq!(
            restart_cwd_input("!q", Some(&recovery)),
            RestartCwdInput::Detach
        );
        assert_eq!(
            restart_cwd_input("send this message", Some(&recovery)),
            RestartCwdInput::Invalid
        );
        assert_eq!(
            restart_cwd_input("!c", Some(&recovery)),
            RestartCwdInput::Invalid
        );
    }

    #[test]
    fn restart_cwd_recovery_prompt_switches_back_to_authoritative_choices() {
        let mut region = AttachStreamRegion::new(ShellUiMode::Stream, None);
        region.set_restart_cwd_gate(Some(RestartCwdGate::ReloadRequired));
        region.show_prompt();
        let recovery = region.content();
        let recovery_plain = attach_strip_ansi(&recovery);
        assert!(recovery_plain.contains("Directory choices unavailable"));
        assert!(recovery_plain.contains("1  Retry loading choices"));
        assert!(recovery_plain.contains("2  Detach for now"));
        assert!(recovery_plain.contains("recovery ❯❯"));
        assert!(!recovery_plain.contains("attach ❯❯"));

        region.set_restart_cwd_gate(Some(RestartCwdGate::Decision(restart_cwd_fixture(true))));
        let choices = region.content();
        let choices_plain = attach_strip_ansi(&choices);
        assert!(choices_plain.contains("Choose the working directory"));
        assert!(choices_plain.contains("choice ❯❯"));
        assert!(!choices_plain.contains("Directory choices unavailable"));
        assert!(!choices_plain.contains("attach ❯❯"));
    }

    #[test]
    fn restart_cwd_gate_accepts_only_available_authoritative_choices() {
        let available = json!({"session_cwd_available": true});
        assert_eq!(
            restart_cwd_input("1", Some(&RestartCwdGate::Decision(available.clone()))),
            RestartCwdInput::Resolve("keep_session")
        );
        assert_eq!(
            restart_cwd_input("!k", Some(&RestartCwdGate::Decision(available.clone()))),
            RestartCwdInput::Resolve("keep_session")
        );
        assert_eq!(
            restart_cwd_input("2", Some(&RestartCwdGate::Decision(available.clone()))),
            RestartCwdInput::Resolve("use_runtime")
        );
        assert_eq!(
            restart_cwd_input("!r", Some(&RestartCwdGate::Decision(available.clone()))),
            RestartCwdInput::Resolve("use_runtime")
        );
        assert_eq!(
            restart_cwd_input(
                "send this",
                Some(&RestartCwdGate::Decision(available.clone()))
            ),
            RestartCwdInput::Invalid
        );
        assert_eq!(
            restart_cwd_input("3", Some(&RestartCwdGate::Decision(available.clone()))),
            RestartCwdInput::Invalid
        );
        assert_eq!(restart_cwd_input("1", None), RestartCwdInput::NotPending);

        let unavailable = json!({"session_cwd_available": false});
        assert_eq!(
            restart_cwd_input("1", Some(&RestartCwdGate::Decision(unavailable.clone()))),
            RestartCwdInput::Invalid
        );
        assert_eq!(
            restart_cwd_input("!k", Some(&RestartCwdGate::Decision(unavailable.clone()))),
            RestartCwdInput::Invalid
        );
        assert_eq!(
            restart_cwd_input("2", Some(&RestartCwdGate::Decision(unavailable.clone()))),
            RestartCwdInput::Resolve("use_runtime")
        );
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

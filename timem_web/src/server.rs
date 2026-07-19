use agent_core::session_store::{
    ChatHistoryEventKind, ChatHistoryRecord, ChatHistoryRole, SessionResumeNotice, SessionStore,
    StoredSession, StoredSessionProfile, StoredSessionState,
};
use agent_core::{
    apply_runtime_config_value, combine_additional_contexts, default_data_root,
    load_workspace_dirs_from_path, provider_config_from_sources, runtime_config_menu_report,
    runtime_info_context, validate_provider_api_key, work_instruction_load_report,
    work_instruction_load_request, work_instruction_mode_from_sources, AgentCore, BashApprovalMode,
    CoreSessionWorkerEvent, CoreSessionWorkerManager, CoreSessionWorkerWorkspace, HostDecision,
    HostDecisionRequest, ProviderConfig, ProviderConfigSource, ResponseProtocolKind,
    RuntimeDataLayout, SessionToolRepo, ToolDetail, ToolGenRequest, ToolSummary, TopicReply,
    WorkInstructionLoadMode, CORE_TOPIC_TOOLGEN, CORE_TOPIC_WORK_INSTRUCTION_LOAD,
};
use agent_core::{capability::CapabilityRegistry, self_tool::SelfToolPaths};
use axum::{
    extract::DefaultBodyLimit,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, Query, State,
    },
    http::{header, HeaderName, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsString,
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, sync::broadcast, time::sleep};

include!(concat!(env!("OUT_DIR"), "/embedded_web_assets.rs"));

const STATIC_PROMPT: &str = include_str!("../../resources/system_prompt/system_prompt.md");
const PORT_START: u16 = 12_345;
const PORT_END: u16 = 23_456;
const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(40);
const EVENT_CHANNEL_CAPACITY: usize = 256;
const SESSION_HISTORY_PAGE_LIMIT: usize = 200;
const MAX_SESSION_MESSAGES: usize = 2_000;
const MAX_SESSION_TURNS: usize = 200;
const MAX_TURN_EVENTS: usize = 500;
const MAX_TURN_USER_ENTRIES: usize = 200;
const MAX_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_SESSION_UPLOADS: usize = 20;
const MAX_BROWSER_COMMAND_BYTES: usize = 1024 * 1024;
const WORK_INSTRUCTION_DECISION_TIMEOUT: Duration = Duration::from_secs(30);
static NEXT_WEB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct AppState {
    token: String,
    manager: Arc<Mutex<CoreSessionWorkerManager>>,
    template: Arc<WorkerTemplate>,
    mem: Arc<Mutex<WebMemState>>,
    events: broadcast::Sender<WireEvent>,
    sessions: Arc<Mutex<BTreeMap<String, WebSession>>>,
}

#[derive(Clone)]
struct WorkerTemplate {
    settings: Arc<Mutex<RuntimeSettings>>,
    data_dir: PathBuf,
    initial_space: String,
    env: BTreeMap<String, String>,
    current_dir: PathBuf,
    workspace_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct WebMemState {
    space: String,
    layout: RuntimeDataLayout,
    session_store: SessionStore,
}

impl WebMemState {
    fn new(data_dir: PathBuf, space: String) -> Result<Self, String> {
        validate_web_space_name(&space)?;
        let layout = RuntimeDataLayout::new(data_dir, space.clone());
        Ok(Self {
            space,
            session_store: SessionStore::new(layout.memory_dir()),
            layout,
        })
    }

    fn info(&self) -> WebMemInfo {
        WebMemInfo {
            space: self.space.clone(),
            data_dir: self.layout.data_root().display().to_string(),
            space_dir: self.layout.space_dir().display().to_string(),
            memory_dir: self.layout.memory_dir().display().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeSettings {
    config: ProviderConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
}

#[derive(Debug, Clone, Serialize)]
struct WebSession {
    session_id: String,
    display_name: String,
    ordinal: u32,
    state: String,
    current_dir: String,
    max_llm_input_tokens: u32,
    tools: Vec<ToolSummary>,
    runtime_profile: WebSessionRuntimeProfile,
    contexts: Vec<WebContext>,
    workers: Vec<WebWorker>,
    active_context_id: String,
    primary_worker_id: String,
    attachments: Vec<WebAttachment>,
    #[serde(skip)]
    consumed_attachment_ids: BTreeSet<String>,
    messages: Vec<WebChatMessage>,
    turns: Vec<WebTurn>,
    history_before_cursor: Option<String>,
    history_has_more: bool,
    #[serde(skip)]
    resume_notice_pending: bool,
    active_turn_id: Option<String>,
    #[serde(skip)]
    pending_completion_message_id: Option<String>,
    #[serde(skip)]
    work_instruction_mode: WorkInstructionLoadMode,
    #[serde(skip)]
    work_instruction_allowed: Option<bool>,
    #[serde(skip)]
    pending_work_instruction_turn: Option<PendingWorkInstructionTurn>,
    #[serde(skip)]
    runtime: WebSessionRuntime,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct WebContext {
    context_id: String,
    current_dir: String,
    worker_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct WebWorker {
    worker_id: String,
    context_id: String,
    display_name: String,
    ordinal: u32,
    state: String,
    parent_worker_id: Option<String>,
}

#[derive(Debug, Clone)]
struct WebSessionRuntime {
    settings: RuntimeSettings,
    env: BTreeMap<String, String>,
    env_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct WebSessionRuntimeProfile {
    provider: String,
    model: String,
    api_protocol: String,
    response_protocol: String,
    base_url: String,
    timeout_secs: u64,
    max_llm_input_tokens: u32,
    max_llm_output_tokens: u32,
    bash_approval: String,
    work_instructions: String,
}

#[derive(Debug, Clone, Serialize)]
struct WebTurn {
    turn_id: String,
    state: String,
    created_at_ms: u128,
    user_entries: Vec<WebTurnUserEntry>,
    events: Vec<WebTurnEvent>,
    final_answer: Option<String>,
    completion: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct WebTurnUserEntry {
    kind: String,
    text: String,
    attachments: Vec<WebAttachment>,
    created_at_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
struct WebTurnEvent {
    event_id: String,
    source: String,
    payload: Value,
    created_at_ms: u128,
}

#[derive(Debug, Clone)]
struct PendingWorkInstructionTurn {
    request_id: String,
    text: String,
    attachments: Vec<WebAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct WebAttachment {
    id: String,
    name: String,
    path: String,
    bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
struct WebChatMessage {
    id: String,
    role: String,
    text: String,
    created_at_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireEvent {
    Hello {
        snapshot: WebSnapshot,
    },
    SessionCreated {
        session: WebSession,
    },
    SessionRenamed {
        session_id: String,
        display_name: String,
    },
    CoreTopic {
        turn_id: Option<String>,
        turn_event_id: Option<String>,
        event: Value,
    },
    WorkerActivity {
        session_id: String,
        context_id: String,
        worker_id: String,
        turn_id: Option<String>,
        turn_event_id: Option<String>,
        event: Value,
    },
    TurnFinished {
        session_id: String,
        turn_id: Option<String>,
        outcome: Value,
    },
    TurnUpdated {
        session_id: String,
        turn: WebTurn,
    },
    HostError {
        message: String,
    },
    HostConfigUpdated {
        key: String,
        value: String,
        session_env_defaults: BTreeMap<String, String>,
    },
    FileUploaded {
        session_id: String,
        file: WebAttachment,
    },
    AttachmentRemoved {
        session_id: String,
        attachment_id: String,
    },
    HistoryPage {
        session_id: String,
        records: Vec<ChatHistoryRecord>,
        before_cursor: Option<String>,
        has_more: bool,
    },
    ToolRepoUpdated {
        session_id: String,
        tools: Vec<ToolSummary>,
    },
    ToolRepoSearchResult {
        session_id: String,
        query: String,
        tools: Vec<ToolSummary>,
    },
    ToolRepoDetail {
        session_id: String,
        detail: ToolDetail,
    },
}

#[derive(Debug, Clone, Serialize)]
struct WebSnapshot {
    server: ServerInfo,
    sessions: Vec<WebSession>,
}

#[derive(Debug, Clone, Serialize)]
struct ServerInfo {
    version: String,
    protocol_version: u8,
    port: u16,
    mem: WebMemInfo,
    runtime_options: Vec<WebRuntimeOption>,
    session_env_defaults: BTreeMap<String, String>,
    workspace_dirs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WebMemInfo {
    space: String,
    data_dir: String,
    space_dir: String,
    memory_dir: String,
}

#[derive(Debug, Clone, Serialize)]
struct WebRuntimeOption {
    key: String,
    value: String,
    applies_to: &'static str,
}

#[derive(Debug, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UploadQuery {
    token: Option<String>,
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientCommand {
    SessionCreate {
        display_name: Option<String>,
        workspace_dir: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    SessionRename {
        session_id: String,
        display_name: String,
    },
    SessionStop {
        session_id: String,
    },
    TurnSubmit {
        session_id: String,
        #[serde(default)]
        text: String,
        input_kind: Option<String>,
        source_turn_id: Option<String>,
    },
    TurnSupplement {
        session_id: String,
        text: String,
    },
    TurnCancel {
        session_id: String,
    },
    AttachmentRemove {
        session_id: String,
        attachment_id: String,
    },
    HistoryPage {
        session_id: String,
        before_cursor: Option<String>,
        limit: Option<usize>,
    },
    ToolRepoSearch {
        session_id: String,
        query: String,
        limit: Option<usize>,
    },
    ToolRepoDetail {
        session_id: String,
        tool_id: String,
    },
    ToolRepoRename {
        session_id: String,
        tool_id: String,
        new_name: String,
    },
    ToolRepoOpenTerminal {
        session_id: String,
        tool_id: String,
    },
    TopicReply {
        session_id: String,
        worker_id: Option<String>,
        topic_name: String,
        request_id: Option<String>,
        decision: String,
        #[serde(default)]
        payload: Value,
    },
    RuntimeUpdate {
        key: String,
        value: String,
    },
    MemSwitch {
        space: String,
    },
}

pub async fn run_from_env() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }

    let launch = WebLaunchOptions::parse(&args)?;
    let template = WorkerTemplate::from_environment(&launch)?;
    let token = generate_token()?;
    let manager = Arc::new(Mutex::new(CoreSessionWorkerManager::new()));
    let sessions = Arc::new(Mutex::new(BTreeMap::new()));
    let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
    let mem = Arc::new(Mutex::new(WebMemState::new(
        template.data_dir.clone(),
        template.initial_space.clone(),
    )?));
    let state = AppState {
        token: token.clone(),
        manager,
        template: Arc::new(template),
        mem,
        events,
        sessions,
    };

    if restore_stored_sessions(&state)? == 0 {
        let default_session = create_session(&state, None, None, BTreeMap::new())?;
        let _ = default_session;
    }
    spawn_event_bridge(state.clone());

    let listener = bind_loopback(launch.port).await?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let app = build_router(state.clone(), port);
    let url = format!("http://127.0.0.1:{port}/?token={token}");
    println!("Timem Web is ready at {url}");
    if launch.open_browser {
        if let Err(error) = open_browser(&url) {
            eprintln!("Could not open the browser automatically: {error}");
            eprintln!("Open this URL manually: {url}");
        }
    }
    println!("The server is bound to 127.0.0.1 only. Press Ctrl+C to stop.");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn build_router(state: AppState, port: u16) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/snapshot", get(snapshot))
        .route("/api/upload", post(upload_file))
        .route("/ws", get(websocket))
        .fallback(get(static_asset))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 64 * 1024))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                apply_browser_security_headers(&mut response);
                response
            },
        ))
        .with_state((state, port))
}

fn apply_browser_security_headers(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    response.headers_mut().insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
}

async fn upload_file(
    State((state, _)): State<(AppState, u16)>,
    Query(query): Query<UploadQuery>,
    mut multipart: Multipart,
) -> Response {
    if query.token.as_deref() != Some(state.token.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let result = async {
        let field = multipart
            .next_field()
            .await
            .map_err(|_| "invalid_upload_multipart".to_string())?
            .ok_or_else(|| "upload_file_required".to_string())?;
        if field.name() != Some("file") {
            return Err("upload_file_required".to_string());
        }
        let name = sanitize_upload_name(field.file_name().unwrap_or("upload"))?;
        let bytes = field
            .bytes()
            .await
            .map_err(|_| "upload_read_failed".to_string())?;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err("upload_too_large".to_string());
        }
        let attachment = store_upload(&state, &query.session_id, name, bytes.as_ref()).await?;
        let _ = state.events.send(WireEvent::FileUploaded {
            session_id: query.session_id,
            file: attachment.clone(),
        });
        Ok::<_, String>(attachment)
    }
    .await;
    match result {
        Ok(file) => Json(file).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

async fn static_asset(uri: Uri) -> Response {
    let path = match uri.path() {
        "/" => "/index.html",
        path => path,
    };
    let (asset_path, content_type) = match embedded_web_asset(path) {
        Some(_) => (path, mime_for_path(path)),
        None => ("/index.html", "text/html; charset=utf-8"),
    };
    let body = embedded_web_asset(asset_path).expect("embedded index asset must exist");
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
        .into_response()
}

fn mime_for_path(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else {
        "application/octet-stream"
    }
}

async fn health(
    State((state, port)): State<(AppState, u16)>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !authorized(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "ok": true, "port": port })).into_response()
}

async fn snapshot(
    State((state, port)): State<(AppState, u16)>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !authorized(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(snapshot_for(&state, port)).into_response()
}

async fn websocket(
    ws: WebSocketUpgrade,
    State((state, port)): State<(AppState, u16)>,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !authorized(&state, &auth) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| websocket_session(socket, state, port))
}

fn authorized(state: &AppState, auth: &AuthQuery) -> bool {
    auth.token.as_deref() == Some(state.token.as_str())
}

fn current_mem_state(state: &AppState) -> Result<WebMemState, String> {
    state
        .mem
        .lock()
        .map(|mem| mem.clone())
        .map_err(|_| "mem_state_poisoned".to_string())
}

fn current_session_store(state: &AppState) -> Result<SessionStore, String> {
    Ok(current_mem_state(state)?.session_store)
}

fn session_tool_repo(state: &AppState, session_id: &str) -> Result<SessionToolRepo, String> {
    let mem = current_mem_state(state)?;
    Ok(SessionToolRepo::new(mem.layout.memory_dir(), session_id))
}

async fn websocket_session(socket: WebSocket, state: AppState, port: u16) {
    let (mut sender, mut receiver) = socket.split();
    if send_event(
        &mut sender,
        &WireEvent::Hello {
            snapshot: snapshot_for(&state, port),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    for event in work_instruction_notice_events(&state) {
        if send_event(&mut sender, &event).await.is_err() {
            return;
        }
    }
    let mut events = state.events.subscribe();
    loop {
        tokio::select! {
            maybe_command = receiver.next() => {
                match maybe_command {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > MAX_BROWSER_COMMAND_BYTES {
                            if send_event(&mut sender, &WireEvent::HostError { message: "browser_command_too_large".to_string() }).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        match serde_json::from_str::<ClientCommand>(&text) {
                            Ok(command) => {
                                match handle_command(&state, port, command) {
                                    Ok(Some(event)) => if send_event(&mut sender, &event).await.is_err() { break; },
                                    Ok(None) => {}
                                    Err(error) => if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() {
                                        break;
                                    },
                                }
                            }
                            Err(error) => {
                                if send_event(&mut sender, &WireEvent::HostError { message: format!("invalid_browser_command:{error}") }).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = events.recv() => match event {
                Ok(event) => if send_event(&mut sender, &event).await.is_err() { break; },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if send_event(&mut sender, &WireEvent::Hello { snapshot: snapshot_for(&state, port) }).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

async fn send_event(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &WireEvent,
) -> Result<(), ()> {
    let text = serde_json::to_string(event).map_err(|_| ())?;
    sender.send(Message::Text(text)).await.map_err(|_| ())
}

fn handle_command(
    state: &AppState,
    port: u16,
    command: ClientCommand,
) -> Result<Option<WireEvent>, String> {
    match command {
        ClientCommand::SessionCreate {
            display_name,
            workspace_dir,
            env,
        } => {
            let session_id = create_session(state, display_name, workspace_dir, env)?;
            let session = state
                .sessions
                .lock()
                .map_err(|_| "session_store_poisoned")?
                .get(&session_id)
                .cloned()
                .ok_or_else(|| "created_session_not_found".to_string())?;
            if let Some(event) = work_instruction_notice_event(state, &session_id) {
                let _ = state.events.send(event);
            }
            return Ok(Some(WireEvent::SessionCreated { session }));
        }
        ClientCommand::SessionRename {
            session_id,
            display_name,
        } => {
            let handle = primary_worker_handle(state, &session_id)?;
            let display_name = nonempty_text(display_name, "session display name")?;
            handle.rename(display_name.clone())?;
            state
                .sessions
                .lock()
                .map_err(|_| "session_store_poisoned")?
                .get_mut(&session_id)
                .ok_or_else(|| "session_not_found".to_string())?
                .display_name = display_name.clone();
            persist_web_session(state, &session_id)?;
            return Ok(Some(WireEvent::SessionRenamed {
                session_id,
                display_name,
            }));
        }
        ClientCommand::SessionStop { session_id } => {
            let worker_ids = session_worker_ids(state, &session_id)?;
            let mut manager = state
                .manager
                .lock()
                .map_err(|_| "worker_manager_poisoned")?;
            for worker_id in worker_ids {
                manager.request_shutdown(&worker_id)?;
            }
        }
        ClientCommand::TurnSubmit {
            session_id,
            text,
            input_kind,
            source_turn_id,
        } => {
            let turn = if input_kind.as_deref() == Some("toolgen") {
                submit_toolgen_turn(
                    state,
                    &session_id,
                    source_turn_id
                        .as_deref()
                        .ok_or_else(|| "toolgen_source_turn_id_required".to_string())?,
                    (!text.trim().is_empty()).then_some(text),
                )?
            } else {
                if input_kind.is_some() || source_turn_id.is_some() {
                    return Err("unsupported_turn_input_kind".to_string());
                }
                let text = nonempty_text(text, "turn text")?;
                submit_or_supplement_turn(state, &session_id, text)?
            };
            return Ok(Some(WireEvent::TurnUpdated { session_id, turn }));
        }
        ClientCommand::TurnSupplement { session_id, text } => {
            let text = nonempty_text(text, "supplement")?;
            let turn = append_supplement_or_submit_turn(state, &session_id, text)?;
            return Ok(Some(WireEvent::TurnUpdated { session_id, turn }));
        }
        ClientCommand::TurnCancel { session_id } => {
            for worker_id in session_worker_ids(state, &session_id)? {
                state
                    .manager
                    .lock()
                    .map_err(|_| "worker_manager_poisoned")?
                    .handle(&worker_id)
                    .ok_or_else(|| "session_worker_not_found".to_string())?
                    .cancel_current_turn();
            }
        }
        ClientCommand::AttachmentRemove {
            session_id,
            attachment_id,
        } => {
            remove_pending_attachment(state, &session_id, &attachment_id)?;
            return Ok(Some(WireEvent::AttachmentRemoved {
                session_id,
                attachment_id,
            }));
        }
        ClientCommand::HistoryPage {
            session_id,
            before_cursor,
            limit,
        } => {
            let page = current_session_store(state)?.read_history_page(
                &session_id,
                before_cursor.as_deref(),
                limit.unwrap_or(200).min(200),
            )?;
            if let Ok(mut sessions) = state.sessions.lock() {
                if let Some(session) = sessions.get_mut(&session_id) {
                    session.history_before_cursor = page.before_cursor.clone();
                    session.history_has_more = page.has_more;
                }
            }
            return Ok(Some(WireEvent::HistoryPage {
                session_id,
                records: page.records,
                before_cursor: page.before_cursor,
                has_more: page.has_more,
            }));
        }
        ClientCommand::ToolRepoSearch {
            session_id,
            query,
            limit,
        } => {
            let tools =
                session_tool_repo(state, &session_id)?.search(&query, limit.unwrap_or(100))?;
            return Ok(Some(WireEvent::ToolRepoSearchResult {
                session_id,
                query,
                tools,
            }));
        }
        ClientCommand::ToolRepoDetail {
            session_id,
            tool_id,
        } => {
            let detail = session_tool_repo(state, &session_id)?.detail(&tool_id)?;
            return Ok(Some(WireEvent::ToolRepoDetail { session_id, detail }));
        }
        ClientCommand::ToolRepoRename {
            session_id,
            tool_id,
            new_name,
        } => {
            let repo = session_tool_repo(state, &session_id)?;
            repo.rename(&tool_id, &new_name)?;
            let tools = repo.list()?;
            if let Ok(mut sessions) = state.sessions.lock() {
                if let Some(session) = sessions.get_mut(&session_id) {
                    session.tools = tools.clone();
                }
            }
            return Ok(Some(WireEvent::ToolRepoUpdated { session_id, tools }));
        }
        ClientCommand::ToolRepoOpenTerminal {
            session_id,
            tool_id,
        } => {
            let detail = session_tool_repo(state, &session_id)?.detail(&tool_id)?;
            open_directory_in_terminal(Path::new(&detail.summary.path))?;
        }
        ClientCommand::TopicReply {
            session_id,
            worker_id,
            topic_name,
            request_id,
            decision,
            payload,
        } => {
            let decision = match decision.as_str() {
                "accept" => HostDecision::Accept,
                "decline" => HostDecision::Decline,
                _ => return Err("invalid_topic_reply_decision".to_string()),
            };
            let approval_summary = decision_summary(&topic_name, decision, &payload);
            if topic_name == CORE_TOPIC_WORK_INSTRUCTION_LOAD {
                if resolve_work_instruction_decision(
                    state,
                    &session_id,
                    request_id.as_deref(),
                    decision,
                )? {
                    let turn =
                        append_turn_user_entry(state, &session_id, "approval", approval_summary)?;
                    return Ok(Some(WireEvent::TurnUpdated { session_id, turn }));
                }
                return Ok(None);
            }
            if !session_has_active_turn(state, &session_id)? {
                return Ok(None);
            }
            let mut reply = TopicReply::new(session_id.clone(), topic_name, decision, payload);
            if let Some(request_id) = request_id {
                reply = reply.with_request_id(request_id);
            }
            relay_topic_reply_to_requesting_worker(
                state,
                &session_id,
                worker_id.as_deref(),
                reply,
            )?;
            return match append_turn_user_entry(state, &session_id, "approval", approval_summary) {
                Ok(turn) => Ok(Some(WireEvent::TurnUpdated { session_id, turn })),
                Err(error) if error == "active_turn_not_found" => Ok(None),
                Err(error) => Err(error),
            };
        }
        ClientCommand::RuntimeUpdate { key, value } => {
            let value = nonempty_text(value, "runtime config value")?;
            let report = update_runtime_setting(state, &key, &value)?;
            let session_env_defaults = state
                .template
                .settings
                .lock()
                .map(|settings| session_env_values(&settings))
                .map_err(|_| "runtime_settings_poisoned".to_string())?;
            let _ = state.events.send(WireEvent::HostConfigUpdated {
                key: report.key.to_string(),
                value: report.value,
                session_env_defaults,
            });
        }
        ClientCommand::MemSwitch { space } => {
            let snapshot = switch_mem_space(state, port, &space)?;
            let _ = state.events.send(WireEvent::Hello { snapshot });
        }
    }
    Ok(None)
}

fn switch_mem_space(state: &AppState, port: u16, space: &str) -> Result<WebSnapshot, String> {
    let space = space.trim();
    validate_web_space_name(space)?;
    let current_space = current_mem_state(state)?.space;
    if current_space == space {
        return Ok(snapshot_for(state, port));
    }
    let old_manager = {
        let mut manager = state
            .manager
            .lock()
            .map_err(|_| "worker_manager_poisoned".to_string())?;
        for worker_id in manager
            .statuses()
            .into_iter()
            .map(|status| status.identity.worker_id)
            .collect::<Vec<_>>()
        {
            let _ = manager.request_shutdown(&worker_id);
        }
        std::mem::take(&mut *manager)
    };
    let _ = old_manager.shutdown_all();
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        sessions.clear();
    }
    {
        let mut mem = state
            .mem
            .lock()
            .map_err(|_| "mem_state_poisoned".to_string())?;
        *mem = WebMemState::new(state.template.data_dir.clone(), space.to_string())?;
    }
    if restore_stored_sessions(state)? == 0 {
        let _ = create_session(state, None, None, BTreeMap::new())?;
    }
    Ok(snapshot_for(state, port))
}

fn create_session(
    state: &AppState,
    display_name: Option<String>,
    requested_workspace: Option<String>,
    env_overrides: BTreeMap<String, String>,
) -> Result<String, String> {
    let session_id = unique_web_id("session");
    let tool_repo = session_tool_repo(state, &session_id)?;
    let current_dir = state
        .template
        .resolve_workspace(requested_workspace.as_deref())?;
    let settings = state.template.session_settings(&env_overrides)?;
    let session_env = state.template.session_env(&settings, &env_overrides);
    let runtime = WebSessionRuntime {
        settings,
        env: session_env,
        env_overrides,
    };
    let max_llm_input_tokens = runtime.settings.config.max_llm_input_tokens;
    let runtime_profile = WebSessionRuntimeProfile::from_settings(&runtime.settings);
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let ordinal = sessions
            .values()
            .map(|session| session.ordinal)
            .max()
            .map(|value| value.saturating_add(1))
            .unwrap_or(0);
        sessions.insert(
            session_id.clone(),
            WebSession {
                session_id: session_id.clone(),
                display_name: display_name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| format!("Session{ordinal}")),
                ordinal,
                state: "ready".to_string(),
                current_dir: current_dir.display().to_string(),
                max_llm_input_tokens,
                tools: tool_repo.list()?,
                runtime_profile,
                contexts: Vec::new(),
                workers: Vec::new(),
                active_context_id: String::new(),
                primary_worker_id: String::new(),
                attachments: Vec::new(),
                consumed_attachment_ids: BTreeSet::new(),
                messages: Vec::new(),
                turns: Vec::new(),
                history_before_cursor: None,
                history_has_more: false,
                resume_notice_pending: false,
                active_turn_id: None,
                pending_completion_message_id: None,
                work_instruction_mode: runtime.settings.work_instruction_mode,
                work_instruction_allowed: None,
                pending_work_instruction_turn: None,
                runtime,
            },
        );
    }
    if let Err(error) =
        create_context_with_worker(state, &session_id, current_dir, display_name, None, true)
    {
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.remove(&session_id);
        }
        return Err(error);
    }
    persist_web_session(state, &session_id)?;
    Ok(session_id)
}

fn restore_stored_sessions(state: &AppState) -> Result<usize, String> {
    let stored_sessions = current_session_store(state)?.list_sessions()?;
    let mut restored = 0usize;
    for stored in stored_sessions {
        if restore_stored_session(state, stored).is_ok() {
            restored += 1;
        }
    }
    Ok(restored)
}

fn restore_stored_session(state: &AppState, stored: StoredSession) -> Result<(), String> {
    let current_dir = PathBuf::from(&stored.current_dir);
    if !current_dir.is_dir() {
        return Err("stored_session_workspace_not_found".to_string());
    }
    // Legacy records stored the complete resolved runtime in `env`, which made
    // restored sessions silently pin stale launch configuration. Only the new
    // provenance-aware field represents explicit per-session overrides.
    let env_overrides = stored.env_overrides.clone().unwrap_or_default();
    let settings = state.template.session_settings(&env_overrides)?;
    let session_env = state.template.session_env(&settings, &env_overrides);
    let runtime = WebSessionRuntime {
        settings,
        env: session_env,
        env_overrides,
    };
    let max_llm_input_tokens = runtime.settings.config.max_llm_input_tokens;
    let runtime_profile = WebSessionRuntimeProfile::from_settings(&runtime.settings);
    let tool_repo = session_tool_repo(state, &stored.session_id)?;
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let ordinal = sessions
            .values()
            .map(|session| session.ordinal)
            .max()
            .map(|value| value.saturating_add(1))
            .unwrap_or(0);
        let history_page = current_session_store(state)?.read_history_page(
            &stored.session_id,
            None,
            SESSION_HISTORY_PAGE_LIMIT,
        )?;
        let history_records = history_page.records;
        let messages = restored_messages_from_history_records(&history_records);
        let turns = restored_turns_from_history_records(&history_records);
        sessions.insert(
            stored.session_id.clone(),
            WebSession {
                session_id: stored.session_id.clone(),
                display_name: stored.display_name.clone(),
                ordinal,
                state: match stored.state {
                    StoredSessionState::Error => "error",
                    StoredSessionState::Interrupted | StoredSessionState::Ready => "ready",
                }
                .to_string(),
                current_dir: current_dir.display().to_string(),
                max_llm_input_tokens,
                tools: tool_repo.list()?,
                runtime_profile,
                contexts: Vec::new(),
                workers: Vec::new(),
                active_context_id: String::new(),
                primary_worker_id: String::new(),
                attachments: Vec::new(),
                consumed_attachment_ids: BTreeSet::new(),
                messages,
                turns,
                history_before_cursor: history_page.before_cursor,
                history_has_more: history_page.has_more,
                resume_notice_pending: true,
                active_turn_id: None,
                pending_completion_message_id: None,
                work_instruction_mode: runtime.settings.work_instruction_mode,
                work_instruction_allowed: None,
                pending_work_instruction_turn: None,
                runtime,
            },
        );
    }
    create_context_with_worker(
        state,
        &stored.session_id,
        current_dir,
        Some(stored.display_name),
        None,
        true,
    )?;
    Ok(())
}

fn restored_messages_from_history_records(records: &[ChatHistoryRecord]) -> Vec<WebChatMessage> {
    records
        .iter()
        .cloned()
        .filter_map(web_message_from_history_record)
        .collect()
}

fn restored_turns_from_history_records(records: &[ChatHistoryRecord]) -> Vec<WebTurn> {
    let mut turns = BTreeMap::<String, WebTurn>::new();
    for record in records.iter().cloned() {
        match record {
            ChatHistoryRecord::Message {
                role,
                turn_id,
                created_at_ms,
                kind,
                content,
            } => {
                let turn = turns.entry(turn_id.clone()).or_insert_with(|| WebTurn {
                    turn_id: turn_id.clone(),
                    state: "restored".to_string(),
                    created_at_ms: created_at_ms as u128,
                    user_entries: Vec::new(),
                    events: Vec::new(),
                    final_answer: None,
                    completion: None,
                });
                turn.created_at_ms = turn.created_at_ms.min(created_at_ms as u128);
                match role {
                    ChatHistoryRole::User => turn.user_entries.push(WebTurnUserEntry {
                        kind: history_user_entry_kind(kind.as_deref()).to_string(),
                        text: content,
                        attachments: Vec::new(),
                        created_at_ms: created_at_ms as u128,
                    }),
                    ChatHistoryRole::Assistant => {
                        turn.final_answer = Some(content);
                    }
                    ChatHistoryRole::System => {}
                }
            }
            ChatHistoryRecord::Event {
                turn_id,
                created_at_ms,
                kind,
                content: _,
                mut extra,
                ..
            } => {
                let payload = extra
                    .remove("payload")
                    .unwrap_or_else(|| json!({"kind": format!("{kind:?}")}));
                let source = extra
                    .remove("source")
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_else(|| "history".to_string());
                let turn = turns.entry(turn_id.clone()).or_insert_with(|| WebTurn {
                    turn_id: turn_id.clone(),
                    state: "restored".to_string(),
                    created_at_ms: created_at_ms as u128,
                    user_entries: Vec::new(),
                    events: Vec::new(),
                    final_answer: None,
                    completion: None,
                });
                turn.created_at_ms = turn.created_at_ms.min(created_at_ms as u128);
                turn.events.push(WebTurnEvent {
                    event_id: format!(
                        "history_event_{turn_id}_{created_at_ms}_{}",
                        turn.events.len()
                    ),
                    source,
                    payload,
                    created_at_ms: created_at_ms as u128,
                });
            }
        }
    }
    let mut restored = turns.into_values().collect::<Vec<_>>();
    restored.sort_by_key(|turn| turn.created_at_ms);
    restored
}

fn web_message_from_history_record(record: ChatHistoryRecord) -> Option<WebChatMessage> {
    match record {
        ChatHistoryRecord::Message {
            role,
            turn_id,
            created_at_ms,
            kind: _,
            content,
        } => {
            let role = match role {
                ChatHistoryRole::User => "user",
                ChatHistoryRole::Assistant => "assistant",
                ChatHistoryRole::System => return None,
            };
            Some(WebChatMessage {
                id: format!("history_msg_{turn_id}_{created_at_ms}_{role}"),
                role: role.to_string(),
                text: content,
                created_at_ms: created_at_ms as u128,
                completion: None,
            })
        }
        ChatHistoryRecord::Event { .. } => None,
    }
}

fn history_user_entry_kind(kind: Option<&str>) -> &str {
    match kind {
        Some(kind @ ("task" | "supplement" | "approval")) => kind,
        _ => "task",
    }
}

fn persist_web_session(state: &AppState, session_id: &str) -> Result<(), String> {
    let stored = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        stored_session_from_web_session(state, session)
    };
    current_session_store(state)?.upsert_session(&stored)
}

fn stored_session_from_web_session(state: &AppState, session: &WebSession) -> StoredSession {
    StoredSession {
        session_id: session.session_id.clone(),
        display_name: session.display_name.clone(),
        created_at_ms: session
            .turns
            .first()
            .map(|turn| turn.created_at_ms as i64)
            .unwrap_or_else(now_ms_i64),
        updated_at_ms: now_ms_i64(),
        current_dir: session.current_dir.clone(),
        profile: StoredSessionProfile {
            provider: session.runtime.settings.config.provider.clone(),
            model: session.runtime.settings.config.model.clone(),
            api_protocol: session
                .runtime
                .settings
                .config
                .api_protocol
                .label()
                .to_string(),
            response_protocol: session
                .runtime
                .settings
                .config
                .response_protocol
                .name()
                .to_string(),
        },
        // Keep the legacy field empty. Resolved settings belong to the launch
        // environment and must not become persistent per-session overrides.
        env: BTreeMap::new(),
        env_overrides: Some(
            session
                .runtime
                .env_overrides
                .iter()
                .filter(|(key, _)| key.as_str() != "TIMEM_API_KEY")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        state: if session.state == "error" {
            StoredSessionState::Error
        } else if session.active_turn_id.is_some() || session.state == "working" {
            StoredSessionState::Interrupted
        } else {
            StoredSessionState::Ready
        },
        last_turn_id: session.turns.last().map(|turn| turn.turn_id.clone()),
        raw_chat_history_path: state
            .mem
            .lock()
            .map(|mem| {
                mem.session_store
                    .history_path_for_session(&session.session_id)
            })
            .unwrap_or_else(|_| PathBuf::from(""))
            .display()
            .to_string(),
    }
}

fn session_has_active_turn(state: &AppState, session_id: &str) -> Result<bool, String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    Ok(session
        .active_turn_id
        .as_ref()
        .is_some_and(|turn_id| session.turns.iter().any(|turn| turn.turn_id == *turn_id)))
}

fn submit_or_supplement_turn(
    state: &AppState,
    session_id: &str,
    text: String,
) -> Result<WebTurn, String> {
    if session_has_active_turn(state, session_id)? {
        if let Some(turn) = try_append_turn_supplement(state, session_id, text.clone())? {
            return Ok(turn);
        }
    }
    submit_turn(state, session_id, text)
}

fn append_supplement_or_submit_turn(
    state: &AppState,
    session_id: &str,
    text: String,
) -> Result<WebTurn, String> {
    match try_append_turn_supplement(state, session_id, text.clone())? {
        Some(turn) => Ok(turn),
        None => submit_turn(state, session_id, text),
    }
}

fn try_append_turn_supplement(
    state: &AppState,
    session_id: &str,
    text: String,
) -> Result<Option<WebTurn>, String> {
    if !session_has_active_turn(state, session_id)? {
        return Ok(None);
    }
    let worker_handle = primary_worker_handle(state, session_id)?;
    let worker_supplement_text = text.clone();
    match append_turn_supplement_with_pending_attachments(state, session_id, text) {
        Ok(turn) => {
            let entry = turn.user_entries.last().cloned();
            let mut supplement = entry
                .as_ref()
                .map(|entry| entry.text.clone())
                .unwrap_or(worker_supplement_text);
            if let Some(context) = entry
                .as_ref()
                .and_then(|entry| uploaded_files_context(&entry.attachments))
            {
                supplement.push_str("\n\n");
                supplement.push_str(&context);
            }
            worker_handle.add_user_supplement(supplement);
            Ok(Some(turn))
        }
        Err(error) if error == "active_turn_not_found" => Ok(None),
        Err(error) => Err(error),
    }
}

fn create_context_with_worker(
    state: &AppState,
    session_id: &str,
    current_dir: PathBuf,
    display_name: Option<String>,
    parent_worker_id: Option<String>,
    primary: bool,
) -> Result<(String, String), String> {
    let context_id = unique_web_id("context");
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        session.contexts.push(WebContext {
            context_id: context_id.clone(),
            current_dir: current_dir.display().to_string(),
            worker_ids: Vec::new(),
        });
    }
    match attach_worker_to_session_context(
        state,
        session_id,
        &context_id,
        display_name,
        parent_worker_id,
        primary,
    ) {
        Ok(worker_id) => {
            if primary {
                let mut sessions = state
                    .sessions
                    .lock()
                    .map_err(|_| "session_store_poisoned".to_string())?;
                let session = sessions
                    .get_mut(session_id)
                    .ok_or_else(|| "session_not_found".to_string())?;
                session.active_context_id = context_id.clone();
                session.current_dir = current_dir.display().to_string();
            }
            Ok((context_id, worker_id))
        }
        Err(error) => {
            if let Ok(mut sessions) = state.sessions.lock() {
                if let Some(session) = sessions.get_mut(session_id) {
                    session
                        .contexts
                        .retain(|context| context.context_id != context_id);
                }
            }
            Err(error)
        }
    }
}

fn attach_worker_to_session_context(
    state: &AppState,
    session_id: &str,
    context_id: &str,
    display_name: Option<String>,
    parent_worker_id: Option<String>,
    primary: bool,
) -> Result<String, String> {
    let (runtime, current_dir) = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if let Some(parent_worker_id) = parent_worker_id.as_deref() {
            if !session
                .workers
                .iter()
                .any(|worker| worker.worker_id == parent_worker_id)
            {
                return Err("parent_worker_not_in_session".to_string());
            }
        }
        let context = session
            .contexts
            .iter()
            .find(|context| context.context_id == context_id)
            .ok_or_else(|| "session_context_not_found".to_string())?;
        if !context.worker_ids.is_empty() {
            return Err("session_context_worker_exists".to_string());
        }
        (session.runtime.clone(), PathBuf::from(&context.current_dir))
    };

    let mem = current_mem_state(state)?;
    let core =
        state
            .template
            .new_core_at(&mem, &current_dir, &runtime.settings, runtime.env.clone())?;
    let workspace = state
        .template
        .workspace_at(&mem, &current_dir, runtime.env.clone());
    let requested_display_name = display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let worker_id = state
        .manager
        .lock()
        .map_err(|_| "worker_manager_poisoned".to_string())?
        .spawn_worker_in_session(
            core,
            runtime.settings.config,
            workspace,
            session_id.to_string(),
            context_id.to_string(),
            display_name,
            parent_worker_id,
        )?;
    let identity = state
        .manager
        .lock()
        .map_err(|_| "worker_manager_poisoned".to_string())?
        .statuses()
        .into_iter()
        .find(|status| status.identity.worker_id == worker_id)
        .map(|status| status.identity)
        .ok_or_else(|| "created_worker_not_found".to_string())?;

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    let context = session
        .contexts
        .iter_mut()
        .find(|context| context.context_id == context_id)
        .ok_or_else(|| "session_context_not_found".to_string())?;
    context.worker_ids.push(worker_id.clone());
    session.workers.push(WebWorker {
        worker_id: worker_id.clone(),
        context_id: context_id.to_string(),
        display_name: identity.display_name.clone(),
        ordinal: identity.ordinal,
        state: "ready".to_string(),
        parent_worker_id: identity.parent_worker_id,
    });
    if primary {
        session.primary_worker_id = worker_id.clone();
        if let Some(display_name) = requested_display_name {
            session.display_name = display_name;
        }
    }
    Ok(worker_id)
}

fn submit_turn(state: &AppState, session_id: &str, text: String) -> Result<WebTurn, String> {
    let request = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if session.work_instruction_mode == WorkInstructionLoadMode::Ask
            && session.work_instruction_allowed.is_none()
        {
            work_instruction_load_request(Path::new(&session.current_dir))
        } else {
            None
        }
    };
    if let Some(request) = request {
        let turn = start_web_turn(state, session_id, &text)?;
        let attachments = turn.user_entries[0].attachments.clone();
        let event = HostDecisionRequest::WorkInstructionLoad(request).topic_event(session_id);
        let request_id = event.payload["request_id"]
            .as_str()
            .ok_or_else(|| "work_instruction_request_id_missing".to_string())?
            .to_string();
        {
            let mut sessions = state
                .sessions
                .lock()
                .map_err(|_| "session_store_poisoned")?;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| "session_not_found".to_string())?;
            session.state = "working".to_string();
            session.pending_work_instruction_turn = Some(PendingWorkInstructionTurn {
                request_id: request_id.clone(),
                text,
                attachments,
            });
        }
        let wire_payload = event.wire_payload();
        let turn_ref =
            append_active_turn_event(state, session_id, "core_topic", wire_payload.clone());
        let _ = state.events.send(WireEvent::CoreTopic {
            turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
            turn_event_id: turn_ref.map(|value| value.event_id),
            event: wire_payload,
        });
        let timeout_state = state.clone();
        let timeout_session = session_id.to_string();
        tokio::spawn(async move {
            sleep(WORK_INSTRUCTION_DECISION_TIMEOUT).await;
            if resolve_work_instruction_decision(
                &timeout_state,
                &timeout_session,
                Some(&request_id),
                HostDecision::Decline,
            )
            .unwrap_or(false)
            {
                if let Ok((context_id, worker_id)) =
                    primary_worker_scope(&timeout_state, &timeout_session)
                {
                    emit_worker_activity(
                        &timeout_state,
                        &timeout_session,
                        &context_id,
                        &worker_id,
                        json!({ "kind": "work_instruction_request_timeout" }),
                    );
                }
            }
        });
        return Ok(turn);
    }
    let turn = start_web_turn(state, session_id, &text)?;
    let attachments = turn.user_entries[0].attachments.clone();
    if let Err(error) = primary_worker_handle(state, session_id)?
        .run_turn(text, session_context(state, session_id, &attachments)?)
    {
        rollback_web_turn(state, session_id, &turn.turn_id, attachments);
        return Err(error);
    }
    Ok(turn)
}

fn resolve_work_instruction_decision(
    state: &AppState,
    session_id: &str,
    request_id: Option<&str>,
    decision: HostDecision,
) -> Result<bool, String> {
    let pending = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        let Some(pending) = session.pending_work_instruction_turn.as_ref() else {
            return Ok(false);
        };
        if request_id != Some(pending.request_id.as_str()) {
            return Err("topic_reply_request_id_mismatch".to_string());
        }
        session.work_instruction_allowed = Some(decision.as_bool());
        session.pending_work_instruction_turn.take()
    };
    let Some(pending) = pending else {
        return Ok(false);
    };
    if decision.as_bool() {
        if let Some(event) = work_instruction_notice_event(state, session_id) {
            let _ = state.events.send(event);
        }
    }
    primary_worker_handle(state, session_id)?.run_turn(
        pending.text,
        session_context(state, session_id, &pending.attachments)?,
    )?;
    Ok(true)
}

fn primary_worker_handle(
    state: &AppState,
    session_id: &str,
) -> Result<agent_core::CoreSessionWorkerHandle, String> {
    session_worker_handle(state, session_id, None)
}

fn session_worker_handle(
    state: &AppState,
    session_id: &str,
    requested_worker_id: Option<&str>,
) -> Result<agent_core::CoreSessionWorkerHandle, String> {
    let worker_id = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        let worker_id = requested_worker_id.unwrap_or(&session.primary_worker_id);
        if !session
            .workers
            .iter()
            .any(|worker| worker.worker_id == worker_id)
        {
            return Err("session_worker_scope_mismatch".to_string());
        }
        worker_id.to_string()
    };
    state
        .manager
        .lock()
        .map_err(|_| "worker_manager_poisoned")?
        .handle(&worker_id)
        .ok_or_else(|| "session_worker_not_found".to_string())
}

fn relay_topic_reply_to_requesting_worker(
    state: &AppState,
    session_id: &str,
    requesting_worker_id: Option<&str>,
    reply: TopicReply,
) -> Result<(), String> {
    // The browser has one user-facing conversation per Session. worker_id is
    // only the return address used to relay that visible decision to the worker
    // whose core request is waiting.
    session_worker_handle(state, session_id, requesting_worker_id)?.reply_to_request(reply)
}

fn primary_worker_scope(state: &AppState, session_id: &str) -> Result<(String, String), String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    let worker = session
        .workers
        .iter()
        .find(|worker| worker.worker_id == session.primary_worker_id)
        .ok_or_else(|| "session_primary_worker_not_found".to_string())?;
    Ok((worker.context_id.clone(), worker.worker_id.clone()))
}

fn session_worker_ids(state: &AppState, session_id: &str) -> Result<Vec<String>, String> {
    state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .get(session_id)
        .map(|session| {
            session
                .workers
                .iter()
                .map(|worker| worker.worker_id.clone())
                .collect()
        })
        .ok_or_else(|| "session_not_found".to_string())
}

fn append_message(
    state: &AppState,
    session_id: &str,
    role: &str,
    text: String,
) -> Result<String, String> {
    let message = WebChatMessage {
        id: unique_web_id(&format!("msg_{role}")),
        role: role.to_string(),
        text,
        created_at_ms: now_ms(),
        completion: None,
    };
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned")?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    let message_id = message.id.clone();
    let turn_id = session.active_turn_id.clone();
    let role_for_history = message.role.clone();
    let text_for_history = message.text.clone();
    let created_at_ms = message.created_at_ms as i64;
    session.messages.push(message);
    if session.messages.len() > MAX_SESSION_MESSAGES {
        let excess = session.messages.len() - MAX_SESSION_MESSAGES;
        session.messages.drain(..excess);
    }
    drop(sessions);
    if let Some(turn_id) = turn_id {
        append_chat_history_message(
            state,
            session_id,
            &turn_id,
            &role_for_history,
            None,
            created_at_ms,
            text_for_history,
        );
    }
    Ok(message_id)
}

fn start_web_turn(state: &AppState, session_id: &str, text: &str) -> Result<WebTurn, String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    if session.active_turn_id.is_some() {
        return Err("turn_already_active_use_supplement".to_string());
    }
    let attachments = std::mem::take(&mut session.attachments);
    let turn = WebTurn {
        turn_id: unique_web_id("web_turn"),
        state: "working".to_string(),
        created_at_ms: now_ms(),
        user_entries: vec![WebTurnUserEntry {
            kind: "task".to_string(),
            text: text.to_string(),
            attachments,
            created_at_ms: now_ms(),
        }],
        events: Vec::new(),
        final_answer: None,
        completion: None,
    };
    session.state = "working".to_string();
    session.active_turn_id = Some(turn.turn_id.clone());
    session.turns.push(turn.clone());
    if session.turns.len() > MAX_SESSION_TURNS {
        let excess = session.turns.len() - MAX_SESSION_TURNS;
        session.turns.drain(..excess);
    }
    session.messages.push(WebChatMessage {
        id: unique_web_id("msg_user"),
        role: "user".to_string(),
        text: text.to_string(),
        created_at_ms: now_ms(),
        completion: None,
    });
    if session.messages.len() > MAX_SESSION_MESSAGES {
        let excess = session.messages.len() - MAX_SESSION_MESSAGES;
        session.messages.drain(..excess);
    }
    let turn_id = turn.turn_id.clone();
    let created_at_ms = turn.created_at_ms as i64;
    drop(sessions);
    append_chat_history_message(
        state,
        session_id,
        &turn_id,
        "user",
        Some("task"),
        created_at_ms,
        text.to_string(),
    );
    let _ = persist_web_session(state, session_id);
    Ok(turn)
}

fn submit_toolgen_turn(
    state: &AppState,
    session_id: &str,
    source_turn_id: &str,
    user_instruction: Option<String>,
) -> Result<WebTurn, String> {
    {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if session.active_turn_id.is_some() {
            return Err("turn_already_active".to_string());
        }
        let turn = session
            .turns
            .iter()
            .find(|turn| turn.turn_id == source_turn_id)
            .cloned()
            .ok_or_else(|| "toolgen_source_turn_not_found".to_string())?;
        if turn.state == "working" || turn.completion.is_none() {
            return Err("toolgen_source_turn_not_completed".to_string());
        }
        if turn.final_answer.as_deref().unwrap_or("").trim().is_empty() {
            return Err("toolgen_source_final_answer_missing".to_string());
        }
    }
    let user_instruction = user_instruction
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let turn = start_web_toolgen_turn(
        state,
        session_id,
        source_turn_id,
        user_instruction.as_deref(),
    )?;
    let request = ToolGenRequest::new(user_instruction);
    if let Err(error) = primary_worker_handle(state, session_id)?.run_toolgen(request) {
        rollback_web_turn(state, session_id, &turn.turn_id, Vec::new());
        return Err(error);
    }
    Ok(turn)
}

fn start_web_toolgen_turn(
    state: &AppState,
    session_id: &str,
    source_turn_id: &str,
    user_instruction: Option<&str>,
) -> Result<WebTurn, String> {
    let created_at_ms = now_ms();
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    if session.active_turn_id.is_some() {
        return Err("turn_already_active".to_string());
    }
    let user_entries = user_instruction
        .map(|text| {
            vec![WebTurnUserEntry {
                kind: "toolgen_instruction".to_string(),
                text: text.to_string(),
                attachments: Vec::new(),
                created_at_ms,
            }]
        })
        .unwrap_or_default();
    let turn = WebTurn {
        turn_id: unique_web_id("web_toolgen_turn"),
        state: "working".to_string(),
        created_at_ms,
        user_entries,
        events: Vec::new(),
        final_answer: None,
        completion: None,
    };
    session.state = "working".to_string();
    session.active_turn_id = Some(turn.turn_id.clone());
    session.pending_completion_message_id = None;
    session.turns.push(turn.clone());
    if session.turns.len() > MAX_SESSION_TURNS {
        let excess = session.turns.len() - MAX_SESSION_TURNS;
        session.turns.drain(..excess);
    }
    drop(sessions);

    if let Some(text) = user_instruction {
        append_chat_history_message(
            state,
            session_id,
            &turn.turn_id,
            "user",
            Some("toolgen_instruction"),
            created_at_ms as i64,
            text.to_string(),
        );
    }
    let mut extra = BTreeMap::new();
    extra.insert(
        "source_turn_id".to_string(),
        Value::String(source_turn_id.to_string()),
    );
    current_session_store(state)?.append_history_record(
        session_id,
        &ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: turn.turn_id.clone(),
            created_at_ms: created_at_ms as i64,
            kind: ChatHistoryEventKind::RuntimeNotice,
            content: "ToolGen requested for a completed turn.".to_string(),
            extra,
        },
    )?;
    persist_web_session(state, session_id)?;
    Ok(turn)
}

fn append_turn_user_entry(
    state: &AppState,
    session_id: &str,
    kind: &str,
    text: String,
) -> Result<WebTurn, String> {
    append_turn_user_entry_with_attachments(state, session_id, kind, text, Vec::new(), false)
}

fn append_turn_supplement_with_pending_attachments(
    state: &AppState,
    session_id: &str,
    text: String,
) -> Result<WebTurn, String> {
    append_turn_user_entry_with_attachments(state, session_id, "supplement", text, Vec::new(), true)
}

fn append_turn_user_entry_with_attachments(
    state: &AppState,
    session_id: &str,
    kind: &str,
    text: String,
    attachments: Vec<WebAttachment>,
    take_pending_attachments: bool,
) -> Result<WebTurn, String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    let active_turn_id = session
        .active_turn_id
        .clone()
        .ok_or_else(|| "active_turn_not_found".to_string())?;
    let turn = session
        .turns
        .iter_mut()
        .find(|turn| turn.turn_id == active_turn_id)
        .ok_or_else(|| "active_turn_not_found".to_string())?;
    let attachments = if attachments.is_empty() && take_pending_attachments {
        std::mem::take(&mut session.attachments)
    } else {
        attachments
    };
    turn.user_entries.push(WebTurnUserEntry {
        kind: kind.to_string(),
        text,
        attachments,
        created_at_ms: now_ms(),
    });
    if turn.user_entries.len() > MAX_TURN_USER_ENTRIES {
        let excess = turn.user_entries.len() - MAX_TURN_USER_ENTRIES;
        turn.user_entries.drain(..excess);
    }
    let turn_snapshot = turn.clone();
    let created_at_ms = turn_snapshot
        .user_entries
        .last()
        .map(|entry| entry.created_at_ms as i64)
        .unwrap_or_else(now_ms_i64);
    let last_entry = turn_snapshot.user_entries.last().cloned();
    let content = last_entry
        .as_ref()
        .map(|entry| entry.text.clone())
        .unwrap_or_default();
    let history_kind = last_entry.as_ref().map(|entry| entry.kind.as_str());
    drop(sessions);
    append_chat_history_message(
        state,
        session_id,
        &active_turn_id,
        "user",
        history_kind,
        created_at_ms,
        content,
    );
    let _ = persist_web_session(state, session_id);
    Ok(turn_snapshot)
}

fn rollback_web_turn(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    attachments: Vec<WebAttachment>,
) {
    if let Ok(mut sessions) = state.sessions.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            session.turns.retain(|turn| turn.turn_id != turn_id);
            if session.active_turn_id.as_deref() == Some(turn_id) {
                session.active_turn_id = None;
                session.state = "ready".to_string();
            }
            session.attachments.splice(0..0, attachments);
        }
    }
}

struct ActiveTurnEventRef {
    turn_id: String,
    event_id: String,
}

fn append_active_turn_event(
    state: &AppState,
    session_id: &str,
    source: &str,
    payload: Value,
) -> Option<ActiveTurnEventRef> {
    let mut sessions = state.sessions.lock().ok()?;
    let session = sessions.get_mut(session_id)?;
    let active_turn_id = session.active_turn_id.clone()?;
    let turn = session
        .turns
        .iter_mut()
        .find(|turn| turn.turn_id == active_turn_id)?;
    let event_id = unique_web_id("turn_event");
    turn.events.push(WebTurnEvent {
        event_id: event_id.clone(),
        source: source.to_string(),
        payload,
        created_at_ms: now_ms(),
    });
    if turn.events.len() > MAX_TURN_EVENTS {
        let excess = turn.events.len() - MAX_TURN_EVENTS;
        turn.events.drain(..excess);
    }
    let history_event = turn
        .events
        .last()
        .map(|event| (event.created_at_ms as i64, event.payload.clone()));
    let turn_id = active_turn_id.clone();
    drop(sessions);
    if let Some((created_at_ms, payload)) = history_event {
        append_chat_history_event(
            state,
            session_id,
            &turn_id,
            created_at_ms,
            chat_history_kind_for_source(source, &payload),
            source,
            payload,
        );
    }
    Some(ActiveTurnEventRef {
        turn_id: active_turn_id,
        event_id,
    })
}

fn append_chat_history_message(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    role: &str,
    kind: Option<&str>,
    created_at_ms: i64,
    content: String,
) {
    let role = match role {
        "user" => ChatHistoryRole::User,
        "assistant" => ChatHistoryRole::Assistant,
        "system" => ChatHistoryRole::System,
        _ => ChatHistoryRole::System,
    };
    if let Ok(store) = current_session_store(state) {
        let _ = store.append_history_record(
            session_id,
            &ChatHistoryRecord::Message {
                role,
                turn_id: turn_id.to_string(),
                created_at_ms,
                kind: kind.map(ToString::to_string),
                content,
            },
        );
    }
}

fn append_chat_history_event(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    created_at_ms: i64,
    kind: ChatHistoryEventKind,
    source: &str,
    payload: Value,
) {
    let content = history_event_content(source, &payload);
    let mut extra = BTreeMap::new();
    extra.insert("source".to_string(), Value::String(source.to_string()));
    extra.insert("payload".to_string(), payload);
    if let Ok(store) = current_session_store(state) {
        let _ = store.append_history_record(
            session_id,
            &ChatHistoryRecord::Event {
                role: ChatHistoryRole::System,
                turn_id: turn_id.to_string(),
                created_at_ms,
                kind,
                content,
                extra,
            },
        );
    }
}

fn chat_history_kind_for_source(source: &str, payload: &Value) -> ChatHistoryEventKind {
    if source == "core_topic" {
        let topic_name = payload
            .get("topic")
            .and_then(|topic| topic.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if topic_name == "core.action" {
            return ChatHistoryEventKind::Action;
        }
        if topic_name == "core.protocol_repair" {
            return ChatHistoryEventKind::Repair;
        }
        if topic_name == "core.context_compact" {
            return ChatHistoryEventKind::ContextCompact;
        }
        if topic_name == "core.model.response" {
            return ChatHistoryEventKind::Progress;
        }
    }
    ChatHistoryEventKind::RuntimeNotice
}

fn history_event_content(source: &str, payload: &Value) -> String {
    let compact = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    format!("{source}: {}", compact_text_for_history(&compact, 2_000))
}

fn compact_text_for_history(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = text.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn decision_summary(topic_name: &str, decision: HostDecision, payload: &Value) -> String {
    let choice = if decision.as_bool() {
        "Accepted"
    } else {
        "Declined"
    };
    let detail = payload
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match detail {
        Some(detail) => format!("{choice}: {detail}"),
        None => format!("{choice}: {topic_name}"),
    }
}

fn session_context(
    state: &AppState,
    session_id: &str,
    attachments: &[WebAttachment],
) -> Result<Option<String>, String> {
    let session = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        let mut session = session.clone();
        if session.resume_notice_pending {
            if let Some(stored) = sessions.get_mut(session_id) {
                stored.resume_notice_pending = false;
            }
            session.resume_notice_pending = true;
        }
        session
    };
    let current_dir = session.current_dir;
    let runtime = runtime_info_context(&["host: local_web", "transport: websocket"]);
    let resume_notice = if session.resume_notice_pending {
        Some(
            SessionResumeNotice {
                history_path: current_session_store(state)?.history_path_for_session(session_id),
                current_dir: PathBuf::from(&current_dir),
            }
            .render(),
        )
    } else {
        None
    };
    let tool_repo = session_tool_repo(state, session_id)?;
    let tool_repo_hint = if tool_repo.list()?.is_empty() {
        None
    } else {
        Some(format!(
            "Previously accumulated reusable scripts are available at: {}\nThe tool directories have semantic names. When one may help with the current task, inspect its short README and use the script through run_bash as needed.",
            tool_repo.root().display()
        ))
    };
    let instructions = match session.work_instruction_mode {
        WorkInstructionLoadMode::Silent => {
            work_instruction_load_report(Path::new(&current_dir)).context
        }
        WorkInstructionLoadMode::Ask if session.work_instruction_allowed == Some(true) => {
            work_instruction_load_report(Path::new(&current_dir)).context
        }
        WorkInstructionLoadMode::Ask | WorkInstructionLoadMode::Off => None,
    };
    let uploaded_files = uploaded_files_context(attachments);
    Ok(combine_additional_contexts([
        runtime.as_deref(),
        resume_notice.as_deref(),
        instructions.as_deref(),
        uploaded_files.as_deref(),
        tool_repo_hint.as_deref(),
    ]))
}

fn uploaded_files_context(attachments: &[WebAttachment]) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    Some(format!(
        "## SYSTEM\nFiles explicitly uploaded by the user for this session:\n{}",
        attachments
            .iter()
            .map(|file| format!("- {} ({})", file.name, file.path))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn sanitize_upload_name(name: &str) -> Result<String, String> {
    let name = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("invalid_upload_name".to_string());
    }
    let normalized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if normalized.is_empty() {
        Err("invalid_upload_name".to_string())
    } else {
        Ok(normalized.chars().take(160).collect())
    }
}

async fn store_upload(
    state: &AppState,
    session_id: &str,
    name: String,
    bytes: &[u8],
) -> Result<WebAttachment, String> {
    let session_uploads = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .get(session_id)
        .ok_or_else(|| "session_not_found".to_string())?
        .attachments
        .len();
    if session_uploads >= MAX_SESSION_UPLOADS {
        return Err("session_upload_limit_reached".to_string());
    }
    let id = unique_web_id("upload");
    let base_dir = state.template.data_dir.join("web_uploads").join(session_id);
    tokio::fs::create_dir_all(&base_dir)
        .await
        .map_err(|_| "upload_directory_create_failed".to_string())?;
    let path = base_dir.join(format!("{id}_{name}"));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|_| "upload_write_failed".to_string())?;
    let file = WebAttachment {
        id,
        name,
        path: path.display().to_string(),
        bytes: bytes.len(),
    };
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    if session.attachments.len() >= MAX_SESSION_UPLOADS {
        let _ = std::fs::remove_file(&path);
        return Err("session_upload_limit_reached".to_string());
    }
    session.attachments.push(file.clone());
    Ok(file)
}

fn remove_pending_attachment(
    state: &AppState,
    session_id: &str,
    attachment_id: &str,
) -> Result<(), String> {
    let (position, attachment) = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if session.consumed_attachment_ids.contains(attachment_id) {
            return Ok(());
        }
        let position = session
            .attachments
            .iter()
            .position(|attachment| attachment.id == attachment_id)
            .ok_or_else(|| "pending_attachment_not_found".to_string())?;
        (position, session.attachments.remove(position))
    };

    match std::fs::remove_file(&attachment.path) {
        Ok(()) => {
            mark_attachment_consumed(state, session_id, attachment_id);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            mark_attachment_consumed(state, session_id, attachment_id);
            Ok(())
        }
        Err(_) => {
            let mut sessions = state
                .sessions
                .lock()
                .map_err(|_| "session_store_poisoned".to_string())?;
            if let Some(session) = sessions.get_mut(session_id) {
                let restore_at = position.min(session.attachments.len());
                session.attachments.insert(restore_at, attachment);
            }
            Err("attachment_remove_failed".to_string())
        }
    }
}

fn mark_attachment_consumed(state: &AppState, session_id: &str, attachment_id: &str) {
    if let Ok(mut sessions) = state.sessions.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            session
                .consumed_attachment_ids
                .insert(attachment_id.to_string());
        }
    }
}

fn work_instruction_notice_event(state: &AppState, session_id: &str) -> Option<WireEvent> {
    let current_dir = {
        let sessions = state.sessions.lock().ok()?;
        let session = sessions.get(session_id)?;
        let loaded = session.work_instruction_mode == WorkInstructionLoadMode::Silent
            || (session.work_instruction_mode == WorkInstructionLoadMode::Ask
                && session.work_instruction_allowed == Some(true));
        loaded.then(|| PathBuf::from(&session.current_dir))?
    };
    let report = work_instruction_load_report(&current_dir);
    if report.file_names.is_empty() && report.error.is_none() {
        return None;
    }
    let event = agent_core::work_instruction_load_topic_event(session_id, &report);
    let wire_payload = event.wire_payload();
    let turn_ref = append_active_turn_event(state, session_id, "core_topic", wire_payload.clone());
    Some(WireEvent::CoreTopic {
        turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
        turn_event_id: turn_ref.map(|value| value.event_id),
        event: wire_payload,
    })
}

fn work_instruction_notice_events(state: &AppState) -> Vec<WireEvent> {
    let session_ids = state
        .sessions
        .lock()
        .map(|sessions| sessions.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    session_ids
        .into_iter()
        .filter_map(|session_id| {
            let should_notify = state
                .sessions
                .lock()
                .ok()?
                .get(&session_id)
                .map(|session| session.work_instruction_mode == WorkInstructionLoadMode::Silent)?;
            should_notify.then(|| work_instruction_notice_event(state, &session_id))?
        })
        .collect()
}

fn spawn_event_bridge(state: AppState) {
    tokio::spawn(async move {
        loop {
            let pending = drain_worker_events(&state);
            for (session_id, context_id, worker_id, event) in pending {
                handle_scoped_worker_event(&state, &session_id, &context_id, &worker_id, event);
            }
            sleep(EVENT_POLL_INTERVAL).await;
        }
    });
}

fn drain_worker_events(state: &AppState) -> Vec<(String, String, String, CoreSessionWorkerEvent)> {
    let workers = match state.sessions.lock() {
        Ok(sessions) => sessions
            .values()
            .flat_map(|session| {
                session.workers.iter().map(|worker| {
                    (
                        session.session_id.clone(),
                        worker.context_id.clone(),
                        worker.worker_id.clone(),
                    )
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    let Ok(mut manager) = state.manager.lock() else {
        return Vec::new();
    };
    let mut events = Vec::new();
    for (session_id, context_id, worker_id) in workers {
        while let Some(event) = manager.try_recv_event(&worker_id) {
            events.push((
                session_id.clone(),
                context_id.clone(),
                worker_id.clone(),
                event,
            ));
        }
    }
    events
}

fn emit_worker_activity(
    state: &AppState,
    session_id: &str,
    context_id: &str,
    worker_id: &str,
    mut event: Value,
) {
    event["session_id"] = json!(session_id);
    event["context_id"] = json!(context_id);
    event["worker_id"] = json!(worker_id);
    let turn_ref = append_active_turn_event(state, session_id, "worker_activity", event.clone());
    let _ = state.events.send(WireEvent::WorkerActivity {
        session_id: session_id.to_string(),
        context_id: context_id.to_string(),
        worker_id: worker_id.to_string(),
        turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
        turn_event_id: turn_ref.map(|value| value.event_id),
        event,
    });
}

fn handle_scoped_worker_event(
    state: &AppState,
    session_id: &str,
    context_id: &str,
    worker_id: &str,
    event: CoreSessionWorkerEvent,
) {
    match event {
        CoreSessionWorkerEvent::Topics(events) => {
            for event in events {
                let toolgen_scoped = event.payload.get("runtime_phase").and_then(Value::as_str)
                    == Some("toolgen")
                    || event.topic.name == CORE_TOPIC_TOOLGEN;
                // A worker event queue is bound to one session. Never allow an
                // inconsistent payload to update or leak into another session's UI.
                if event.session_id != session_id
                    || event.context_id.as_deref() != Some(context_id)
                    || event.worker_id.as_deref() != Some(worker_id)
                {
                    emit_worker_activity(
                        state,
                        session_id,
                        context_id,
                        worker_id,
                        json!({
                            "kind": "topic_scope_mismatch",
                            "expected_session_id": session_id,
                            "expected_context_id": context_id,
                            "expected_worker_id": worker_id,
                            "received_session_id": event.session_id,
                            "received_context_id": event.context_id,
                            "received_worker_id": event.worker_id,
                        }),
                    );
                    continue;
                }
                let mut wire_payload = event.wire_payload();
                if !toolgen_scoped {
                    if let Some(cwd) = event
                        .payload
                        .get("context_state")
                        .and_then(|value| value.get("cwd"))
                        .and_then(Value::as_str)
                    {
                        if let Ok(mut sessions) = state.sessions.lock() {
                            if let Some(session) = sessions.get_mut(session_id) {
                                if let Some(context) = session
                                    .contexts
                                    .iter_mut()
                                    .find(|context| context.context_id == context_id)
                                {
                                    context.current_dir = cwd.to_string();
                                }
                                if session.active_context_id == context_id {
                                    session.current_dir = cwd.to_string();
                                }
                            }
                        }
                        let _ = persist_web_session(state, session_id);
                    }
                }
                if let Some(response) = event.as_model_response() {
                    if !response.final_answer.is_empty()
                        && !toolgen_scoped
                        && is_primary_worker(state, session_id, worker_id)
                    {
                        if let Ok(message_id) =
                            append_message(state, session_id, "assistant", response.final_answer)
                        {
                            wire_payload["payload"]["ui_message_id"] = Value::String(message_id);
                            if let Ok(mut sessions) = state.sessions.lock() {
                                if let Some(session) = sessions.get_mut(session_id) {
                                    session.pending_completion_message_id = wire_payload["payload"]
                                        ["ui_message_id"]
                                        .as_str()
                                        .map(str::to_string);
                                    if let Some(active_turn_id) = session.active_turn_id.as_deref()
                                    {
                                        if let Some(turn) = session
                                            .turns
                                            .iter_mut()
                                            .find(|turn| turn.turn_id == active_turn_id)
                                        {
                                            turn.final_answer = Some(
                                                wire_payload["payload"]["final_answer"]
                                                    .as_str()
                                                    .unwrap_or_default()
                                                    .to_string(),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !toolgen_scoped {
                        set_worker_state(
                            state,
                            session_id,
                            worker_id,
                            if response.continue_work {
                                "working"
                            } else {
                                "ready"
                            },
                        );
                    }
                }
                if event.topic.name == CORE_TOPIC_TOOLGEN {
                    if let Ok(repo) = session_tool_repo(state, session_id) {
                        if let Ok(tools) = repo.list() {
                            if let Ok(mut sessions) = state.sessions.lock() {
                                if let Some(session) = sessions.get_mut(session_id) {
                                    session.tools = tools.clone();
                                }
                            }
                            let _ = state.events.send(WireEvent::ToolRepoUpdated {
                                session_id: session_id.to_string(),
                                tools,
                            });
                        }
                    }
                }
                if let Some(lifecycle) = event.as_lifecycle() {
                    if let Ok(mut sessions) = state.sessions.lock() {
                        if let Some(session) = sessions.get_mut(session_id) {
                            if let Some(worker) = lifecycle.worker {
                                if let Some(stored_worker) = session
                                    .workers
                                    .iter_mut()
                                    .find(|stored| stored.worker_id == worker_id)
                                {
                                    stored_worker.display_name = worker.display_name.clone();
                                }
                            }
                            session.max_llm_input_tokens = lifecycle.max_llm_input_tokens;
                        }
                    }
                    set_worker_state(state, session_id, worker_id, "ready");
                }
                let turn_ref = if event.topic.name == agent_core::CORE_TOPIC_LIFECYCLE {
                    None
                } else {
                    append_active_turn_event(state, session_id, "core_topic", wire_payload.clone())
                };
                let _ = state.events.send(WireEvent::CoreTopic {
                    turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
                    turn_event_id: turn_ref.map(|value| value.event_id),
                    event: wire_payload,
                });
            }
        }
        CoreSessionWorkerEvent::ModelRequest { round } => {
            set_worker_state(state, session_id, worker_id, "working");
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "model_request", "round": round }),
            );
        }
        CoreSessionWorkerEvent::ModelResponse {
            round,
            usage,
            runtime_phase,
        } => {
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "model_response", "round": round, "usage": usage, "runtime_phase": runtime_phase }),
            );
        }
        CoreSessionWorkerEvent::ModelResponseDiscarded { round, reason } => {
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "model_response_discarded", "round": round, "reason": reason }),
            );
        }
        CoreSessionWorkerEvent::ModelRetry {
            attempt,
            max_attempts,
            delay,
            error,
        } => {
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "model_retry", "attempt": attempt, "max_attempts": max_attempts, "delay_ms": delay.as_millis(), "error": error }),
            );
        }
        CoreSessionWorkerEvent::ModelError { error } => {
            set_worker_state(state, session_id, worker_id, "error");
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "model_error", "error": error }),
            );
        }
        CoreSessionWorkerEvent::TurnFinished { outcome } => {
            set_worker_state(state, session_id, worker_id, "ready");
            if !is_primary_worker(state, session_id, worker_id) {
                emit_worker_activity(
                    state,
                    session_id,
                    context_id,
                    worker_id,
                    json!({ "kind": "subworker_turn_finished", "text": outcome.text }),
                );
                return;
            }
            let completion = json!({
                "stats": outcome.stats,
                "latest_usage": outcome.latest_usage,
                "elapsed_ms": outcome.elapsed.as_millis(),
                "repair_issue": outcome.repair_issue,
                "stop_reason": outcome.stop_reason.map(|reason| format!("{reason:?}")),
                "toolgen_retrospect": outcome.toolgen_retrospect,
            });
            let (message_id, turn_id) =
                if let Ok(mut sessions) = state.sessions.lock() {
                    sessions
                        .get_mut(session_id)
                        .map(|session| {
                            let turn_id = session.active_turn_id.take();
                            if let Some(active_turn_id) = turn_id.as_deref() {
                                if let Some(turn) = session
                                    .turns
                                    .iter_mut()
                                    .find(|turn| turn.turn_id == active_turn_id)
                                {
                                    turn.state = "finished".to_string();
                                    turn.completion = Some(completion.clone());
                                }
                            }
                            let message_id = session.pending_completion_message_id.take().and_then(
                                |message_id| {
                                    session
                                        .messages
                                        .iter_mut()
                                        .find(|message| message.id == message_id)
                                        .map(|message| {
                                            message.completion = Some(completion.clone());
                                            message.id.clone()
                                        })
                                },
                            );
                            (message_id, turn_id)
                        })
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };
            if let Some(turn_id) = turn_id.as_deref() {
                let mut extra = BTreeMap::new();
                extra.insert("completion".to_string(), completion.clone());
                if let Ok(store) = current_session_store(state) {
                    let _ = store.append_history_record(
                        session_id,
                        &ChatHistoryRecord::Event {
                            role: ChatHistoryRole::System,
                            turn_id: turn_id.to_string(),
                            created_at_ms: now_ms_i64(),
                            kind: ChatHistoryEventKind::Stats,
                            content: "Turn completed.".to_string(),
                            extra,
                        },
                    );
                }
            }
            let _ = persist_web_session(state, session_id);
            let _ = state.events.send(WireEvent::TurnFinished { session_id: session_id.to_string(), turn_id, outcome: json!({ "text": outcome.text, "message_id": message_id, "completion": completion }) });
        }
        CoreSessionWorkerEvent::WorkerStopped => {
            set_worker_state(state, session_id, worker_id, "stopped");
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "worker_stopped" }),
            );
        }
    }
}

fn is_primary_worker(state: &AppState, session_id: &str, worker_id: &str) -> bool {
    state
        .sessions
        .lock()
        .ok()
        .and_then(|sessions| {
            sessions
                .get(session_id)
                .map(|session| session.primary_worker_id == worker_id)
        })
        .unwrap_or(false)
}

fn set_worker_state(state: &AppState, session_id: &str, worker_id: &str, worker_state: &str) {
    if let Ok(mut sessions) = state.sessions.lock() {
        if let Some(session) = sessions.get_mut(session_id) {
            if let Some(worker) = session
                .workers
                .iter_mut()
                .find(|worker| worker.worker_id == worker_id)
            {
                worker.state = worker_state.to_string();
            }
            session.state = if session
                .workers
                .iter()
                .any(|worker| worker.state == "working")
            {
                "working"
            } else if session.workers.iter().any(|worker| worker.state == "error") {
                "error"
            } else if session
                .workers
                .iter()
                .all(|worker| worker.state == "stopped")
            {
                "stopped"
            } else {
                "ready"
            }
            .to_string();
        }
    }
}

fn snapshot_for(state: &AppState, port: u16) -> WebSnapshot {
    let sessions = state
        .sessions
        .lock()
        .map(|sessions| sessions.values().cloned().collect())
        .unwrap_or_default();
    let runtime_options = state
        .template
        .settings
        .lock()
        .map(|settings| {
            runtime_config_menu_report(
                &settings.config,
                settings.bash_approval_mode,
                settings.work_instruction_mode,
            )
            .items
            .into_iter()
            .map(|item| WebRuntimeOption {
                key: item.key.to_string(),
                value: item.value,
                applies_to: "new_sessions",
            })
            .collect()
        })
        .unwrap_or_default();
    let session_env_defaults = state
        .template
        .settings
        .lock()
        .map(|settings| session_env_values(&settings))
        .unwrap_or_default();
    let workspace_dirs = web_workspace_dirs(&state.template);
    let mem = current_mem_state(state)
        .map(|mem| mem.info())
        .unwrap_or_else(|_| WebMemInfo {
            space: "unknown".to_string(),
            data_dir: String::new(),
            space_dir: String::new(),
            memory_dir: String::new(),
        });
    WebSnapshot {
        server: ServerInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: 1,
            port,
            mem,
            runtime_options,
            session_env_defaults,
            workspace_dirs,
        },
        sessions,
    }
}

fn validate_web_space_name(space: &str) -> Result<(), String> {
    let trimmed = space.trim();
    if trimmed.is_empty() {
        return Err("mem_space_empty".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("mem_space_invalid".to_string());
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || Path::new(trimmed).is_absolute()
    {
        return Err("mem_space_must_be_name_not_path".to_string());
    }
    Ok(())
}

fn web_workspace_dirs(template: &WorkerTemplate) -> Vec<String> {
    template
        .workspace_dirs
        .iter()
        .chain(std::iter::once(&template.current_dir))
        .map(|path| path.display().to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn update_runtime_setting(
    state: &AppState,
    key: &str,
    value: &str,
) -> Result<agent_core::RuntimeConfigApplyReport, String> {
    let field = runtime_config_field_from_key(key)?;
    let mut settings = state
        .template
        .settings
        .lock()
        .map_err(|_| "runtime_settings_poisoned")?;
    let RuntimeSettings {
        config,
        bash_approval_mode,
        work_instruction_mode,
    } = &mut *settings;
    let effect = apply_runtime_config_value(
        config,
        bash_approval_mode,
        work_instruction_mode,
        field,
        value,
    )
    .map_err(|error| format!("invalid_runtime_config:{error:?}"))?;
    Ok(agent_core::runtime_config_apply_report(
        config,
        *bash_approval_mode,
        *work_instruction_mode,
        field,
        effect,
    ))
}

fn runtime_config_field_from_key(key: &str) -> Result<agent_core::RuntimeConfigField, String> {
    Ok(match key {
        "TIMEM_MODEL" => agent_core::RuntimeConfigField::Model,
        "TIMEM_GATEWAY_PROVIDER" => agent_core::RuntimeConfigField::GatewayProvider,
        "TIMEM_API_PROTOCOL" => agent_core::RuntimeConfigField::ApiProtocol,
        "TIMEM_BASE_URL" => agent_core::RuntimeConfigField::BaseUrl,
        "TIMEM_MAX_LLM_INPUT" => agent_core::RuntimeConfigField::MaxInput,
        "TIMEM_MAX_LLM_OUTPUT" => agent_core::RuntimeConfigField::MaxOutput,
        "TIMEM_BASH_APPROVAL" => agent_core::RuntimeConfigField::BashApproval,
        "TIMEM_WORK_INSTRUCTIONS" => agent_core::RuntimeConfigField::WorkInstructions,
        _ => return Err("unsupported_runtime_config_key".to_string()),
    })
}

impl WorkerTemplate {
    fn from_environment(launch: &WebLaunchOptions) -> Result<Self, String> {
        let env = std::env::vars().collect::<HashMap<_, _>>();
        let config = provider_config_from_sources(&launch.provider_source(), &env)?;
        let response_protocol = launch
            .response_protocol
            .as_deref()
            .or_else(|| env.get("TIMEM_RESPONSE_PROTOCOL").map(String::as_str))
            .map(ResponseProtocolKind::from_name)
            .unwrap_or_default();
        let space = launch
            .space
            .clone()
            .or_else(|| env.get("TIMEM_SPACE").cloned())
            .unwrap_or_else(|| ".test_mem".to_string());
        let data_dir = launch
            .data_dir
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(default_data_root);
        let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
        let workspace_dirs =
            load_workspace_dirs_from_path(&agent_core::workspace_config_file(&data_dir))
                .into_iter()
                .map(PathBuf::from)
                .collect();
        Ok(Self {
            settings: Arc::new(Mutex::new(RuntimeSettings {
                config: ProviderConfig {
                    response_protocol,
                    ..config
                },
                bash_approval_mode: agent_core::bash_approval_mode_from_sources(
                    launch.bash_approval.as_deref(),
                    &env,
                ),
                work_instruction_mode: work_instruction_mode_from_sources(
                    launch.work_instructions.as_deref(),
                    &env,
                ),
            })),
            data_dir,
            initial_space: space,
            env: env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            current_dir,
            workspace_dirs,
        })
    }

    fn new_core_at(
        &self,
        mem: &WebMemState,
        current_dir: &Path,
        settings: &RuntimeSettings,
        session_env: BTreeMap<String, String>,
    ) -> Result<AgentCore, String> {
        let memory_dir = mem.layout.memory_dir();
        let audit_file = mem.layout.api_audit_file();
        std::fs::create_dir_all(&memory_dir).map_err(|error| error.to_string())?;
        let mut core = AgentCore::new(STATIC_PROMPT, settings.config.core_profile(), &memory_dir);
        core.change_prompt_cwd(current_dir.display().to_string())?;
        core.set_response_protocol(settings.config.response_protocol);
        core.configure_runtime_from_host(&settings.config, settings.bash_approval_mode);
        core.configure_self_tool_runtime(
            session_env,
            SelfToolPaths {
                space_dir: absolute_path(memory_dir.parent().unwrap_or(&memory_dir)),
                memory_dir: absolute_path(&memory_dir),
                memory_file: absolute_path(memory_dir.join("memory.jsonl")),
                scratch_file: absolute_path(memory_dir.join("scratch_notes.jsonl")),
                api_audit_file: absolute_path(&audit_file),
                action_audit_file: absolute_path(audit_file.with_file_name("action_audit.json")),
            },
        );
        if let Ok(registry) =
            CapabilityRegistry::builtin_with_overlay_dir(self.data_dir.join("capabilities"))
        {
            core.set_capability_registry(registry);
        }
        Ok(core)
    }

    fn workspace_at(
        &self,
        mem: &WebMemState,
        current_dir: &Path,
        session_env: BTreeMap<String, String>,
    ) -> CoreSessionWorkerWorkspace {
        let mut workspace = CoreSessionWorkerWorkspace::new(
            self.data_dir.clone(),
            mem.layout.api_audit_file(),
            "timem_web",
            "user_local_machine",
        );
        workspace.current_dir = Some(current_dir.to_path_buf());
        workspace.env = session_env;
        workspace.workspace_dirs = self.workspace_dirs.clone();
        workspace
    }

    fn session_settings(
        &self,
        env_overrides: &BTreeMap<String, String>,
    ) -> Result<RuntimeSettings, String> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "runtime_settings_poisoned")?
            .clone();
        for (key, value) in env_overrides {
            if value.trim().is_empty() {
                return Err(format!("empty_session_env_value:{key}"));
            }
            if !SESSION_ENV_KEYS.contains(&key.as_str()) {
                return Err(format!("unsupported_session_env_key:{key}"));
            }
        }

        if let Some(provider) = env_overrides.get("TIMEM_GATEWAY_PROVIDER") {
            apply_session_runtime_field(
                &mut settings,
                agent_core::RuntimeConfigField::GatewayProvider,
                provider,
            )?;
        }
        for key in [
            "TIMEM_MODEL",
            "TIMEM_API_PROTOCOL",
            "TIMEM_BASE_URL",
            "TIMEM_MAX_LLM_INPUT",
            "TIMEM_MAX_LLM_OUTPUT",
            "TIMEM_BASH_APPROVAL",
            "TIMEM_WORK_INSTRUCTIONS",
        ] {
            if let Some(value) = env_overrides.get(key) {
                apply_session_runtime_field(
                    &mut settings,
                    runtime_config_field_from_key(key)?,
                    value,
                )?;
            }
        }
        if let Some(value) = env_overrides.get("TIMEM_TIMEOUT") {
            settings.config.timeout_secs = value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "invalid_session_timeout".to_string())?;
        }
        if let Some(value) = env_overrides.get("TIMEM_RESPONSE_PROTOCOL") {
            settings.config.response_protocol = match value.trim().to_ascii_lowercase().as_str() {
                "markdown" => ResponseProtocolKind::Markdown,
                "json" => ResponseProtocolKind::Json,
                "xml" => ResponseProtocolKind::Xml,
                _ => return Err("invalid_session_response_protocol".to_string()),
            };
        }
        if let Some(value) = env_overrides.get("TIMEM_API_KEY") {
            validate_provider_api_key(value).map_err(|_| "invalid_session_api_key".to_string())?;
            settings.config.api_key = value.clone();
        }
        for key in [
            "TIMEM_ENABLE_THINKING",
            "TIMEM_REASONING_EFFORT",
            "TIMEM_STREAM",
        ] {
            if let Some(value) = env_overrides.get(key) {
                agent_core::apply_openai_compatible_env_value(
                    &mut settings.config.openai_compatible,
                    key,
                    value,
                )?;
            }
        }
        Ok(settings)
    }

    fn session_env(
        &self,
        settings: &RuntimeSettings,
        env_overrides: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut env = self.env.clone();
        env.extend(env_overrides.clone());
        env.insert(
            "TIMEM_GATEWAY_PROVIDER".to_string(),
            settings.config.provider.clone(),
        );
        env.insert("TIMEM_MODEL".to_string(), settings.config.model.clone());
        env.insert(
            "TIMEM_API_PROTOCOL".to_string(),
            settings.config.api_protocol.label().to_string(),
        );
        env.insert(
            "TIMEM_RESPONSE_PROTOCOL".to_string(),
            settings.config.response_protocol.name().to_string(),
        );
        env.insert(
            "TIMEM_BASE_URL".to_string(),
            settings.config.base_url.clone(),
        );
        env.insert(
            "TIMEM_TIMEOUT".to_string(),
            settings.config.timeout_secs.to_string(),
        );
        env.insert(
            "TIMEM_MAX_LLM_INPUT".to_string(),
            settings.config.max_llm_input_tokens.to_string(),
        );
        env.insert(
            "TIMEM_MAX_LLM_OUTPUT".to_string(),
            settings.config.max_llm_output_tokens.to_string(),
        );
        if let Some(value) = settings.config.openai_compatible.enable_thinking {
            env.insert("TIMEM_ENABLE_THINKING".to_string(), value.to_string());
        }
        if let Some(value) = &settings.config.openai_compatible.reasoning_effort {
            env.insert("TIMEM_REASONING_EFFORT".to_string(), value.clone());
        }
        env.insert(
            "TIMEM_STREAM".to_string(),
            settings.config.openai_compatible.stream.to_string(),
        );
        env
    }

    fn resolve_workspace(&self, requested: Option<&str>) -> Result<PathBuf, String> {
        let selected = match requested {
            Some(path) => {
                std::fs::canonicalize(path).map_err(|_| "workspace_not_found".to_string())?
            }
            None => self.current_dir.clone(),
        };
        if !selected.is_dir() {
            return Err("workspace_not_directory".to_string());
        }
        let mut allowed = self.workspace_dirs.clone();
        allowed.push(self.current_dir.clone());
        if allowed.iter().any(|candidate| {
            std::fs::canonicalize(candidate)
                .map(|candidate| candidate == selected)
                .unwrap_or(false)
        }) {
            Ok(selected)
        } else {
            Err("workspace_not_registered".to_string())
        }
    }
}

const SESSION_ENV_KEYS: &[&str] = &[
    "TIMEM_GATEWAY_PROVIDER",
    "TIMEM_MODEL",
    "TIMEM_API_PROTOCOL",
    "TIMEM_RESPONSE_PROTOCOL",
    "TIMEM_BASE_URL",
    "TIMEM_API_KEY",
    "TIMEM_TIMEOUT",
    "TIMEM_MAX_LLM_INPUT",
    "TIMEM_MAX_LLM_OUTPUT",
    "TIMEM_BASH_APPROVAL",
    "TIMEM_WORK_INSTRUCTIONS",
    "TIMEM_ENABLE_THINKING",
    "TIMEM_REASONING_EFFORT",
    "TIMEM_STREAM",
];

fn apply_session_runtime_field(
    settings: &mut RuntimeSettings,
    field: agent_core::RuntimeConfigField,
    value: &str,
) -> Result<(), String> {
    apply_runtime_config_value(
        &mut settings.config,
        &mut settings.bash_approval_mode,
        &mut settings.work_instruction_mode,
        field,
        value,
    )
    .map(|_| ())
    .map_err(|error| format!("invalid_session_env:{error:?}"))
}

impl WebSessionRuntimeProfile {
    fn from_settings(settings: &RuntimeSettings) -> Self {
        Self {
            provider: settings.config.provider.clone(),
            model: settings.config.model.clone(),
            api_protocol: settings.config.api_protocol.label().to_string(),
            response_protocol: settings.config.response_protocol.name().to_string(),
            base_url: settings.config.base_url.clone(),
            timeout_secs: settings.config.timeout_secs,
            max_llm_input_tokens: settings.config.max_llm_input_tokens,
            max_llm_output_tokens: settings.config.max_llm_output_tokens,
            bash_approval: agent_core::bash_approval_mode_label(settings.bash_approval_mode)
                .to_string(),
            work_instructions: agent_core::work_instruction_mode_label(
                settings.work_instruction_mode,
            )
            .to_string(),
        }
    }
}

fn session_env_values(settings: &RuntimeSettings) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        (
            "TIMEM_GATEWAY_PROVIDER".to_string(),
            settings.config.provider.clone(),
        ),
        ("TIMEM_MODEL".to_string(), settings.config.model.clone()),
        (
            "TIMEM_API_PROTOCOL".to_string(),
            settings.config.api_protocol.label().to_string(),
        ),
        (
            "TIMEM_RESPONSE_PROTOCOL".to_string(),
            settings.config.response_protocol.name().to_string(),
        ),
        (
            "TIMEM_BASE_URL".to_string(),
            settings.config.base_url.clone(),
        ),
        (
            "TIMEM_TIMEOUT".to_string(),
            settings.config.timeout_secs.to_string(),
        ),
        (
            "TIMEM_MAX_LLM_INPUT".to_string(),
            settings.config.max_llm_input_tokens.to_string(),
        ),
        (
            "TIMEM_MAX_LLM_OUTPUT".to_string(),
            settings.config.max_llm_output_tokens.to_string(),
        ),
        (
            "TIMEM_BASH_APPROVAL".to_string(),
            agent_core::bash_approval_mode_label(settings.bash_approval_mode).to_string(),
        ),
        (
            "TIMEM_WORK_INSTRUCTIONS".to_string(),
            agent_core::work_instruction_mode_label(settings.work_instruction_mode).to_string(),
        ),
    ]);
    if let Some(value) = settings.config.openai_compatible.enable_thinking {
        env.insert("TIMEM_ENABLE_THINKING".to_string(), value.to_string());
    }
    if let Some(value) = &settings.config.openai_compatible.reasoning_effort {
        env.insert("TIMEM_REASONING_EFFORT".to_string(), value.clone());
    }
    env.insert(
        "TIMEM_STREAM".to_string(),
        settings.config.openai_compatible.stream.to_string(),
    );
    env
}

#[derive(Debug)]
struct WebLaunchOptions {
    port: Option<u16>,
    space: Option<String>,
    provider: Option<String>,
    api_protocol: Option<String>,
    response_protocol: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    data_dir: Option<String>,
    timeout_secs: Option<u64>,
    max_llm_input_tokens: Option<u32>,
    max_llm_output_tokens: Option<u32>,
    bash_approval: Option<String>,
    work_instructions: Option<String>,
    open_browser: bool,
}

impl Default for WebLaunchOptions {
    fn default() -> Self {
        Self {
            port: None,
            space: None,
            provider: None,
            api_protocol: None,
            response_protocol: None,
            api_key: None,
            model: None,
            base_url: None,
            data_dir: None,
            timeout_secs: None,
            max_llm_input_tokens: None,
            max_llm_output_tokens: None,
            bash_approval: None,
            work_instructions: None,
            open_browser: true,
        }
    }
}

impl WebLaunchOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self::default();
        let mut index = 0;
        while index < args.len() {
            let key = &args[index];
            let value = args.get(index + 1).cloned();
            let mut string = |slot: &mut Option<String>| -> Result<(), String> {
                *slot = Some(
                    value
                        .clone()
                        .ok_or_else(|| format!("missing_value:{key}"))?,
                );
                index += 2;
                Ok(())
            };
            match key.as_str() {
                "--port" => {
                    options.port = Some(
                        value
                            .ok_or_else(|| "missing_value:--port".to_string())?
                            .parse()
                            .map_err(|_| "invalid_port".to_string())?,
                    );
                    index += 2;
                }
                "--space" => string(&mut options.space)?,
                "--gateway-provider" => string(&mut options.provider)?,
                "--api-protocol" => string(&mut options.api_protocol)?,
                "--response-protocol" => string(&mut options.response_protocol)?,
                "--api-key" => string(&mut options.api_key)?,
                "--model" => string(&mut options.model)?,
                "--base-url" => string(&mut options.base_url)?,
                "--data-dir" => string(&mut options.data_dir)?,
                "--bash-approval" => string(&mut options.bash_approval)?,
                "--work-instructions" => string(&mut options.work_instructions)?,
                "--no-open" => {
                    options.open_browser = false;
                    index += 1;
                }
                "--timeout" => {
                    options.timeout_secs = Some(
                        value
                            .ok_or_else(|| "missing_value:--timeout".to_string())?
                            .parse()
                            .map_err(|_| "invalid_timeout".to_string())?,
                    );
                    index += 2;
                }
                "--max-llm-input" => {
                    options.max_llm_input_tokens = Some(
                        value
                            .as_deref()
                            .ok_or_else(|| "missing_value:--max-llm-input".to_string())
                            .and_then(|value| {
                                agent_core::parse_token_count(value)
                                    .ok_or_else(|| "invalid_max_llm_input".to_string())
                            })?,
                    );
                    index += 2;
                }
                "--max-llm-output" => {
                    options.max_llm_output_tokens = Some(
                        value
                            .as_deref()
                            .ok_or_else(|| "missing_value:--max-llm-output".to_string())
                            .and_then(|value| {
                                agent_core::parse_token_count(value)
                                    .ok_or_else(|| "invalid_max_llm_output".to_string())
                            })?,
                    );
                    index += 2;
                }
                unknown if unknown.starts_with('-') => {
                    return Err(format!("unknown_option:{unknown}"))
                }
                _ => index += 1,
            }
        }
        if let Some(port) = options.port {
            if !(PORT_START..=PORT_END).contains(&port) {
                return Err(format!(
                    "port_out_of_range:{port}; expected {PORT_START}..={PORT_END}"
                ));
            }
        }
        Ok(options)
    }

    fn provider_source(&self) -> ProviderConfigSource {
        ProviderConfigSource {
            provider: self.provider.clone(),
            api_protocol: self.api_protocol.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            timeout_secs: self.timeout_secs,
            max_llm_output_tokens: self.max_llm_output_tokens,
            max_llm_input_tokens: self.max_llm_input_tokens,
            enable_thinking: None,
            reasoning_effort: None,
            stream: None,
            local_api_key: agent_core::LocalLLMKeyFile::load(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../key"),
            )
            .ok()
            .map(|key| key.api_key),
        }
    }
}

fn browser_command(url: &str) -> (OsString, Vec<OsString>) {
    #[cfg(target_os = "macos")]
    {
        (OsString::from("open"), vec![OsString::from(url)])
    }
    #[cfg(target_os = "windows")]
    {
        (
            OsString::from("cmd"),
            vec![
                OsString::from("/C"),
                OsString::from("start"),
                OsString::from(""),
                OsString::from(url),
            ],
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        (OsString::from("xdg-open"), vec![OsString::from(url)])
    }
}

fn open_browser(url: &str) -> Result<(), String> {
    let (program, args) = browser_command(url);
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|error| error.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn open_directory_in_terminal(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("tool_directory_not_found".to_string());
    }
    #[cfg(target_os = "macos")]
    let child = Command::new("open")
        .args(["-a", "Terminal"])
        .arg(path)
        .spawn();
    #[cfg(target_os = "linux")]
    let child = Command::new("x-terminal-emulator")
        .arg("--working-directory")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let child = Command::new("cmd")
        .args(["/C", "start", "cmd", "/K", "cd", "/D"])
        .arg(path)
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let child: Result<std::process::Child, std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unsupported platform",
    ));
    match child {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        Err(error) => Err(format!("terminal_open_failed:{error}")),
    }
}

async fn bind_loopback(requested_port: Option<u16>) -> Result<TcpListener, String> {
    let explicitly_requested = requested_port.is_some();
    let ports = match requested_port {
        Some(port) => vec![port],
        None => {
            let offset = (now_ms() % u128::from(PORT_END - PORT_START + 1)) as u16;
            (0..=PORT_END - PORT_START)
                .map(|index| PORT_START + ((offset + index) % (PORT_END - PORT_START + 1)))
                .collect()
        }
    };
    for port in ports {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        if let Ok(listener) = TcpListener::bind(address).await {
            return Ok(listener);
        }
    }
    if explicitly_requested {
        Err("requested_port_unavailable".to_string())
    } else {
        Err(format!(
            "no_available_port_in_range:{PORT_START}..={PORT_END}"
        ))
    }
}

fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn now_ms_i64() -> i64 {
    now_ms() as i64
}

fn unique_web_id(prefix: &str) -> String {
    let sequence = NEXT_WEB_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{}", now_ms(), sequence)
}

fn generate_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("secure_access_token_generation_failed:{error}"))?;
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut token, "{byte:02x}");
    }
    Ok(token)
}

fn nonempty_text(text: String, label: &str) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        Err(format!("empty_{label}"))
    } else {
        Ok(text)
    }
}

fn print_help() {
    println!("Timem Web\n\nUsage: timem-web [options]\n\nOptions:\n  --port <n>                   loopback port in {PORT_START}..={PORT_END}; default auto-select\n  --no-open                    do not open the browser automatically\n  --space <name>               memory/audit space\n  --gateway-provider <name>    provider\n  --api-protocol <protocol>    provider wire protocol\n  --response-protocol <name>   model response protocol\n  --model <name>               model\n  --api-key <key>              API key (environment is safer)\n  --base-url <url>             provider base URL\n  --data-dir <path>            data root\n  --timeout <seconds>          provider timeout\n  --max-llm-input <n>          input context limit\n  --max-llm-output <n>         output limit\n  --bash-approval <mode>       ask|approve\n  --work-instructions <mode>   silent|ask|off\n");
}

#[cfg(test)]
#[path = "../tests/unit/web_host_tests.rs"]
mod tests;

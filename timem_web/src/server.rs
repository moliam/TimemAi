use crate::event_journal::EventJournal;
use agent_core::mcp::{McpRuntime, McpServerConfig, McpServerReport, McpStore, McpTool};
use agent_core::session_store::{
    ChatCommandDeliveryState, ChatHistoryEventKind, ChatHistoryRecord, ChatHistoryRole,
    SessionResumeNotice, SessionStore, StoredSession, StoredSessionProfile, StoredSessionState,
};
use agent_core::{
    apply_runtime_config_value, combine_additional_contexts, default_data_root,
    load_workspace_dirs_from_path, model_service_config_from_sources_allow_missing_api_key,
    runtime_config_menu_report, runtime_info_context, validate_api_key,
    work_instruction_load_report, work_instruction_load_request,
    work_instruction_mode_from_sources, AgentCore, BashApprovalMode, CoreSessionWorkerEvent,
    CoreSessionWorkerManager, CoreSessionWorkerWorkspace, HostDecision, HostDecisionRequest,
    ModelServiceConfig, ModelServiceConfigSource, ResponseProtocolKind, RuntimeDataLayout,
    SessionToolRepo, ToolDetail, ToolGenRequest, ToolSummary, TopicReply, WorkInstructionLoadMode,
    CORE_TOPIC_TOOLGEN, CORE_TOPIC_USER_APPROVAL_REQUEST, CORE_TOPIC_WORK_INSTRUCTION_LOAD,
};
use agent_core::{capability::CapabilityRegistry, self_tool::SelfToolPaths};
use axum::{
    extract::DefaultBodyLimit,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Multipart, Query, State,
    },
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    ffi::OsString,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc as tokio_mpsc},
    time::sleep,
};

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
const BROWSER_COMMAND_QUEUE_CAPACITY: usize = 32;
const COMMAND_DEDUP_CAPACITY: usize = 4_096;
const MAX_COMMAND_DEDUP_RESULT_BYTES: usize = 64 * 1024;
const MAX_COMMAND_ID_BYTES: usize = 256;
const WORK_INSTRUCTION_DECISION_TIMEOUT: Duration = Duration::from_secs(30);
static NEXT_WEB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct AppState {
    token: String,
    public_access: bool,
    manager: Arc<Mutex<CoreSessionWorkerManager>>,
    template: Arc<WorkerTemplate>,
    mem: Arc<Mutex<WebMemState>>,
    events: broadcast::Sender<WireEvent>,
    sessions: Arc<Mutex<BTreeMap<String, WebSession>>>,
    command_dedup: Arc<Mutex<CommandDedupCache>>,
    event_journal: Arc<Mutex<EventJournal>>,
    command_lanes: Arc<Mutex<HashMap<String, Arc<TicketCommandLane>>>>,
    command_global_barrier: Arc<RwLock<()>>,
    mem_epoch: Arc<RwLock<u64>>,
}

#[derive(Debug, Default)]
struct TicketCommandLane {
    state: Mutex<TicketCommandLaneState>,
    ready: std::sync::Condvar,
}

#[derive(Debug, Default)]
struct TicketCommandLaneState {
    next_ticket: u64,
    serving_ticket: u64,
    skipped_tickets: BTreeSet<u64>,
    active: bool,
}

impl TicketCommandLane {
    fn issue(&self) -> Result<u64, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "command_lane_poisoned".to_string())?;
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| "command_lane_ticket_exhausted".to_string())?;
        Ok(ticket)
    }

    fn enter(&self, ticket: u64) -> Result<TicketCommandLaneGuard<'_>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "command_lane_poisoned".to_string())?;
        while state.serving_ticket != ticket {
            state = self
                .ready
                .wait(state)
                .map_err(|_| "command_lane_poisoned".to_string())?;
        }
        state.active = true;
        Ok(TicketCommandLaneGuard { lane: self })
    }

    fn skip(&self, ticket: u64) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "command_lane_poisoned".to_string())?;
        state.skipped_tickets.insert(ticket);
        if !state.active {
            skip_cancelled_tickets(&mut state);
        }
        self.ready.notify_all();
        Ok(())
    }
}

fn advance_ticket_lane(state: &mut TicketCommandLaneState) {
    state.active = false;
    state.serving_ticket = state.serving_ticket.saturating_add(1);
    skip_cancelled_tickets(state);
}

fn skip_cancelled_tickets(state: &mut TicketCommandLaneState) {
    while state.skipped_tickets.remove(&state.serving_ticket) {
        state.serving_ticket = state.serving_ticket.saturating_add(1);
    }
}

struct TicketCommandLaneGuard<'a> {
    lane: &'a TicketCommandLane,
}

impl Drop for TicketCommandLaneGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lane.state.lock() {
            advance_ticket_lane(&mut state);
            self.lane.ready.notify_all();
        }
    }
}

#[derive(Debug, Clone)]
enum CommandDedupState {
    Accepted,
    Committed {
        event: Option<WireEvent>,
        serialized_event: Option<Value>,
    },
    Rejected {
        error: String,
    },
}

#[derive(Debug, Default)]
struct CommandDedupCache {
    records: HashMap<String, CommandDedupState>,
    insertion_order: VecDeque<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedCommandDedup {
    records: Vec<PersistedCommandDedupRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedCommandDedupRecord {
    command_id: String,
    status: PersistedCommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PersistedCommandStatus {
    Accepted,
    Committed,
    Rejected,
}

impl CommandDedupCache {
    fn reserve(&mut self, command_id: &str) -> Option<CommandDedupState> {
        if let Some(existing) = self.records.get(command_id) {
            return Some(existing.clone());
        }
        while self.records.len() >= COMMAND_DEDUP_CAPACITY {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            // An accepted command must remain reserved until it reaches a terminal state.
            if matches!(self.records.get(&oldest), Some(CommandDedupState::Accepted)) {
                self.insertion_order.push_back(oldest);
                if self
                    .insertion_order
                    .iter()
                    .all(|id| matches!(self.records.get(id), Some(CommandDedupState::Accepted)))
                {
                    break;
                }
                continue;
            }
            self.records.remove(&oldest);
        }
        self.records
            .insert(command_id.to_string(), CommandDedupState::Accepted);
        self.insertion_order.push_back(command_id.to_string());
        None
    }

    fn finish(&mut self, command_id: &str, state: CommandDedupState) {
        self.records.insert(command_id.to_string(), state);
    }

    fn unreserve(&mut self, command_id: &str) {
        self.records.remove(command_id);
        self.insertion_order.retain(|id| id != command_id);
    }

    fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            std::fs::read(path).map_err(|error| format!("command_dedup_read_failed:{error}"))?;
        let persisted: PersistedCommandDedup = serde_json::from_slice(&raw)
            .map_err(|error| format!("command_dedup_parse_failed:{error}"))?;
        let mut cache = Self::default();
        for record in persisted
            .records
            .into_iter()
            .rev()
            .take(COMMAND_DEDUP_CAPACITY)
            .rev()
        {
            let state = match record.status {
                // Accepted is deliberately retained as uncertain. Re-executing
                // after a crash can duplicate a non-idempotent domain effect;
                // a command-specific reconciler may later prove committed or
                // rejected, but generic recovery must not guess.
                PersistedCommandStatus::Accepted => CommandDedupState::Accepted,
                PersistedCommandStatus::Committed => CommandDedupState::Committed {
                    event: None,
                    serialized_event: record.result,
                },
                PersistedCommandStatus::Rejected => CommandDedupState::Rejected {
                    error: record
                        .error
                        .unwrap_or_else(|| "command_rejected".to_string()),
                },
            };
            cache.insertion_order.push_back(record.command_id.clone());
            cache.records.insert(record.command_id, state);
        }
        Ok(cache)
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("command_dedup_dir_failed:{error}"))?;
        }
        let records = self
            .insertion_order
            .iter()
            .filter_map(|command_id| {
                self.records.get(command_id).map(|state| {
                    let (status, error, result) = match state {
                        CommandDedupState::Accepted => {
                            (PersistedCommandStatus::Accepted, None, None)
                        }
                        CommandDedupState::Committed {
                            event: _,
                            serialized_event,
                        } => (
                            PersistedCommandStatus::Committed,
                            None,
                            serialized_event.clone(),
                        ),
                        CommandDedupState::Rejected { error } => {
                            (PersistedCommandStatus::Rejected, Some(error.clone()), None)
                        }
                    };
                    PersistedCommandDedupRecord {
                        command_id: command_id.clone(),
                        status,
                        error,
                        result,
                    }
                })
            })
            .collect();
        let raw = serde_json::to_vec(&PersistedCommandDedup { records })
            .map_err(|error| format!("command_dedup_serialize_failed:{error}"))?;
        let temporary = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("command_dedup_open_failed:{error}"))?;
        file.write_all(&raw)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("command_dedup_write_failed:{error}"))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("command_dedup_replace_failed:{error}"))
    }
}

fn load_command_dedup_resilient(path: &Path) -> Result<CommandDedupCache, String> {
    match CommandDedupCache::load(path) {
        Ok(cache) => Ok(cache),
        Err(error) if error.starts_with("command_dedup_parse_failed:") => {
            let replacement = serde_json::to_vec(&PersistedCommandDedup {
                records: Vec::new(),
            })
            .map_err(|serialize_error| {
                format!("command_dedup_recovery_serialize_failed:{serialize_error}")
            })?;
            let backup = backup_and_replace_corrupt_state(
                path,
                &replacement,
                "command-dedup-corrupt-backup",
            )?;
            eprintln!(
                "[timem_web_warning] command_dedup_corruption_quarantined error={error} backup={}",
                backup.display()
            );
            CommandDedupCache::load(path)
        }
        Err(error) => Err(error),
    }
}

#[derive(Clone)]
struct WorkerTemplate {
    settings: Arc<Mutex<RuntimeSettings>>,
    data_dir: PathBuf,
    initial_space: String,
    env: BTreeMap<String, String>,
    current_dir: PathBuf,
    workspace_dirs: Vec<PathBuf>,
    reminder_tips_config: agent_core::ReminderTipsConfig,
}

#[derive(Debug, Clone)]
struct WebMemState {
    space: String,
    layout: RuntimeDataLayout,
    session_store: SessionStore,
    mcp_store: McpStore,
    mcp_runtime: McpRuntime,
    mcp_configs: Vec<McpServerConfig>,
    mcp_reports: BTreeMap<String, McpServerReport>,
}

impl WebMemState {
    fn new(data_dir: PathBuf, space: String) -> Result<Self, String> {
        validate_web_space_name(&space)?;
        let layout = RuntimeDataLayout::new(data_dir, space.clone());
        let mcp_store = McpStore::new(layout.memory_dir());
        let mcp_configs = load_mcp_configs_resilient(&mcp_store)?;
        Ok(Self {
            space,
            session_store: SessionStore::new(layout.memory_dir()),
            mcp_store,
            mcp_runtime: McpRuntime::default(),
            mcp_configs,
            mcp_reports: BTreeMap::new(),
            layout,
        })
    }

    fn from_directory(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = validate_web_mem_directory(path.as_ref())?;
        let space = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "mem_path_invalid".to_string())?
            .to_string();
        let data_dir = path
            .parent()
            .ok_or_else(|| "mem_path_invalid".to_string())?
            .to_path_buf();
        Self::new(data_dir, space)
    }

    fn info(&self) -> WebMemInfo {
        WebMemInfo {
            space: self.space.clone(),
            data_dir: absolute_path(self.layout.data_root()).display().to_string(),
            space_dir: absolute_path(self.layout.space_dir()).display().to_string(),
            memory_dir: absolute_path(self.layout.memory_dir())
                .display()
                .to_string(),
        }
    }
}

fn load_mcp_configs_resilient(mcp_store: &McpStore) -> Result<Vec<McpServerConfig>, String> {
    match mcp_store.list() {
        Ok(configs) => Ok(configs),
        Err(error) if error.starts_with("mcp_store_parse_failed:") => {
            let backup = backup_and_replace_corrupt_state(
                mcp_store.file(),
                b"[]\n",
                "mcp-config-corrupt-backup",
            )?;
            eprintln!(
                "[timem_web_warning] mcp_config_corruption_quarantined error={error} backup={}",
                backup.display()
            );
            mcp_store.list()
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone)]
struct RuntimeSettings {
    config: ModelServiceConfig,
    bash_approval_mode: BashApprovalMode,
    work_instruction_mode: WorkInstructionLoadMode,
    max_rounds: u32,
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
    mcp_server_ids: Vec<String>,
    #[serde(skip)]
    mcp_config_revision: u64,
    #[serde(skip)]
    applied_mcp_config_revision: u64,
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
    /// A durable Host intent that has not yet emitted Core TurnStarted.
    /// This is routing metadata, not live working state.
    #[serde(skip)]
    pending_turn_id: Option<String>,
    #[serde(skip)]
    pending_completion_message_id: Option<String>,
    #[serde(skip)]
    pending_unconsumed_supplements: Vec<String>,
    #[serde(skip)]
    reported_session_working_worker_count: Option<usize>,
    #[serde(skip)]
    work_instruction_mode: WorkInstructionLoadMode,
    #[serde(skip)]
    work_instruction_allowed: Option<bool>,
    #[serde(skip)]
    pending_work_instruction_turn: Option<PendingWorkInstructionTurn>,
    #[serde(skip)]
    runtime: WebSessionRuntime,
}

fn initial_mcp_revisions(server_ids: &[String]) -> (u64, u64) {
    if server_ids.is_empty() {
        (0, 0)
    } else {
        // MCP discovery may involve a slow or unavailable external process.
        // Keep the desired selection pending until the next turn boundary.
        (1, 0)
    }
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
    model: String,
    api_protocol: String,
    response_protocol: String,
    base_url: String,
    timeout_secs: u64,
    max_llm_input_tokens: u32,
    max_llm_output_tokens: u32,
    max_rounds: String,
    bash_approval: String,
    work_instructions: String,
    api_key_configured: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery_state: Option<ChatCommandDeliveryState>,
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
    command_id: Option<String>,
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
        event_cursor: u64,
        event_replay_floor: u64,
    },
    SemanticEvent {
        event_seq: u64,
        event: Value,
    },
    SessionCreated {
        session: WebSession,
    },
    SessionRenamed {
        session_id: String,
        display_name: String,
    },
    SessionDeleted {
        session_id: String,
    },
    ChatMessageDeleted {
        session_id: String,
        turn_id: String,
        role: String,
        role_index: usize,
    },
    SessionRuntimeUpdated {
        session_id: String,
        runtime_profile: WebSessionRuntimeProfile,
    },
    SessionRuntimeConfigUpdated {
        session_id: String,
        key: String,
        value: String,
        runtime_profile: WebSessionRuntimeProfile,
    },
    SessionApiKeyRevealed {
        session_id: String,
        api_key: String,
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
    TurnStarted {
        session_id: String,
        context_id: String,
        worker_id: String,
        turn: WebTurn,
    },
    TurnUpdated {
        session_id: String,
        turn: WebTurn,
    },
    HostError {
        message: String,
    },
    CommandAck {
        command_id: String,
        status: CommandAckStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
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
    McpUpdated {
        session_id: Option<String>,
        servers: Vec<McpServerReport>,
        enabled_server_ids: Vec<String>,
    },
    McpServerSecretsRevealed {
        server_id: String,
        values: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommandAckStatus {
    Accepted,
    Committed,
    Rejected,
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
    bind_host: String,
    public_access: bool,
    mem: WebMemInfo,
    runtime_options: Vec<WebRuntimeOption>,
    session_env_defaults: BTreeMap<String, String>,
    workspace_dirs: Vec<String>,
    mcp_servers: Vec<McpServerReport>,
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
    last_event_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UploadQuery {
    token: Option<String>,
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct BrowserCommand {
    #[serde(default)]
    command_id: Option<String>,
    #[serde(skip)]
    accepted_mem_epoch: u64,
    #[serde(skip)]
    accepted_lane: Option<AcceptedCommandLane>,
    #[serde(flatten)]
    command: ClientCommand,
}

#[derive(Debug)]
struct AcceptedCommandLane {
    key: String,
    lane: Arc<TicketCommandLane>,
    lanes: Arc<Mutex<HashMap<String, Arc<TicketCommandLane>>>>,
    ticket: u64,
}

impl Drop for AcceptedCommandLane {
    fn drop(&mut self) {
        let Ok(mut lanes) = self.lanes.lock() else {
            return;
        };
        let Some(mapped) = lanes.get(&self.key) else {
            return;
        };
        if !Arc::ptr_eq(mapped, &self.lane) || Arc::strong_count(&self.lane) != 2 {
            return;
        }
        let idle = self
            .lane
            .state
            .lock()
            .map(|state| {
                !state.active
                    && state.serving_ticket == state.next_ticket
                    && state.skipped_tickets.is_empty()
            })
            .unwrap_or(false);
        if idle {
            lanes.remove(&self.key);
        }
    }
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
    SessionApiKeyUpdate {
        session_id: String,
        api_key: String,
    },
    SessionApiKeyReveal {
        session_id: String,
    },
    SessionStop {
        session_id: String,
    },
    SessionDelete {
        session_id: String,
    },
    ChatMessageDelete {
        session_id: String,
        turn_id: String,
        role: String,
        role_index: usize,
    },
    TurnSubmit {
        session_id: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        attachment_ids: Option<Vec<String>>,
        input_kind: Option<String>,
        source_turn_id: Option<String>,
    },
    TurnSupplement {
        session_id: String,
        text: String,
        #[serde(default)]
        attachment_ids: Option<Vec<String>>,
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
    SessionRuntimeUpdate {
        session_id: String,
        key: String,
        value: String,
    },
    McpServerUpsert {
        session_id: String,
        config: McpServerConfig,
    },
    McpServerDelete {
        server_id: String,
    },
    McpSessionToggle {
        session_id: String,
        server_id: String,
        enabled: bool,
    },
    McpServerReconnect {
        session_id: String,
        server_id: String,
    },
    McpServerSecretsReveal {
        server_id: String,
    },
    MemSwitch {
        #[serde(alias = "space")]
        path: String,
    },
}

impl ClientCommand {
    fn mutation_lane(&self) -> Option<String> {
        match self {
            Self::HistoryPage { .. }
            | Self::ToolRepoSearch { .. }
            | Self::ToolRepoDetail { .. }
            | Self::SessionApiKeyReveal { .. }
            | Self::McpServerSecretsReveal { .. } => None,
            Self::RuntimeUpdate { .. } | Self::MemSwitch { .. } | Self::McpServerDelete { .. } => {
                Some("global".to_string())
            }
            Self::SessionCreate { .. } => Some("session:create".to_string()),
            Self::McpServerUpsert { config, .. } => Some(format!("mcp:{}", config.id)),
            Self::SessionRename { session_id, .. }
            | Self::SessionApiKeyUpdate { session_id, .. }
            | Self::SessionStop { session_id }
            | Self::SessionDelete { session_id }
            | Self::ChatMessageDelete { session_id, .. }
            | Self::TurnSubmit { session_id, .. }
            | Self::TurnSupplement { session_id, .. }
            | Self::TurnCancel { session_id }
            | Self::AttachmentRemove { session_id, .. }
            | Self::ToolRepoRename { session_id, .. }
            | Self::ToolRepoOpenTerminal { session_id, .. }
            | Self::TopicReply { session_id, .. }
            | Self::SessionRuntimeUpdate { session_id, .. }
            | Self::McpSessionToggle { session_id, .. }
            | Self::McpServerReconnect { session_id, .. } => Some(format!("session:{session_id}")),
        }
    }

    fn uses_global_mutation_barrier(&self) -> bool {
        matches!(
            self,
            Self::RuntimeUpdate { .. } | Self::MemSwitch { .. } | Self::McpServerDelete { .. }
        )
    }

    fn result_is_sensitive(&self) -> bool {
        matches!(
            self,
            Self::SessionApiKeyReveal { .. } | Self::McpServerSecretsReveal { .. }
        )
    }

    fn result_is_direct(&self) -> bool {
        matches!(
            self,
            Self::HistoryPage { .. }
                | Self::ToolRepoSearch { .. }
                | Self::ToolRepoDetail { .. }
                | Self::SessionApiKeyReveal { .. }
                | Self::McpServerSecretsReveal { .. }
        )
    }

    fn waits_for_core_acceptance(&self) -> bool {
        matches!(self, Self::TurnSubmit { .. } | Self::TurnSupplement { .. })
    }
}

pub async fn run_from_env() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }

    let launch = WebLaunchOptions::parse(&args)?;
    let template = WorkerTemplate::from_environment(&launch)?;
    println!("Starting Timem Web and restoring the selected workspace...");
    let token = generate_token()?;
    let manager = Arc::new(Mutex::new(CoreSessionWorkerManager::new()));
    let sessions = Arc::new(Mutex::new(BTreeMap::new()));
    let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
    let initial_mem = WebMemState::new(template.data_dir.clone(), template.initial_space.clone())?;
    let command_dedup = load_command_dedup_resilient(&command_dedup_path(&initial_mem))?;
    let event_journal = EventJournal::open(event_journal_path(&initial_mem)).map_err(|error| {
        friendly_journal_error(error, &template.data_dir, &template.initial_space)
    })?;
    let mem = Arc::new(Mutex::new(initial_mem));
    let state = AppState {
        token: token.clone(),
        public_access: launch.public_access,
        manager,
        template: Arc::new(template),
        mem,
        events,
        sessions,
        command_dedup: Arc::new(Mutex::new(command_dedup)),
        event_journal: Arc::new(Mutex::new(event_journal)),
        command_lanes: Arc::new(Mutex::new(HashMap::new())),
        command_global_barrier: Arc::new(RwLock::new(())),
        mem_epoch: Arc::new(RwLock::new(1)),
    };

    let restored_sessions = restore_stored_sessions(&state).map_err(|error| {
        friendly_memory_space_error(
            error,
            &state.template.data_dir,
            &state.template.initial_space,
        )
    })?;
    if restored_sessions == 0 {
        let default_session = create_session(&state, None, None, BTreeMap::new())?;
        let _ = default_session;
    }
    spawn_event_bridge(state.clone());

    let listener = bind_web_listener(launch.port, launch.public_access)
        .await
        .map_err(|error| friendly_bind_error(error, launch.port))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let app = build_router(state.clone(), port);
    let local_url = format!("http://127.0.0.1:{port}/?token={token}");
    if launch.public_access {
        if let Some(public_url) = public_access_url(launch.public_host.as_deref(), port, &token) {
            println!("Timem Web is ready at {public_url}");
        } else {
            println!("Timem Web is ready at {local_url}");
            println!(
                "Could not detect a reachable server address. Set TIMEM_PUBLIC_HOST or pass --public-host <host> to get a remote browser URL."
            );
        }
        println!(
            "Public mode is enabled. Browser, API, upload, and WebSocket access require the token above."
        );
        println!("Local access: {local_url}");
    } else {
        println!("Timem Web is ready at {local_url}");
    }
    let _ = schedule_selected_session_mcp_refreshes(&state);
    if launch.open_browser && !launch.public_access {
        if should_auto_open_browser() {
            if let Err(error) = open_browser(&local_url) {
                eprintln!("Could not open the browser automatically: {error}");
                eprintln!("Open this URL manually: {local_url}");
            }
        } else {
            println!(
                "[INFO] No local graphical session detected; browser auto-open skipped. Open: {local_url}"
            );
        }
    }
    println!(
        "The server is bound to {}. Stop with {}.",
        web_bind_host(launch.public_access),
        web_shutdown_signal_names().join("/")
    );
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| error.to_string());
    let shutdown_result = shutdown_web_runtime(&state);
    serve_result?;
    shutdown_result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        shutdown_signal_unix().await;
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn web_shutdown_signal_names() -> &'static [&'static str] {
    #[cfg(unix)]
    {
        &["Ctrl+C", "SIGTERM", "SIGHUP"]
    }
    #[cfg(not(unix))]
    {
        &["Ctrl+C"]
    }
}

#[cfg(unix)]
async fn shutdown_signal_unix() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = signal(SignalKind::terminate()).ok();
    let mut hangup = signal(SignalKind::hangup()).ok();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = recv_optional_signal(&mut terminate) => {},
        _ = recv_optional_signal(&mut hangup) => {},
    }
}

#[cfg(unix)]
async fn recv_optional_signal(stream: &mut Option<tokio::signal::unix::Signal>) {
    if let Some(stream) = stream.as_mut() {
        let _ = stream.recv().await;
    } else {
        std::future::pending::<()>().await;
    }
}

fn shutdown_web_runtime(state: &AppState) -> Result<(), String> {
    let manager = {
        let mut manager = state
            .manager
            .lock()
            .map_err(|_| "worker_manager_poisoned".to_string())?;
        std::mem::take(&mut *manager)
    };
    manager.shutdown_all_detached()
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
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:; font-src 'self'; form-action 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
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
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if !authorized_api_request(&state, query.token.as_deref(), &headers) {
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
        publish_semantic(
            &state,
            WireEvent::FileUploaded {
                session_id: query.session_id,
                file: attachment.clone(),
            },
        )?;
        Ok::<_, String>(attachment)
    }
    .await;
    match result {
        Ok(file) => Json(file).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response(),
    }
}

async fn static_asset(
    State((state, _)): State<(AppState, u16)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let token_from_query = query.token.as_deref() == Some(state.token.as_str());
    if !authorized_token_or_cookie(&state, query.token.as_deref(), &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"))],
            "Timem Web requires the full authenticated URL printed at startup, including ?token=... .\n",
        )
            .into_response();
    }
    let path = match uri.path() {
        "/" => "/index.html",
        path => path,
    };
    let (asset_path, content_type) = match embedded_web_asset(path) {
        Some(_) => (path, mime_for_path(path)),
        None => ("/index.html", "text/html; charset=utf-8"),
    };
    let body = embedded_web_asset(asset_path).expect("embedded index asset must exist");
    let mut response = (
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
        .into_response();
    if token_from_query {
        if let Ok(cookie) = HeaderValue::from_str(&format!(
            "timem_web_token={}; Path=/; SameSite=Strict; HttpOnly",
            state.token
        )) {
            response.headers_mut().insert(header::SET_COOKIE, cookie);
        }
    }
    response
}

fn authorized_by_cookie(state: &AppState, headers: &HeaderMap) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookie| {
            cookie
                .split(';')
                .map(str::trim)
                .any(|part| part == format!("timem_web_token={}", state.token))
        })
        .unwrap_or(false)
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
    headers: HeaderMap,
) -> Response {
    if !authorized_api_request(&state, auth.token.as_deref(), &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({ "ok": true, "port": port })).into_response()
}

async fn snapshot(
    State((state, port)): State<(AppState, u16)>,
    Query(auth): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized_api_request(&state, auth.token.as_deref(), &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(snapshot_for(&state, port)).into_response()
}

async fn websocket(
    ws: WebSocketUpgrade,
    State((state, port)): State<(AppState, u16)>,
    Query(auth): Query<AuthQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized_api_request(&state, auth.token.as_deref(), &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let last_event_seq = auth.last_event_seq;
    ws.on_upgrade(move |socket| websocket_session(socket, state, port, last_event_seq))
}

#[cfg(test)]
fn authorized(state: &AppState, auth: &AuthQuery, headers: &HeaderMap) -> bool {
    authorized_token_or_cookie(state, auth.token.as_deref(), headers)
}

fn authorized_token_or_cookie(state: &AppState, token: Option<&str>, headers: &HeaderMap) -> bool {
    match token {
        Some(token) => token == state.token,
        None => authorized_by_cookie(state, headers),
    }
}

fn authorized_api_request(state: &AppState, token: Option<&str>, headers: &HeaderMap) -> bool {
    authorized_token_or_cookie(state, token, headers) && request_origin_allowed(headers)
}

fn request_origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin_authority(origin).is_some_and(|authority| authority.eq_ignore_ascii_case(host.trim()))
}

fn origin_authority(origin: &str) -> Option<&str> {
    let origin = origin.trim();
    let scheme_end = origin.find("://")?;
    let after_scheme = &origin[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    (!authority.is_empty()).then_some(authority)
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

fn command_dedup_path(mem: &WebMemState) -> PathBuf {
    mem.layout.memory_dir().join("web_command_dedup.json")
}

fn friendly_journal_error(error: String, data_dir: &std::path::Path, space: &str) -> String {
    if error == "event_journal_in_use" {
        let space_dir = absolute_path(RuntimeDataLayout::new(data_dir, space).space_dir());
        format!(
            "Timem Web is already running on this memory space.\n\n  data dir: {}\n  space:    {}\n  location:  {}\n\nOptions:\n  - Use a different space:   timem-web --space <name>\n  - Use a different data dir: timem-web --data-dir <path>\n  - Or stop the other Timem Web instance first.",
            data_dir.display(),
            space,
            space_dir.display(),
        )
    } else {
        error
    }
}

fn friendly_memory_space_error(error: String, data_dir: &std::path::Path, space: &str) -> String {
    if error == "mem_guard_timeout" {
        let space_dir = absolute_path(RuntimeDataLayout::new(data_dir, space).space_dir());
        format!(
            "The selected Timem workspace is still locked by another running operation.\n\n  data dir: {}\n  space:    {}\n  location: {}\n\nTimem automatically recovers locks left by processes that have exited. If this message persists, another Timem process is still using this workspace. Close that process and retry, or start with a different workspace:\n\n  cargo run -p timem_web -- --space <another-name>\n\nYou can also select another data directory with --data-dir <path>. Do not delete the lock while another Timem process is running.",
            data_dir.display(),
            space,
            space_dir.display(),
        )
    } else {
        friendly_journal_error(error, data_dir, space)
    }
}

fn friendly_bind_error(error: String, requested_port: Option<u16>) -> String {
    match (error.as_str(), requested_port) {
        ("requested_port_unavailable", Some(port)) => format!(
            "Port {port} is already in use or cannot be opened. Stop the process using it, choose another port, or let Timem select one automatically:\n\n  cargo run -p timem_web\n  cargo run -p timem_web -- --port 18080"
        ),
        (error, _) if error.starts_with("no_available_port_in_range:") => format!(
            "Timem could not open a local web port in the supported range {PORT_START}–{PORT_END}. Check whether local-network access is blocked by a firewall or sandbox, then retry with an explicit port:\n\n  cargo run -p timem_web -- --port 18080\n\nIf another Timem Web process is running, close it first."
        ),
        _ => error,
    }
}

fn event_journal_path(mem: &WebMemState) -> PathBuf {
    mem.layout.memory_dir().join("web_events.ndjson")
}

fn current_command_dedup_path(state: &AppState) -> Result<PathBuf, String> {
    Ok(command_dedup_path(&current_mem_state(state)?))
}

fn session_tool_repo(state: &AppState, session_id: &str) -> Result<SessionToolRepo, String> {
    let mem = current_mem_state(state)?;
    Ok(SessionToolRepo::new(mem.layout.memory_dir(), session_id))
}

async fn websocket_session(
    socket: WebSocket,
    state: AppState,
    port: u16,
    last_event_seq: Option<u64>,
) {
    let (mut sender, mut receiver) = socket.split();
    // Subscribe before taking the snapshot. Events produced after the snapshot
    // is captured are then buffered instead of falling into a subscribe gap.
    let mut events = state.events.subscribe();
    let (event_cursor, event_replay_floor) = state
        .event_journal
        .lock()
        .map(|journal| (journal.cursor(), journal.replay_floor()))
        .unwrap_or_default();
    if send_event(
        &mut sender,
        &WireEvent::Hello {
            snapshot: snapshot_for(&state, port),
            event_cursor,
            event_replay_floor,
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let replay_after = last_event_seq
        .filter(|cursor| *cursor >= event_replay_floor && *cursor <= event_cursor)
        .unwrap_or(event_cursor);
    let mut last_sent_event_seq = replay_after;
    let replay = state
        .event_journal
        .lock()
        .map_err(|_| "event_journal_poisoned".to_string())
        .and_then(|journal| journal.replay_after(replay_after));
    match replay {
        Ok(replay) => {
            for entry in replay {
                let event_seq = entry.event_seq;
                if send_event(
                    &mut sender,
                    &WireEvent::SemanticEvent {
                        event_seq: entry.event_seq,
                        event: entry.event,
                    },
                )
                .await
                .is_err()
                {
                    return;
                }
                last_sent_event_seq = event_seq;
            }
        }
        Err(error) => {
            if send_event(&mut sender, &WireEvent::HostError { message: error })
                .await
                .is_err()
            {
                return;
            }
        }
    }
    let (command_tx, command_rx) =
        tokio_mpsc::channel::<BrowserCommand>(BROWSER_COMMAND_QUEUE_CAPACITY);
    let (command_result_tx, mut command_result_rx) = tokio_mpsc::unbounded_channel();
    let command_state = state.clone();
    let command_worker = tokio::spawn(run_ordered_blocking_queue(
        command_rx,
        command_result_tx,
        move |command| Ok(execute_browser_command(&command_state, port, command)),
    ));
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
                        match serde_json::from_str::<BrowserCommand>(&text) {
                            Ok(mut command) => {
                                // Acceptance and mem switching share this barrier. A switch
                                // cannot advance the epoch between stamping and queueing a
                                // command, and the non-Send guard is dropped before any await.
                                let enqueue_outcome = match state.mem_epoch.read() {
                                    Err(_) => BrowserCommandEnqueueOutcome::Rejected {
                                        command_id: command.command_id.clone(),
                                        error: "mem_epoch_poisoned".to_string(),
                                    },
                                    Ok(epoch) => {
                                        command.accepted_mem_epoch = *epoch;
                                        let mut rejection = None;
                                        let mut cached = None;
                                        if let Some(command_id) = command.command_id.as_deref() {
                                            if let Err(error) = validate_command_id(command_id) {
                                                rejection = Some(error);
                                            } else {
                                                match reserve_command_dedup(&state, command_id) {
                                                    Ok(Some(CommandDedupState::Accepted))
                                                        if command.command.waits_for_core_acceptance() =>
                                                    {
                                                        // An accepted TurnSubmit is a durable
                                                        // intent, not proof of Core delivery.
                                                        // Re-drive it; Core deduplicates by ID.
                                                    }
                                                    Ok(Some(previous)) => {
                                                        cached = Some((command_id.to_string(), previous));
                                                    }
                                                    Ok(None) => {}
                                                    Err(error) => rejection = Some(error),
                                                }
                                            }
                                        }
                                        if let Some(error) = rejection {
                                            BrowserCommandEnqueueOutcome::Rejected {
                                                command_id: command.command_id.clone(),
                                                error,
                                            }
                                        } else if let Some((command_id, state)) = cached {
                                            BrowserCommandEnqueueOutcome::Cached { command_id, state }
                                        } else {
                                            enqueue_reserved_browser_command(&state, &command_tx, command)
                                        }
                                    }
                                };
                                match enqueue_outcome {
                                    BrowserCommandEnqueueOutcome::Accepted(Some(command_id)) => {
                                        if send_event(&mut sender, &command_ack(&command_id, CommandAckStatus::Accepted, None)).await.is_err() { break; }
                                    }
                                    BrowserCommandEnqueueOutcome::Accepted(None) => {}
                                    BrowserCommandEnqueueOutcome::Cached { command_id, state: cached } => {
                                        if send_cached_command_state(&mut sender, &command_id, cached).await.is_err() { break; }
                                    }
                                    BrowserCommandEnqueueOutcome::Rejected { command_id: Some(command_id), error } => {
                                        if send_event(&mut sender, &command_ack(&command_id, CommandAckStatus::Rejected, Some(error))).await.is_err() { break; }
                                    }
                                    BrowserCommandEnqueueOutcome::Rejected { command_id: None, error } => {
                                        if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() { break; }
                                    }
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
            result = command_result_rx.recv() => {
                match result {
                    Some(Ok(completion)) => {
                        if let Some(event) = completion.event {
                            if send_event(&mut sender, &event).await.is_err() { break; }
                        }
                        if let Some(ack) = completion.ack {
                            if send_event(&mut sender, &ack).await.is_err() { break; }
                        }
                        if let Some(error) = completion.legacy_error {
                            if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() { break; }
                        }
                    }
                    Some(Err(error)) => if send_event(&mut sender, &WireEvent::HostError { message: error }).await.is_err() {
                        break;
                    },
                    None => {
                        let _ = send_event(&mut sender, &WireEvent::HostError { message: "browser_command_worker_stopped".to_string() }).await;
                        break;
                    }
                }
            }
            event = events.recv() => match event {
                Ok(WireEvent::SemanticEvent { event_seq, .. }) if event_seq <= last_sent_event_seq => {}
                Ok(event) => {
                    let sent_seq = match &event {
                        WireEvent::SemanticEvent { event_seq, .. } => Some(*event_seq),
                        WireEvent::Hello { event_cursor, .. } => Some(*event_cursor),
                        _ => None,
                    };
                    if send_event(&mut sender, &event).await.is_err() { break; }
                    if let Some(sent_seq) = sent_seq {
                        last_sent_event_seq = sent_seq;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let replay = state
                        .event_journal
                        .lock()
                        .map_err(|_| "event_journal_poisoned".to_string())
                        .and_then(|journal| journal.replay_after(last_sent_event_seq));
                    match replay {
                        Ok(entries) => {
                            let mut disconnected = false;
                            for entry in entries {
                                let event_seq = entry.event_seq;
                                if send_event(
                                    &mut sender,
                                    &semantic_event_envelope(event_seq, entry.event),
                                )
                                .await
                                .is_err()
                                {
                                    disconnected = true;
                                    break;
                                }
                                last_sent_event_seq = event_seq;
                            }
                            if disconnected { break; }
                        }
                        Err(message) => {
                            if send_event(&mut sender, &WireEvent::HostError { message }).await.is_err() { break; }
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    drop(command_tx);
    // Dropping a JoinHandle detaches the worker. It must drain commands that this
    // connection already received even when the browser disconnects before ACK.
    drop(command_worker);
}

#[derive(Debug)]
struct BrowserCommandCompletion {
    event: Option<WireEvent>,
    ack: Option<WireEvent>,
    legacy_error: Option<String>,
}

enum BrowserCommandEnqueueOutcome {
    Accepted(Option<String>),
    Cached {
        command_id: String,
        state: CommandDedupState,
    },
    Rejected {
        command_id: Option<String>,
        error: String,
    },
}

fn enqueue_reserved_browser_command(
    state: &AppState,
    command_tx: &tokio_mpsc::Sender<BrowserCommand>,
    mut command: BrowserCommand,
) -> BrowserCommandEnqueueOutcome {
    if let Some((key, lane)) = command_lane(state, &command.command) {
        match lane.issue() {
            Ok(ticket) => {
                command.accepted_lane = Some(AcceptedCommandLane {
                    key,
                    lane,
                    lanes: Arc::clone(&state.command_lanes),
                    ticket,
                })
            }
            Err(error) => {
                return reject_reserved_enqueue(state, command, error);
            }
        }
    }
    let command_id = command.command_id.clone();
    match command_tx.try_send(command) {
        Ok(()) => BrowserCommandEnqueueOutcome::Accepted(command_id),
        Err(error) => {
            let (message, command) = match error {
                tokio_mpsc::error::TrySendError::Full(command) => {
                    ("browser_command_queue_full", command)
                }
                tokio_mpsc::error::TrySendError::Closed(command) => {
                    ("browser_command_worker_stopped", command)
                }
            };
            reject_reserved_enqueue(state, command, message.to_string())
        }
    }
}

fn reject_reserved_enqueue(
    state: &AppState,
    command: BrowserCommand,
    mut error: String,
) -> BrowserCommandEnqueueOutcome {
    if let Some(accepted) = command.accepted_lane.as_ref() {
        let _ = accepted.lane.skip(accepted.ticket);
    }
    if let Some(command_id) = command.command_id.as_deref() {
        if let Err(persist_error) = finish_command_dedup(
            state,
            command_id,
            CommandDedupState::Rejected {
                error: error.clone(),
            },
        ) {
            error = persist_error;
        }
    }
    BrowserCommandEnqueueOutcome::Rejected {
        command_id: command.command_id,
        error,
    }
}

fn execute_browser_command(
    state: &AppState,
    port: u16,
    browser_command: BrowserCommand,
) -> BrowserCommandCompletion {
    let BrowserCommand {
        command_id,
        command,
        accepted_mem_epoch,
        accepted_lane,
    } = browser_command;
    let sensitive_result = command.result_is_sensitive();
    let direct_result = command.result_is_direct();
    let waits_for_core_acceptance = command.waits_for_core_acceptance();
    let mutation_lane = command.mutation_lane();
    let _global_write_guard;
    let _global_read_guard;
    if mutation_lane.is_some() && command.uses_global_mutation_barrier() {
        _global_write_guard = match state.command_global_barrier.write() {
            Ok(guard) => Some(guard),
            Err(_) => {
                return rejected_browser_command(
                    state,
                    command_id.as_deref(),
                    "command_global_barrier_poisoned".to_string(),
                )
            }
        };
        _global_read_guard = None;
    } else if mutation_lane.is_some() {
        _global_read_guard = match state.command_global_barrier.read() {
            Ok(guard) => Some(guard),
            Err(_) => {
                return rejected_browser_command(
                    state,
                    command_id.as_deref(),
                    "command_global_barrier_poisoned".to_string(),
                )
            }
        };
        _global_write_guard = None;
    } else {
        _global_write_guard = None;
        _global_read_guard = None;
    }
    let _lane_guard = match accepted_lane
        .as_ref()
        .map(|accepted| accepted.lane.enter(accepted.ticket))
        .transpose()
    {
        Ok(guard) => guard,
        Err(error) => return rejected_browser_command(state, command_id.as_deref(), error),
    };
    let is_mem_switch = matches!(command, ClientCommand::MemSwitch { .. });
    let mem_epoch_guard = if is_mem_switch {
        None
    } else {
        state.mem_epoch.read().ok()
    };
    if !is_mem_switch
        && mem_epoch_guard
            .as_deref()
            .is_none_or(|epoch| *epoch != accepted_mem_epoch)
    {
        return rejected_browser_command(
            state,
            command_id.as_deref(),
            "command_mem_epoch_stale".to_string(),
        );
    }
    match handle_command_with_id(state, port, command_id.as_deref(), command) {
        Ok(event) => {
            let direct_event = direct_result.then(|| event.clone()).flatten();
            if let Some(command_id) = command_id.as_deref() {
                if sensitive_result {
                    if let Ok(mut cache) = state.command_dedup.lock() {
                        cache.unreserve(command_id);
                    }
                    return BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(command_id, CommandAckStatus::Committed, None)),
                        legacy_error: None,
                    };
                }
                if waits_for_core_acceptance {
                    // The user entry is durable, but the command is not terminal
                    // until Core reports that it dequeued the matching intent.
                    return BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(command_id, CommandAckStatus::Accepted, None)),
                        legacy_error: None,
                    };
                }
                match finish_command_dedup(
                    state,
                    command_id,
                    CommandDedupState::Committed {
                        serialized_event: direct_event.as_ref().and_then(durable_command_result),
                        event: direct_event.clone(),
                    },
                ) {
                    Ok(()) => BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(command_id, CommandAckStatus::Committed, None)),
                        legacy_error: None,
                    },
                    Err(error) => BrowserCommandCompletion {
                        event: direct_event,
                        ack: Some(command_ack(
                            command_id,
                            CommandAckStatus::Accepted,
                            Some(format!("command_terminal_persist_pending:{error}")),
                        )),
                        legacy_error: None,
                    },
                }
            } else {
                BrowserCommandCompletion {
                    event: direct_event,
                    ack: None,
                    legacy_error: None,
                }
            }
        }
        Err(error) => {
            if let Some(command_id) = command_id.as_deref() {
                let persist_error = finish_command_dedup(
                    state,
                    command_id,
                    CommandDedupState::Rejected {
                        error: error.clone(),
                    },
                )
                .err();
                BrowserCommandCompletion {
                    event: None,
                    ack: Some(command_ack(
                        command_id,
                        CommandAckStatus::Rejected,
                        Some(persist_error.unwrap_or(error)),
                    )),
                    legacy_error: None,
                }
            } else {
                BrowserCommandCompletion {
                    event: None,
                    ack: None,
                    legacy_error: Some(error),
                }
            }
        }
    }
}

fn rejected_browser_command(
    state: &AppState,
    command_id: Option<&str>,
    error: String,
) -> BrowserCommandCompletion {
    if let Some(command_id) = command_id {
        let persisted = finish_command_dedup(
            state,
            command_id,
            CommandDedupState::Rejected {
                error: error.clone(),
            },
        )
        .err();
        BrowserCommandCompletion {
            event: None,
            ack: Some(command_ack(
                command_id,
                CommandAckStatus::Rejected,
                Some(persisted.unwrap_or(error)),
            )),
            legacy_error: None,
        }
    } else {
        BrowserCommandCompletion {
            event: None,
            ack: None,
            legacy_error: Some(error),
        }
    }
}

fn command_lane(
    state: &AppState,
    command: &ClientCommand,
) -> Option<(String, Arc<TicketCommandLane>)> {
    command.mutation_lane().and_then(|key| {
        state.command_lanes.lock().ok().map(|mut lanes| {
            let lane = lanes.entry(key.clone()).or_default().clone();
            (key, lane)
        })
    })
}

fn durable_command_result(event: &WireEvent) -> Option<Value> {
    if matches!(
        event,
        WireEvent::SessionApiKeyRevealed { .. } | WireEvent::McpServerSecretsRevealed { .. }
    ) {
        return None;
    }
    let value = serde_json::to_value(event).ok()?;
    (serde_json::to_vec(&value).ok()?.len() <= MAX_COMMAND_DEDUP_RESULT_BYTES).then_some(value)
}

fn reserve_command_dedup(
    state: &AppState,
    command_id: &str,
) -> Result<Option<CommandDedupState>, String> {
    let path = current_command_dedup_path(state)?;
    let mut cache = state
        .command_dedup
        .lock()
        .map_err(|_| "command_dedup_poisoned".to_string())?;
    if !cache.records.contains_key(command_id)
        && cache.records.len() >= COMMAND_DEDUP_CAPACITY
        && cache
            .records
            .values()
            .all(|record| matches!(record, CommandDedupState::Accepted))
    {
        // Accepted entries are ownership records and must never be evicted to
        // make room for a click flood. Reject new ownership instead of letting
        // an all-uncertain cache grow without bound.
        return Err("command_dedup_capacity_exhausted".to_string());
    }
    let previous = cache.reserve(command_id);
    if previous.is_none() {
        if let Err(error) = cache.save(&path) {
            cache.unreserve(command_id);
            return Err(error);
        }
    }
    Ok(previous)
}

fn finish_command_dedup(
    state: &AppState,
    command_id: &str,
    terminal: CommandDedupState,
) -> Result<(), String> {
    let path = current_command_dedup_path(state)?;
    let mut cache = state
        .command_dedup
        .lock()
        .map_err(|_| "command_dedup_poisoned".to_string())?;
    cache.finish(command_id, terminal);
    cache.save(&path)
}

fn command_ack(command_id: &str, status: CommandAckStatus, error: Option<String>) -> WireEvent {
    WireEvent::CommandAck {
        command_id: command_id.to_string(),
        status,
        error,
    }
}

fn validate_command_id(command_id: &str) -> Result<(), String> {
    if command_id.is_empty() {
        return Err("command_id_empty".to_string());
    }
    if command_id.len() > MAX_COMMAND_ID_BYTES {
        return Err("command_id_too_large".to_string());
    }
    if command_id.chars().any(char::is_control) {
        return Err("command_id_invalid".to_string());
    }
    Ok(())
}

async fn send_cached_command_state(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    command_id: &str,
    state: CommandDedupState,
) -> Result<(), ()> {
    match state {
        CommandDedupState::Accepted => {
            send_event(
                sender,
                &command_ack(command_id, CommandAckStatus::Accepted, None),
            )
            .await
        }
        CommandDedupState::Committed {
            event,
            serialized_event,
        } => {
            if let Some(event) = event {
                send_event(sender, &event).await?;
            } else if let Some(event) = serialized_event {
                let text = serde_json::to_string(&event).map_err(|_| ())?;
                sender.send(Message::Text(text)).await.map_err(|_| ())?;
            }
            send_event(
                sender,
                &command_ack(command_id, CommandAckStatus::Committed, None),
            )
            .await
        }
        CommandDedupState::Rejected { error } => {
            send_event(
                sender,
                &command_ack(command_id, CommandAckStatus::Rejected, Some(error)),
            )
            .await
        }
    }
}

async fn run_ordered_blocking_queue<T, R, F>(
    mut receiver: tokio_mpsc::Receiver<T>,
    results: tokio_mpsc::UnboundedSender<Result<R, String>>,
    handler: F,
) where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Result<R, String> + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    while let Some(item) = receiver.recv().await {
        let handler = handler.clone();
        let result = tokio::task::spawn_blocking(move || handler(item))
            .await
            .unwrap_or_else(|error| Err(format!("browser_command_worker_failed:{error}")));
        // The WebSocket may disappear after accepting a command. Continue
        // draining the queue so accepted mutations are not cancelled with it.
        let _ = results.send(result);
    }
}

async fn send_event(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &WireEvent,
) -> Result<(), ()> {
    let text = serde_json::to_string(event).map_err(|_| ())?;
    sender.send(Message::Text(text)).await.map_err(|_| ())
}

fn publish_semantic(state: &AppState, event: WireEvent) -> Result<u64, String> {
    let value = serde_json::to_value(&event)
        .map_err(|error| format!("semantic_event_serialize_failed:{error}"))?;
    let entry = state
        .event_journal
        .lock()
        .map_err(|_| "event_journal_poisoned".to_string())?
        .append(value)?;
    #[cfg(not(test))]
    let _ = state
        .events
        .send(semantic_event_envelope(entry.event_seq, entry.event));
    #[cfg(test)]
    let _ = state.events.send(event);
    Ok(entry.event_seq)
}

fn semantic_event_envelope(event_seq: u64, event: Value) -> WireEvent {
    WireEvent::SemanticEvent { event_seq, event }
}

fn publish_core_semantic(state: &AppState, session_id: &str, event: WireEvent) {
    if let Err(error) = publish_semantic(state, event) {
        if let Ok(mut sessions) = state.sessions.lock() {
            if let Some(session) = sessions.get_mut(session_id) {
                session.state = "error".to_string();
            }
        }
        eprintln!("[timem_web_semantic_publish_error] session_id={session_id:?} reason={error}");
        let _ = state.events.send(WireEvent::HostError {
            message: format!("semantic_event_persist_failed:{error}"),
        });
    }
}

#[cfg(test)]
fn handle_command(
    state: &AppState,
    port: u16,
    command: ClientCommand,
) -> Result<Option<WireEvent>, String> {
    handle_command_with_id(state, port, None, command)
}

fn handle_command_with_id(
    state: &AppState,
    port: u16,
    command_id: Option<&str>,
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
                publish_semantic(state, event)?;
            }
            let event = WireEvent::SessionCreated { session };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
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
            let event = WireEvent::SessionRenamed {
                session_id,
                display_name,
            };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::SessionApiKeyUpdate {
            session_id,
            api_key,
        } => {
            let runtime_profile = update_session_api_key(state, &session_id, api_key)?;
            publish_semantic(
                state,
                WireEvent::SessionRuntimeUpdated {
                    session_id,
                    runtime_profile,
                },
            )?;
        }
        ClientCommand::SessionApiKeyReveal { session_id } => {
            return Ok(Some(WireEvent::SessionApiKeyRevealed {
                api_key: session_api_key(state, &session_id)?,
                session_id,
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
        ClientCommand::SessionDelete { session_id } => {
            let worker_ids = session_worker_ids(state, &session_id)?;
            {
                let mut manager = state
                    .manager
                    .lock()
                    .map_err(|_| "worker_manager_poisoned")?;
                for worker_id in worker_ids {
                    manager.shutdown_worker(&worker_id)?;
                }
            }
            current_session_store(state)?.delete_session(&session_id)?;
            session_tool_repo(state, &session_id)?.delete_session_data()?;
            state
                .sessions
                .lock()
                .map_err(|_| "session_store_poisoned")?
                .remove(&session_id)
                .ok_or_else(|| "session_not_found".to_string())?;
            let event = WireEvent::SessionDeleted { session_id };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::ChatMessageDelete {
            session_id,
            turn_id,
            role,
            role_index,
        } => {
            delete_chat_message(state, &session_id, &turn_id, &role, role_index)?;
            let event = WireEvent::ChatMessageDeleted {
                session_id,
                turn_id,
                role,
                role_index,
            };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::TurnSubmit {
            session_id,
            text,
            attachment_ids,
            input_kind,
            source_turn_id,
        } => {
            if let Some(command_id) = command_id {
                if let Some(turn) = turn_for_command_id(state, &session_id, command_id)? {
                    redeliver_recorded_turn(
                        state,
                        &session_id,
                        command_id,
                        input_kind.as_deref(),
                        &text,
                        &turn,
                    )?;
                    return Ok(Some(WireEvent::TurnUpdated { session_id, turn }));
                }
            }
            let turn = if input_kind.as_deref() == Some("toolgen") {
                if attachment_ids.is_some() {
                    return Err("toolgen_attachments_not_supported".to_string());
                }
                submit_toolgen_turn(
                    state,
                    &session_id,
                    source_turn_id
                        .as_deref()
                        .ok_or_else(|| "toolgen_source_turn_id_required".to_string())?,
                    (!text.trim().is_empty()).then_some(text),
                    command_id,
                )?
            } else {
                if input_kind.is_some() || source_turn_id.is_some() {
                    return Err("unsupported_turn_input_kind".to_string());
                }
                let text = nonempty_text(text, "turn text")?;
                submit_or_supplement_turn(
                    state,
                    &session_id,
                    text,
                    attachment_ids.as_deref(),
                    command_id,
                )?
            };
            return Ok(Some(WireEvent::TurnUpdated { session_id, turn }));
        }
        ClientCommand::TurnSupplement {
            session_id,
            text,
            attachment_ids,
        } => {
            if let Some(command_id) = command_id {
                if let Some(turn) = turn_for_command_id(state, &session_id, command_id)? {
                    return Ok(Some(WireEvent::TurnUpdated { session_id, turn }));
                }
            }
            let text = nonempty_text(text, "supplement")?;
            let turn = append_supplement_or_submit_turn(
                state,
                &session_id,
                text,
                attachment_ids.as_deref(),
                command_id,
            )?;
            let event = WireEvent::TurnUpdated { session_id, turn };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
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
            let event = WireEvent::AttachmentRemoved {
                session_id,
                attachment_id,
            };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
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
            let event = WireEvent::ToolRepoUpdated { session_id, tools };
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
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
            let always_allow = decision == "always_allow";
            let decision = match decision.as_str() {
                "accept" | "always_allow" => HostDecision::Accept,
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
                    let event = WireEvent::TurnUpdated { session_id, turn };
                    publish_semantic(state, event.clone())?;
                    return Ok(Some(event));
                }
                return Ok(None);
            }
            if !session_has_active_turn(state, &session_id)? {
                return Ok(None);
            }
            let is_user_approval = topic_name == CORE_TOPIC_USER_APPROVAL_REQUEST;
            let mut reply = TopicReply::new(session_id.clone(), topic_name, decision, payload);
            if let Some(request_id) = request_id {
                reply = reply.with_request_id(request_id);
            }
            if always_allow && is_user_approval {
                reply = reply.with_always_allow();
            }
            relay_topic_reply_to_requesting_worker(
                state,
                &session_id,
                worker_id.as_deref(),
                reply,
            )?;
            if always_allow && is_user_approval {
                switch_session_bash_approval(state, &session_id, BashApprovalMode::Approve)?;
            }
            return match append_turn_user_entry(state, &session_id, "approval", approval_summary) {
                Ok(turn) => {
                    let event = WireEvent::TurnUpdated { session_id, turn };
                    publish_semantic(state, event.clone())?;
                    Ok(Some(event))
                }
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
            publish_semantic(
                state,
                WireEvent::HostConfigUpdated {
                    key: report.key.to_string(),
                    value: report.value.clone(),
                    session_env_defaults,
                },
            )?;
            // Propagate config change to all active sessions
            let field = runtime_config_field_from_key(&key)?;
            propagate_runtime_config_to_sessions(state, field, &report.value);
        }
        ClientCommand::SessionRuntimeUpdate {
            session_id,
            key,
            value,
        } => {
            let value = nonempty_text(value, "runtime config value")?;
            let (value, runtime_profile) =
                update_session_runtime_setting(state, &session_id, &key, &value)?;
            publish_semantic(
                state,
                WireEvent::SessionRuntimeConfigUpdated {
                    session_id,
                    key,
                    value,
                    runtime_profile,
                },
            )?;
        }
        ClientCommand::McpServerUpsert { session_id, config } => {
            let server_id = config.id.clone();
            upsert_mcp_server(state, config)?;
            enable_mcp_for_session(state, &session_id, true, Some(&server_id))?;
            let _ = mark_sessions_using_mcp_server(state, &server_id)?;
            schedule_mcp_server_refresh(state, &server_id)?;
            persist_web_session(state, &session_id)?;
            let event = mcp_updated_event(state, Some(session_id))?;
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::McpServerDelete { server_id } => {
            delete_mcp_server(state, &server_id)?;
            let event = mcp_updated_event(state, None)?;
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::McpSessionToggle {
            session_id,
            server_id,
            enabled,
        } => {
            enable_mcp_for_session(state, &session_id, enabled, Some(&server_id))?;
            if enabled {
                schedule_mcp_server_refresh(state, &server_id)?;
            }
            persist_web_session(state, &session_id)?;
            let event = mcp_updated_event(state, Some(session_id))?;
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::McpServerReconnect {
            session_id,
            server_id,
        } => {
            mark_session_mcp_changed(state, &session_id)?;
            state
                .mem
                .lock()
                .map_err(|_| "mem_state_poisoned".to_string())?
                .mcp_runtime
                .disconnect(&server_id);
            schedule_mcp_server_refresh(state, &server_id)?;
            let event = mcp_updated_event(state, Some(session_id))?;
            publish_semantic(state, event.clone())?;
            return Ok(Some(event));
        }
        ClientCommand::McpServerSecretsReveal { server_id } => {
            return Ok(Some(WireEvent::McpServerSecretsRevealed {
                values: mcp_server_secret_values(state, &server_id)?,
                server_id,
            }));
        }
        ClientCommand::MemSwitch { path } => {
            let mut epoch = state
                .mem_epoch
                .write()
                .map_err(|_| "mem_epoch_poisoned".to_string())?;
            let snapshot = switch_mem_space(state, port, &path)?;
            *epoch = epoch.saturating_add(1);
            let (event_cursor, event_replay_floor) = state
                .event_journal
                .lock()
                .map(|journal| (journal.cursor(), journal.replay_floor()))
                .unwrap_or_default();
            let _ = state.events.send(WireEvent::Hello {
                snapshot,
                event_cursor,
                event_replay_floor,
            });
        }
    }
    Ok(None)
}

fn switch_mem_space(state: &AppState, port: u16, path: &str) -> Result<WebSnapshot, String> {
    if state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .values()
        .any(|session| {
            session.active_turn_id.is_some()
                || session.pending_turn_id.is_some()
                || session.state == "working"
                || !session.pending_unconsumed_supplements.is_empty()
                || session.pending_work_instruction_turn.is_some()
        })
    {
        return Err("mem_switch_active_sessions".to_string());
    }
    let requested_path = Path::new(path);
    let next_mem = if requested_path.is_absolute() {
        WebMemState::from_directory(requested_path)?
    } else {
        validate_web_space_name(path)?;
        let data_root = current_mem_state(state)?.layout.data_root().to_path_buf();
        WebMemState::new(data_root, path.to_string())?
    };
    let next_command_dedup = load_command_dedup_resilient(&command_dedup_path(&next_mem))?;
    let next_event_journal =
        EventJournal::open(event_journal_path(&next_mem)).map_err(|error| {
            friendly_journal_error(error, next_mem.layout.data_root(), &next_mem.space)
        })?;
    let current_path = absolute_path(current_mem_state(state)?.layout.space_dir());
    let next_path = absolute_path(next_mem.layout.space_dir());
    if current_path == next_path {
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
        *mem = next_mem;
    }
    {
        let mut cache = state
            .command_dedup
            .lock()
            .map_err(|_| "command_dedup_poisoned".to_string())?;
        *cache = next_command_dedup;
    }
    {
        let mut journal = state
            .event_journal
            .lock()
            .map_err(|_| "event_journal_poisoned".to_string())?;
        *journal = next_event_journal;
    }
    if restore_stored_sessions(state)? == 0 {
        let _ = create_session(state, None, None, BTreeMap::new())?;
    }
    let _ = schedule_selected_session_mcp_refreshes(state);
    Ok(snapshot_for(state, port))
}

fn mcp_reports(mem: &WebMemState) -> Vec<McpServerReport> {
    mem.mcp_configs
        .iter()
        .map(|config| {
            mem.mcp_reports
                .get(&config.id)
                .cloned()
                .unwrap_or_else(|| McpServerReport {
                    config: config.clone(),
                    state: if config.enabled {
                        "disconnected"
                    } else {
                        "disabled"
                    }
                    .to_string(),
                    error: None,
                    tools: Vec::new(),
                })
        })
        .map(redact_mcp_report)
        .collect()
}

fn upsert_mcp_server(state: &AppState, mut config: McpServerConfig) -> Result<(), String> {
    let mut mem = state
        .mem
        .lock()
        .map_err(|_| "mem_state_poisoned".to_string())?;
    if let Some(existing) = mem.mcp_configs.iter().find(|item| item.id == config.id) {
        merge_redacted_mcp_values(&mut config, existing);
    }
    if let Some(existing) = mem.mcp_configs.iter_mut().find(|item| item.id == config.id) {
        *existing = config;
    } else {
        mem.mcp_configs.push(config);
    }
    mem.mcp_configs.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    mem.mcp_store.save(&mem.mcp_configs)
}

fn redact_mcp_report(mut report: McpServerReport) -> McpServerReport {
    match &mut report.config.transport {
        agent_core::mcp::McpTransportConfig::Stdio { env, .. } => redact_sensitive_map(env),
        agent_core::mcp::McpTransportConfig::StreamableHttp { headers, .. } => {
            redact_sensitive_map(headers)
        }
        agent_core::mcp::McpTransportConfig::Sse { headers, .. } => redact_sensitive_map(headers),
    }
    report
}

fn redact_sensitive_map(values: &mut BTreeMap<String, String>) {
    for (key, value) in values {
        if is_sensitive_mcp_key(key) {
            *value = "****".to_string();
        }
    }
}

fn merge_redacted_mcp_values(config: &mut McpServerConfig, existing: &McpServerConfig) {
    match (&mut config.transport, &existing.transport) {
        (
            agent_core::mcp::McpTransportConfig::Stdio { env, .. },
            agent_core::mcp::McpTransportConfig::Stdio { env: old, .. },
        )
        | (
            agent_core::mcp::McpTransportConfig::StreamableHttp { headers: env, .. },
            agent_core::mcp::McpTransportConfig::StreamableHttp { headers: old, .. },
        )
        | (
            agent_core::mcp::McpTransportConfig::Sse { headers: env, .. },
            agent_core::mcp::McpTransportConfig::Sse { headers: old, .. },
        ) => {
            for (key, value) in env {
                if value == "****" {
                    if let Some(previous) = old.get(key) {
                        *value = previous.clone();
                    }
                }
            }
        }
        _ => {}
    }
}

fn mcp_server_secret_values(
    state: &AppState,
    server_id: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mem = state
        .mem
        .lock()
        .map_err(|_| "mem_state_poisoned".to_string())?;
    let config = mem
        .mcp_configs
        .iter()
        .find(|config| config.id == server_id)
        .ok_or_else(|| "mcp_server_not_found".to_string())?;
    let values = match &config.transport {
        agent_core::mcp::McpTransportConfig::Stdio { env, .. } => env,
        agent_core::mcp::McpTransportConfig::StreamableHttp { headers, .. }
        | agent_core::mcp::McpTransportConfig::Sse { headers, .. } => headers,
    };
    Ok(values
        .iter()
        .filter(|(key, _)| is_sensitive_mcp_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn is_sensitive_mcp_key(key: &str) -> bool {
    let normalized = key.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "AUTH", "CREDENTIAL"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

fn delete_mcp_server(state: &AppState, server_id: &str) -> Result<(), String> {
    let server_id = server_id.trim();
    if server_id.is_empty() {
        return Err("mcp_server_id_required".to_string());
    }
    {
        let mut mem = state
            .mem
            .lock()
            .map_err(|_| "mem_state_poisoned".to_string())?;
        mem.mcp_runtime.disconnect(server_id);
        mem.mcp_configs.retain(|config| config.id != server_id);
        mem.mcp_reports.remove(server_id);
        mem.mcp_store.save(&mem.mcp_configs)?;
    }
    let session_ids = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let mut changed = Vec::new();
        for session in sessions.values_mut() {
            let before = session.mcp_server_ids.len();
            session.mcp_server_ids.retain(|id| id != server_id);
            if session.mcp_server_ids.len() != before {
                session.mcp_config_revision = session.mcp_config_revision.saturating_add(1);
                changed.push(session.session_id.clone());
            }
        }
        changed
    };
    for session_id in session_ids {
        persist_web_session(state, &session_id)?;
    }
    Ok(())
}

fn enable_mcp_for_session(
    state: &AppState,
    session_id: &str,
    enabled: bool,
    server_id: Option<&str>,
) -> Result<(), String> {
    let server_id = server_id.ok_or_else(|| "mcp_server_id_required".to_string())?;
    if !current_mem_state(state)?
        .mcp_configs
        .iter()
        .any(|config| config.id == server_id)
    {
        return Err("mcp_server_not_found".to_string());
    }
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    let before = session.mcp_server_ids.clone();
    if enabled {
        if !session.mcp_server_ids.iter().any(|id| id == server_id) {
            session.mcp_server_ids.push(server_id.to_string());
            session.mcp_server_ids.sort();
        }
    } else {
        session.mcp_server_ids.retain(|id| id != server_id);
    }
    if session.mcp_server_ids != before {
        session.mcp_config_revision = session.mcp_config_revision.saturating_add(1);
    }
    Ok(())
}

fn mark_session_mcp_changed(state: &AppState, session_id: &str) -> Result<(), String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    session.mcp_config_revision = session.mcp_config_revision.saturating_add(1);
    Ok(())
}

fn mark_sessions_using_mcp_server(
    state: &AppState,
    server_id: &str,
) -> Result<Vec<String>, String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let mut changed = Vec::new();
    for session in sessions.values_mut() {
        if session.mcp_server_ids.iter().any(|id| id == server_id) {
            session.mcp_config_revision = session.mcp_config_revision.saturating_add(1);
            changed.push(session.session_id.clone());
        }
    }
    Ok(changed)
}

fn schedule_mcp_server_refresh(state: &AppState, server_id: &str) -> Result<bool, String> {
    let (runtime, config, space) = {
        let mut mem = state
            .mem
            .lock()
            .map_err(|_| "mem_state_poisoned".to_string())?;
        let config = mem
            .mcp_configs
            .iter()
            .find(|config| config.id == server_id)
            .cloned()
            .ok_or_else(|| "mcp_server_not_found".to_string())?;
        if !config.enabled {
            return Ok(false);
        }
        if mem
            .mcp_reports
            .get(server_id)
            .is_some_and(|report| report.config == config && report.state == "connecting")
        {
            return Ok(false);
        }
        mem.mcp_reports.insert(
            server_id.to_string(),
            McpServerReport {
                config: config.clone(),
                state: "connecting".to_string(),
                error: None,
                tools: Vec::new(),
            },
        );
        (mem.mcp_runtime.clone(), config, mem.space.clone())
    };
    let state = state.clone();
    std::thread::Builder::new()
        .name(format!("timem-mcp-connect-{}", config.id))
        .spawn(move || {
            let report = match runtime.connect(&config) {
                Ok(tools) => McpServerReport {
                    config: config.clone(),
                    state: "connected".to_string(),
                    error: None,
                    tools,
                },
                Err(error) => McpServerReport {
                    config: config.clone(),
                    state: "error".to_string(),
                    error: Some(error),
                    tools: Vec::new(),
                },
            };
            let connected = report.state == "connected";
            let accepted = state.mem.lock().ok().is_some_and(|mut mem| {
                if mem.space != space || !mem.mcp_configs.iter().any(|current| current == &config) {
                    return false;
                }
                mem.mcp_reports.insert(config.id.clone(), report);
                true
            });
            if !accepted {
                runtime.disconnect(&config.id);
                return;
            }
            let session_ids = if connected {
                mark_sessions_using_mcp_server(&state, &config.id).unwrap_or_default()
            } else {
                state
                    .sessions
                    .lock()
                    .ok()
                    .map(|sessions| {
                        sessions
                            .values()
                            .filter(|session| {
                                session.mcp_server_ids.iter().any(|id| id == &config.id)
                            })
                            .map(|session| session.session_id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            for session_id in session_ids {
                if let Ok(event) = mcp_updated_event(&state, Some(session_id)) {
                    if let Err(error) = publish_semantic(&state, event) {
                        eprintln!("[timem_web_semantic_publish_error] reason={error}");
                    }
                }
            }
        })
        .map_err(|error| format!("mcp_refresh_spawn_failed:{error}"))?;
    Ok(true)
}

fn schedule_selected_session_mcp_refreshes(state: &AppState) -> Result<usize, String> {
    let selected = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .values()
        .flat_map(|session| session.mcp_server_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let enabled = state
        .mem
        .lock()
        .map_err(|_| "mem_state_poisoned".to_string())?
        .mcp_configs
        .iter()
        .filter(|config| config.enabled && selected.contains(&config.id))
        .map(|config| config.id.clone())
        .collect::<Vec<_>>();
    let mut scheduled = 0usize;
    for server_id in enabled {
        scheduled += usize::from(schedule_mcp_server_refresh(state, &server_id)?);
    }
    Ok(scheduled)
}

fn apply_pending_session_mcp(state: &AppState, session_id: &str) -> Result<bool, String> {
    let pending = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        session.mcp_config_revision != session.applied_mcp_config_revision
    };
    if pending {
        sync_session_mcp(state, session_id)?;
    }
    Ok(pending)
}

fn sync_session_mcp(state: &AppState, session_id: &str) -> Result<(), String> {
    let (server_ids, worker_ids, target_revision) = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        (
            session.mcp_server_ids.clone(),
            session
                .workers
                .iter()
                .map(|worker| worker.worker_id.clone())
                .collect::<Vec<_>>(),
            session.mcp_config_revision,
        )
    };
    let (runtime, configs, cached_reports) = {
        let mem = state
            .mem
            .lock()
            .map_err(|_| "mem_state_poisoned".to_string())?;
        let configs = mem
            .mcp_configs
            .iter()
            .filter(|config| config.enabled && server_ids.iter().any(|id| id == &config.id))
            .cloned()
            .collect::<Vec<_>>();
        (mem.mcp_runtime.clone(), configs, mem.mcp_reports.clone())
    };
    let mut tools = Vec::<McpTool>::new();
    let mut refresh_ids = Vec::new();
    for config in &configs {
        let cached = cached_reports
            .get(&config.id)
            .filter(|report| report.config == *config && report.state == "connected")
            .map(|report| report.tools.clone());
        if let Some(discovered) = cached {
            tools.extend(discovered);
        } else {
            refresh_ids.push(config.id.clone());
        }
    }
    let base =
        CapabilityRegistry::builtin_with_overlay_dir(state.template.data_dir.join("capabilities"))
            .unwrap_or_else(|_| CapabilityRegistry::builtin());
    let manager = state
        .manager
        .lock()
        .map_err(|_| "worker_manager_poisoned".to_string())?;
    for worker_id in worker_ids {
        if let Some(handle) = manager.handle(&worker_id) {
            handle.update_mcp(
                base.clone(),
                runtime.clone(),
                configs.clone(),
                tools.clone(),
            )?;
        }
    }
    drop(manager);
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if session.mcp_config_revision == target_revision {
            session.applied_mcp_config_revision = target_revision;
        }
    }
    persist_web_session(state, session_id)?;
    for server_id in refresh_ids {
        let _ = schedule_mcp_server_refresh(state, &server_id)?;
    }
    Ok(())
}

fn mcp_updated_event(state: &AppState, session_id: Option<String>) -> Result<WireEvent, String> {
    let servers = mcp_reports(&current_mem_state(state)?);
    let enabled_server_ids = match session_id.as_deref() {
        Some(session_id) => state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?
            .get(session_id)
            .map(|session| session.mcp_server_ids.clone())
            .ok_or_else(|| "session_not_found".to_string())?,
        None => Vec::new(),
    };
    Ok(WireEvent::McpUpdated {
        session_id,
        servers,
        enabled_server_ids,
    })
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
        let mcp_server_ids = current_mem_state(state)?
            .mcp_configs
            .into_iter()
            .filter(|config| config.enabled)
            .map(|config| config.id)
            .collect::<Vec<_>>();
        let (mcp_config_revision, applied_mcp_config_revision) =
            initial_mcp_revisions(&mcp_server_ids);
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
                mcp_server_ids,
                mcp_config_revision,
                applied_mcp_config_revision,
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
                pending_turn_id: None,
                pending_completion_message_id: None,
                pending_unconsumed_supplements: Vec::new(),
                reported_session_working_worker_count: None,
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
    let stored_sessions = list_stored_sessions_resilient(&current_session_store(state)?)?;
    let mut restored = 0usize;
    for stored in stored_sessions {
        let session_id = stored.session_id.clone();
        match restore_stored_session(state, stored) {
            Ok(()) => restored += 1,
            Err(error) => eprintln!(
                "[timem_web_session_restore_error] session_id={session_id:?} reason={error}"
            ),
        }
    }
    Ok(restored)
}

fn list_stored_sessions_resilient(store: &SessionStore) -> Result<Vec<StoredSession>, String> {
    match store.list_sessions() {
        Ok(sessions) => Ok(sessions),
        Err(error) if error == "session_record_parse_failed" => {
            let path = store.index_path();
            let raw = std::fs::read(&path)
                .map_err(|read_error| format!("session_index_recovery_read_failed:{read_error}"))?;
            let mut sessions = Vec::new();
            let mut repaired = Vec::with_capacity(raw.len());
            let mut invalid_records = 0usize;
            for line in raw.split(|byte| *byte == b'\n') {
                if line.iter().all(|byte| byte.is_ascii_whitespace()) {
                    continue;
                }
                match serde_json::from_slice::<StoredSession>(line) {
                    Ok(session) => {
                        serde_json::to_writer(&mut repaired, &session).map_err(
                            |serialize_error| {
                                format!("session_index_recovery_serialize_failed:{serialize_error}")
                            },
                        )?;
                        repaired.push(b'\n');
                        sessions.push(session);
                    }
                    Err(_) => invalid_records = invalid_records.saturating_add(1),
                }
            }
            let backup =
                backup_and_replace_corrupt_state(&path, &repaired, "session-index-corrupt-backup")?;
            sessions.sort_by(|left, right| {
                right
                    .updated_at_ms
                    .cmp(&left.updated_at_ms)
                    .then_with(|| left.session_id.cmp(&right.session_id))
            });
            eprintln!(
                "[timem_web_warning] session_index_corruption_repaired invalid_records={invalid_records} backup={}",
                backup.display()
            );
            Ok(sessions)
        }
        Err(error) => Err(error),
    }
}

fn backup_and_replace_corrupt_state(
    path: &Path,
    replacement: &[u8],
    backup_label: &str,
) -> Result<PathBuf, String> {
    let raw =
        std::fs::read(path).map_err(|error| format!("recoverable_state_read_failed:{error}"))?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup = path_with_appended_suffix(path, &format!(".{backup_label}-{suffix}"));
    write_new_private_synced_file(&backup, &raw, "recoverable_state_backup")?;

    let temporary = path_with_appended_suffix(
        path,
        &format!(".recovery-{}-{suffix}.tmp", std::process::id()),
    );
    write_new_private_synced_file(&temporary, replacement, "recoverable_state_replacement")?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("recoverable_state_replace_failed:{error}"));
    }
    sync_state_parent_directory(path)?;
    Ok(backup)
}

fn path_with_appended_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn write_new_private_synced_file(path: &Path, body: &[u8], label: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("{label}_open_failed:{error}"))?;
    file.write_all(body)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("{label}_write_failed:{error}"))
}

fn sync_state_parent_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("recoverable_state_dir_sync_failed:{error}"))?;
    }
    Ok(())
}

fn restore_stored_session(state: &AppState, stored: StoredSession) -> Result<(), String> {
    let current_dir = PathBuf::from(&stored.current_dir);
    if !current_dir.is_dir() {
        return Err("stored_session_workspace_not_found".to_string());
    }
    let cached_env = sanitize_restored_session_env(if stored.env.is_empty() {
        stored.env_overrides.clone().unwrap_or_default()
    } else {
        stored.env.clone()
    });
    let settings = state.template.session_settings(&cached_env)?;
    let session_env = state.template.session_env(&settings, &cached_env);
    let runtime = WebSessionRuntime {
        settings,
        env: session_env,
        env_overrides: stored.env_overrides.clone().unwrap_or_default(),
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
        let (mcp_config_revision, applied_mcp_config_revision) =
            initial_mcp_revisions(&stored.mcp_server_ids);
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
                mcp_server_ids: stored.mcp_server_ids.clone(),
                mcp_config_revision,
                applied_mcp_config_revision,
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
                pending_turn_id: None,
                pending_completion_message_id: None,
                pending_unconsumed_supplements: Vec::new(),
                reported_session_working_worker_count: None,
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
        Some(stored.display_name.clone()),
        None,
        true,
    )?;
    resume_unfinished_core_command_after_restore(state, &stored.session_id)?;
    persist_restored_session_runtime_cache(state, &stored)?;
    Ok(())
}

fn resume_unfinished_core_command_after_restore(
    state: &AppState,
    session_id: &str,
) -> Result<(), String> {
    let pending = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        // Only the newest turn may represent work interrupted by the process
        // restart. Never skip a completed newer turn and revive an older,
        // abandoned turn: doing so makes a restored ready session appear to be
        // working immediately after Web startup.
        let pending = session.turns.last().and_then(|turn| {
            if turn.final_answer.is_some() || turn.completion.is_some() {
                return None;
            }
            let entries = turn
                .user_entries
                .iter()
                .filter_map(|entry| {
                    entry.delivery_state.as_ref()?;
                    Some((
                        entry.command_id.clone()?,
                        entry.text.clone(),
                        entry.attachments.clone(),
                        entry.kind.clone(),
                    ))
                })
                .collect::<Vec<_>>();
            (!entries.is_empty()).then(|| (turn.turn_id.clone(), entries))
        });
        // Persisted history may restore chat content and an intent to redrive,
        // but it is not evidence that the new Core process is executing.
        // Keep the session ready and the turn restored until TurnStarted.
        pending
    };
    let Some((_turn_id, entries)) = pending else {
        return Ok(());
    };
    let worker = primary_worker_handle(state, session_id)?;
    let (command_id, text, attachments, kind) = entries[0].clone();
    if kind == "toolgen_instruction" {
        worker.run_toolgen_with_command_id(ToolGenRequest::new(Some(text)), Some(command_id))
    } else {
        worker.run_turn_batch_with_command_ids(
            text,
            session_context(state, session_id, &attachments)?,
            Some(command_id),
            entries
                .iter()
                .skip(1)
                .filter(|(_, _, _, kind)| kind == "supplement")
                .map(|(command_id, text, attachments, _)| {
                    let mut worker_text = text.clone();
                    if let Some(context) = uploaded_files_context(attachments) {
                        worker_text.push_str("\n\n");
                        worker_text.push_str(&context);
                    }
                    (worker_text, Some(command_id.clone()))
                })
                .collect(),
        )
    }?;
    Ok(())
}

fn persist_restored_session_runtime_cache(
    state: &AppState,
    stored: &StoredSession,
) -> Result<(), String> {
    let (profile, env, env_overrides) = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(&stored.session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        (
            StoredSessionProfile {
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
            session_cached_env_values(&session.runtime.settings),
            session
                .runtime
                .env_overrides
                .iter()
                .filter(|(key, _)| key.as_str() != "TIMEM_API_KEY")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    };
    let mut migrated = stored.clone();
    migrated.profile = profile;
    migrated.env = env;
    migrated.env_overrides = Some(env_overrides);
    current_session_store(state)?.upsert_session(&migrated)
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
                command_id,
                delivery_state,
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
                        command_id,
                        delivery_state,
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
                if kind == ChatHistoryEventKind::RuntimeNotice
                    && extra.get("kind").and_then(Value::as_str) == Some("command_delivery")
                {
                    if let (Some(command_id), Some(delivery_state)) = (
                        extra.get("command_id").and_then(Value::as_str),
                        extra
                            .get("delivery_state")
                            .and_then(|value| serde_json::from_value(value.clone()).ok()),
                    ) {
                        for restored_turn in turns.values_mut() {
                            for entry in &mut restored_turn.user_entries {
                                if entry.command_id.as_deref() == Some(command_id) {
                                    entry.delivery_state = Some(delivery_state);
                                }
                            }
                        }
                    }
                    continue;
                }
                let completion = (kind == ChatHistoryEventKind::Stats)
                    .then(|| extra.get("completion").cloned())
                    .flatten();
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
                if let Some(completion) = completion {
                    turn.state = "completed".to_string();
                    turn.completion = Some(completion);
                }
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
            ..
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
        env: session_cached_env_values(&session.runtime.settings),
        env_overrides: Some(
            session
                .runtime
                .env_overrides
                .iter()
                .filter(|(key, _)| key.as_str() != "TIMEM_API_KEY")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        mcp_server_ids: session.mcp_server_ids.clone(),
        state: if session.state == "error" {
            StoredSessionState::Error
        } else if session.active_turn_id.is_some()
            || session.pending_turn_id.is_some()
            || session.state == "working"
        {
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

fn current_turn_id(session: &WebSession) -> Option<&str> {
    session
        .active_turn_id
        .as_deref()
        .or(session.pending_turn_id.as_deref())
}

fn session_has_active_turn(state: &AppState, session_id: &str) -> Result<bool, String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    Ok(current_turn_id(session)
        .is_some_and(|turn_id| session.turns.iter().any(|turn| turn.turn_id == turn_id)))
}

fn turn_for_command_id(
    state: &AppState,
    session_id: &str,
    command_id: &str,
) -> Result<Option<WebTurn>, String> {
    Ok(state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .get(session_id)
        .ok_or_else(|| "session_not_found".to_string())?
        .turns
        .iter()
        .find(|turn| {
            turn.user_entries
                .iter()
                .any(|entry| entry.command_id.as_deref() == Some(command_id))
        })
        .cloned())
}

fn redeliver_recorded_turn(
    state: &AppState,
    session_id: &str,
    command_id: &str,
    input_kind: Option<&str>,
    text: &str,
    turn: &WebTurn,
) -> Result<(), String> {
    let Some(entry) = turn
        .user_entries
        .iter()
        .find(|entry| entry.command_id.as_deref() == Some(command_id))
    else {
        return Ok(());
    };
    // CoreAccepted is process-local dequeue, not durable completion. An
    // unfinished turn is therefore re-driven after restart with the same ID.
    let worker = primary_worker_handle(state, session_id)?;
    if input_kind == Some("toolgen") {
        let instruction = (!text.trim().is_empty()).then(|| text.trim().to_string());
        worker.run_toolgen_with_command_id(
            ToolGenRequest::new(instruction),
            Some(command_id.to_string()),
        )
    } else {
        worker.run_turn_with_command_id(
            entry.text.clone(),
            session_context(state, session_id, &entry.attachments)?,
            Some(command_id.to_string()),
        )
    }
}

fn submit_or_supplement_turn(
    state: &AppState,
    session_id: &str,
    text: String,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    if session_has_active_turn(state, session_id)? {
        if let Some(turn) =
            try_append_turn_supplement(state, session_id, text.clone(), attachment_ids, command_id)?
        {
            return Ok(turn);
        }
    }
    submit_turn_with_selected_attachments(state, session_id, text, attachment_ids, command_id)
}

fn append_supplement_or_submit_turn(
    state: &AppState,
    session_id: &str,
    text: String,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    match try_append_turn_supplement(state, session_id, text.clone(), attachment_ids, command_id)? {
        Some(turn) => Ok(turn),
        None => submit_turn_with_selected_attachments(
            state,
            session_id,
            text,
            attachment_ids,
            command_id,
        ),
    }
}

fn try_append_turn_supplement(
    state: &AppState,
    session_id: &str,
    text: String,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<Option<WebTurn>, String> {
    if !session_has_active_turn(state, session_id)? {
        return Ok(None);
    }
    let worker_handle = primary_worker_handle(state, session_id)?;
    let selected_attachments = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        pending_attachments_for_ids(session, attachment_ids)?
    };
    let mut worker_supplement_text = text.clone();
    if let Some(context) = uploaded_files_context(&selected_attachments) {
        worker_supplement_text.push_str("\n\n");
        worker_supplement_text.push_str(&context);
    }
    let mut appended_turn = None;
    let accepted = worker_handle.try_add_user_supplement_with_command_id_after(
        worker_supplement_text,
        command_id.map(str::to_string),
        || {
            appended_turn = Some(append_turn_supplement_with_selected_attachments(
                state,
                session_id,
                text.clone(),
                attachment_ids,
                command_id,
            )?);
            Ok(())
        },
    )?;
    if !accepted {
        settle_closed_primary_turn(state, session_id)?;
        return Ok(None);
    }
    Ok(appended_turn)
}

fn settle_closed_primary_turn(state: &AppState, session_id: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while session_has_active_turn(state, session_id)? {
        for (event_session_id, context_id, worker_id, event) in drain_worker_events(state) {
            handle_scoped_worker_event(state, &event_session_id, &context_id, &worker_id, event);
        }
        if !session_has_active_turn(state, session_id)? {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err("closed_turn_settle_timeout".to_string());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
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
    let (runtime, current_dir, mcp_server_ids) = {
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
        (
            session.runtime.clone(),
            PathBuf::from(&context.current_dir),
            session.mcp_server_ids.clone(),
        )
    };

    let mem = current_mem_state(state)?;
    let mut core =
        state
            .template
            .new_core_at(&mem, &current_dir, &runtime.settings, runtime.env.clone())?;
    let mcp_configs = mem
        .mcp_configs
        .iter()
        .filter(|config| config.enabled && mcp_server_ids.iter().any(|id| id == &config.id))
        .cloned()
        .collect::<Vec<_>>();
    let mut mcp_tools = Vec::new();
    for config in &mcp_configs {
        if let Some(report) = mem
            .mcp_reports
            .get(&config.id)
            .filter(|report| report.config == *config && report.state == "connected")
        {
            mcp_tools.extend(report.tools.clone());
        }
    }
    let base =
        CapabilityRegistry::builtin_with_overlay_dir(state.template.data_dir.join("capabilities"))
            .unwrap_or_else(|_| CapabilityRegistry::builtin());
    core.configure_mcp(base, mem.mcp_runtime.clone(), mcp_configs, mcp_tools)?;
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
    submit_turn_with_command_id(state, session_id, text, None)
}

fn submit_turn_with_command_id(
    state: &AppState,
    session_id: &str,
    text: String,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    submit_turn_with_selected_attachments(state, session_id, text, None, command_id)
}

fn submit_turn_with_selected_attachments(
    state: &AppState,
    session_id: &str,
    text: String,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    validate_session_model_service_config(state, session_id)?;
    apply_pending_session_mcp(state, session_id)?;
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
        let turn = start_web_turn_with_selected_attachments(
            state,
            session_id,
            &text,
            attachment_ids,
            command_id,
        )?;
        publish_semantic(
            state,
            WireEvent::TurnUpdated {
                session_id: session_id.to_string(),
                turn: turn.clone(),
            },
        )?;
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
            // Waiting for a Host decision is not evidence of Core execution.
            session.pending_work_instruction_turn = Some(PendingWorkInstructionTurn {
                request_id: request_id.clone(),
                text,
                attachments,
                command_id: command_id.map(str::to_string),
            });
        }
        let wire_payload = event.wire_payload();
        let turn_ref =
            append_active_turn_event(state, session_id, "core_topic", wire_payload.clone());
        publish_semantic(
            state,
            WireEvent::CoreTopic {
                turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
                turn_event_id: turn_ref.map(|value| value.event_id),
                event: wire_payload,
            },
        )?;
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
    let turn = start_web_turn_with_selected_attachments(
        state,
        session_id,
        &text,
        attachment_ids,
        command_id,
    )?;
    // Publish the authoritative turn before allowing Core to emit activity for
    // it. Otherwise a fast worker event can overtake the direct command reply.
    if let Err(error) = publish_semantic(
        state,
        WireEvent::TurnUpdated {
            session_id: session_id.to_string(),
            turn: turn.clone(),
        },
    ) {
        let attachments = turn.user_entries[0].attachments.clone();
        rollback_web_turn(state, session_id, &turn.turn_id, attachments);
        return Err(error);
    }
    let attachments = turn.user_entries[0].attachments.clone();
    if let Err(error) = primary_worker_handle(state, session_id)?.run_turn_with_command_id(
        text,
        session_context(state, session_id, &attachments)?,
        command_id.map(str::to_string),
    ) {
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
            publish_semantic(state, event)?;
        }
    }
    primary_worker_handle(state, session_id)?.run_turn_with_command_id(
        pending.text,
        session_context(state, session_id, &pending.attachments)?,
        pending.command_id,
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

fn switch_session_bash_approval(
    state: &AppState,
    session_id: &str,
    mode: BashApprovalMode,
) -> Result<(), String> {
    // Update session runtime settings
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned")?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        session.runtime.settings.bash_approval_mode = mode;
        session.runtime.env.insert(
            "TIMEM_BASH_APPROVAL".to_string(),
            agent_core::bash_approval_mode_label(mode).to_string(),
        );
        session.runtime_profile.bash_approval =
            agent_core::bash_approval_mode_label(mode).to_string();
    }
    // Notify all workers in the session
    let worker_ids = session_worker_ids(state, session_id)?;
    let manager = state
        .manager
        .lock()
        .map_err(|_| "worker_manager_poisoned")?;
    for worker_id in &worker_ids {
        if let Some(handle) = manager.handle(worker_id) {
            let _ = handle.update_bash_approval(mode);
        }
    }
    drop(manager);
    persist_web_session(state, session_id)
}

fn update_session_api_key(
    state: &AppState,
    session_id: &str,
    api_key: String,
) -> Result<WebSessionRuntimeProfile, String> {
    if api_key.len() > 8 * 1024 {
        return Err("session_api_key_too_large".to_string());
    }
    if !api_key.is_empty() {
        validate_api_key(&api_key).map_err(|error| format!("invalid_session_api_key:{error}"))?;
    }
    if session_has_active_turn(state, session_id)? {
        return Err("session_api_key_update_while_working".to_string());
    }

    let worker_ids = session_worker_ids(state, session_id)?;
    {
        let manager = state
            .manager
            .lock()
            .map_err(|_| "worker_manager_poisoned".to_string())?;
        for worker_id in &worker_ids {
            manager
                .handle(worker_id)
                .ok_or_else(|| "session_worker_not_found".to_string())?
                .update_api_key(api_key.clone())?;
        }
    }

    let runtime_profile = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        session.runtime.settings.config.api_key = api_key.clone();
        session
            .runtime
            .env
            .insert("TIMEM_API_KEY".to_string(), api_key);
        session.runtime_profile =
            WebSessionRuntimeProfile::from_settings(&session.runtime.settings);
        session.runtime_profile.clone()
    };
    persist_web_session(state, session_id)?;
    Ok(runtime_profile)
}

fn session_api_key(state: &AppState, session_id: &str) -> Result<String, String> {
    state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .get(session_id)
        .map(|session| session.runtime.settings.config.api_key.clone())
        .ok_or_else(|| "session_not_found".to_string())
}

fn update_session_runtime_setting(
    state: &AppState,
    session_id: &str,
    key: &str,
    value: &str,
) -> Result<(String, WebSessionRuntimeProfile), String> {
    if key == "TIMEM_MAX_ROUNDS" {
        let max_rounds = parse_round_budget(value)?;
        let normalized_value = round_budget_value(max_rounds);
        let worker_ids = session_worker_ids(state, session_id)?;
        {
            let manager = state
                .manager
                .lock()
                .map_err(|_| "worker_manager_poisoned".to_string())?;
            for worker_id in &worker_ids {
                manager
                    .handle(worker_id)
                    .ok_or_else(|| "session_worker_not_found".to_string())?
                    .update_max_rounds(max_rounds)?;
            }
        }
        let runtime_profile = {
            let mut sessions = state
                .sessions
                .lock()
                .map_err(|_| "session_store_poisoned".to_string())?;
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| "session_not_found".to_string())?;
            session.runtime.settings.max_rounds = max_rounds;
            session
                .runtime
                .env
                .insert(key.to_string(), normalized_value.clone());
            session
                .runtime
                .env_overrides
                .insert(key.to_string(), normalized_value.clone());
            session.runtime_profile =
                WebSessionRuntimeProfile::from_settings(&session.runtime.settings);
            session.runtime_profile.clone()
        };
        persist_web_session(state, session_id)?;
        return Ok((normalized_value, runtime_profile));
    }
    let field = runtime_config_field_from_key(key)?;
    let worker_ids = session_worker_ids(state, session_id)?;
    let mut settings = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        session.runtime.settings.clone()
    };
    let effect = apply_runtime_config_value(
        &mut settings.config,
        &mut settings.bash_approval_mode,
        &mut settings.work_instruction_mode,
        field,
        value,
    )
    .map_err(|error| format!("invalid_session_runtime_config:{error:?}"))?;
    let report = agent_core::runtime_config_apply_report(
        &settings.config,
        settings.bash_approval_mode,
        settings.work_instruction_mode,
        field,
        effect,
    );

    {
        let manager = state
            .manager
            .lock()
            .map_err(|_| "worker_manager_poisoned".to_string())?;
        for worker_id in &worker_ids {
            manager
                .handle(worker_id)
                .ok_or_else(|| "session_worker_not_found".to_string())?
                .update_runtime_config(field, report.value.clone())?;
        }
    }

    let runtime_profile = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        session.runtime.settings = settings;
        session
            .runtime
            .env
            .extend(session_cached_env_values(&session.runtime.settings));
        session.runtime_profile =
            WebSessionRuntimeProfile::from_settings(&session.runtime.settings);
        session.max_llm_input_tokens = session.runtime.settings.config.max_llm_input_tokens;
        session.runtime_profile.clone()
    };
    persist_web_session(state, session_id)?;
    Ok((report.value, runtime_profile))
}

fn propagate_runtime_config_to_sessions(
    state: &AppState,
    field: agent_core::RuntimeConfigField,
    value: &str,
) {
    // Collect all session IDs and their worker IDs
    let session_worker_pairs: Vec<(String, Vec<String>)> = {
        let Ok(sessions) = state.sessions.lock() else {
            return;
        };
        sessions
            .iter()
            .map(|(sid, session)| {
                let worker_ids: Vec<String> = session
                    .workers
                    .iter()
                    .map(|w| w.worker_id.clone())
                    .collect();
                (sid.clone(), worker_ids)
            })
            .collect()
    };

    // Update each session's runtime settings
    {
        let Ok(mut sessions) = state.sessions.lock() else {
            return;
        };
        for (sid, _) in &session_worker_pairs {
            if let Some(session) = sessions.get_mut(sid.as_str()) {
                let settings = &mut session.runtime.settings;
                let _ = agent_core::apply_runtime_config_value(
                    &mut settings.config,
                    &mut settings.bash_approval_mode,
                    &mut settings.work_instruction_mode,
                    field,
                    value,
                );
                // Update the runtime_profile for UI display
                session.runtime_profile =
                    WebSessionRuntimeProfile::from_settings(&session.runtime.settings);
                session
                    .runtime
                    .env
                    .extend(session_cached_env_values(&session.runtime.settings));
            }
        }
    }

    // Notify all workers
    let Ok(manager) = state.manager.lock() else {
        return;
    };
    for (_, worker_ids) in &session_worker_pairs {
        for worker_id in worker_ids {
            if let Some(handle) = manager.handle(worker_id) {
                let _ = handle.update_runtime_config(field, value.to_string());
            }
        }
    }
    drop(manager);
    for (session_id, _) in &session_worker_pairs {
        let _ = persist_web_session(state, session_id);
    }
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
    let turn_id = current_turn_id(session).map(str::to_string);
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
            None,
            created_at_ms,
            text_for_history,
        )?;
    }
    Ok(message_id)
}

fn delete_chat_message(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    role: &str,
    role_index: usize,
) -> Result<(), String> {
    let history_role = match role {
        "user" => ChatHistoryRole::User,
        "assistant" if role_index == 0 => ChatHistoryRole::Assistant,
        "assistant" => return Err("assistant_message_index_invalid".to_string()),
        _ => return Err("chat_message_role_invalid".to_string()),
    };
    let (content, created_at_ms) = {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if session.active_turn_id.as_deref() == Some(turn_id) {
            return Err("active_turn_message_delete_not_allowed".to_string());
        }
        let turn = session
            .turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .ok_or_else(|| "turn_not_found".to_string())?;
        match history_role {
            ChatHistoryRole::User => turn
                .user_entries
                .get(role_index)
                .map(|entry| (entry.text.clone(), entry.created_at_ms))
                .ok_or_else(|| "chat_message_not_found".to_string())?,
            ChatHistoryRole::Assistant => turn
                .final_answer
                .as_ref()
                .map(|answer| (answer.clone(), turn.created_at_ms))
                .ok_or_else(|| "chat_message_not_found".to_string())?,
            ChatHistoryRole::System => return Err("chat_message_role_invalid".to_string()),
        }
    };

    current_session_store(state)?.delete_history_message(
        session_id,
        turn_id,
        history_role,
        role_index,
    )?;

    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        let turn = session
            .turns
            .iter_mut()
            .find(|turn| turn.turn_id == turn_id)
            .ok_or_else(|| "turn_not_found".to_string())?;
        match history_role {
            ChatHistoryRole::User => {
                if role_index >= turn.user_entries.len() {
                    return Err("chat_message_not_found".to_string());
                }
                turn.user_entries.remove(role_index);
            }
            ChatHistoryRole::Assistant => turn.final_answer = None,
            ChatHistoryRole::System => return Err("chat_message_role_invalid".to_string()),
        }
        remove_closest_chat_message(&mut session.messages, role, &content, created_at_ms);
    }
    persist_web_session(state, session_id)?;
    Ok(())
}

fn remove_closest_chat_message(
    messages: &mut Vec<WebChatMessage>,
    role: &str,
    content: &str,
    created_at_ms: u128,
) {
    let candidate = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == role && message.text == content)
        .min_by_key(|(_, message)| message.created_at_ms.abs_diff(created_at_ms))
        .map(|(index, _)| index);
    if let Some(index) = candidate {
        messages.remove(index);
    }
}

#[cfg(test)]
fn start_web_turn(state: &AppState, session_id: &str, text: &str) -> Result<WebTurn, String> {
    let turn = start_web_turn_with_command_id(state, session_id, text, None)?;
    let worker_id = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .get(session_id)
        .map(|session| session.primary_worker_id.clone())
        .ok_or_else(|| "session_not_found".to_string())?;

    // This helper represents a turn already entered by Core. Production
    // submission paths remain pending until the real TurnStarted event.
    activate_core_started_turn(state, session_id, &worker_id, None);
    state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?
        .get(session_id)
        .and_then(|session| {
            session
                .turns
                .iter()
                .find(|candidate| candidate.turn_id == turn.turn_id)
                .cloned()
        })
        .ok_or_else(|| "turn_not_found".to_string())
}

#[cfg(test)]
fn start_web_turn_with_command_id(
    state: &AppState,
    session_id: &str,
    text: &str,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    start_web_turn_with_selected_attachments(state, session_id, text, None, command_id)
}

fn start_web_turn_with_selected_attachments(
    state: &AppState,
    session_id: &str,
    text: &str,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    if current_turn_id(session).is_some() {
        return Err("turn_already_active_use_supplement".to_string());
    }
    // Host state must not expose a turn that failed its durable write. Keep a
    // complete pre-mutation image because this mutation also consumes pending
    // attachments and may evict bounded turn/message history.
    let previous_session = session.clone();
    let attachments = take_pending_attachments_for_ids(session, attachment_ids)?;
    let turn = WebTurn {
        turn_id: unique_web_id("web_turn"),
        state: "pending".to_string(),
        created_at_ms: now_ms(),
        user_entries: vec![WebTurnUserEntry {
            kind: "task".to_string(),
            text: text.to_string(),
            attachments,
            created_at_ms: now_ms(),
            command_id: command_id.map(str::to_string),
            delivery_state: command_id.map(|_| ChatCommandDeliveryState::Recorded),
        }],
        events: Vec::new(),
        final_answer: None,
        completion: None,
    };
    session.pending_turn_id = Some(turn.turn_id.clone());
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
    let persist_result = append_chat_history_message(
        state,
        session_id,
        &turn_id,
        "user",
        Some("task"),
        command_id,
        created_at_ms,
        text.to_string(),
    )
    .and_then(|()| persist_web_session(state, session_id));
    if let Err(error) = persist_result {
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.insert(session_id.to_string(), previous_session);
        }
        return Err(error);
    }
    Ok(turn)
}

fn submit_toolgen_turn(
    state: &AppState,
    session_id: &str,
    source_turn_id: &str,
    user_instruction: Option<String>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    validate_session_model_service_config(state, session_id)?;
    {
        let sessions = state
            .sessions
            .lock()
            .map_err(|_| "session_store_poisoned".to_string())?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| "session_not_found".to_string())?;
        if current_turn_id(session).is_some() {
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
    apply_pending_session_mcp(state, session_id)?;
    let turn = start_web_toolgen_turn(
        state,
        session_id,
        source_turn_id,
        user_instruction.as_deref(),
        command_id,
    )?;
    if let Err(error) = publish_semantic(
        state,
        WireEvent::TurnUpdated {
            session_id: session_id.to_string(),
            turn: turn.clone(),
        },
    ) {
        rollback_web_turn(state, session_id, &turn.turn_id, Vec::new());
        return Err(error);
    }
    let request = ToolGenRequest::new(user_instruction);
    if let Err(error) = primary_worker_handle(state, session_id)?
        .run_toolgen_with_command_id(request, command_id.map(str::to_string))
    {
        rollback_web_turn(state, session_id, &turn.turn_id, Vec::new());
        return Err(error);
    }
    Ok(turn)
}

fn validate_session_model_service_config(state: &AppState, session_id: &str) -> Result<(), String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    validate_api_key(&session.runtime.settings.config.api_key)
        .map_err(|error| format!("session_model_service_config_incomplete:{error}"))
}

fn start_web_toolgen_turn(
    state: &AppState,
    session_id: &str,
    source_turn_id: &str,
    user_instruction: Option<&str>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    let created_at_ms = now_ms();
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    if current_turn_id(session).is_some() {
        return Err("turn_already_active".to_string());
    }
    let user_entries = user_instruction
        .map(|text| {
            vec![WebTurnUserEntry {
                kind: "toolgen_instruction".to_string(),
                text: text.to_string(),
                attachments: Vec::new(),
                created_at_ms,
                command_id: command_id.map(str::to_string),
                delivery_state: command_id.map(|_| ChatCommandDeliveryState::Recorded),
            }]
        })
        .unwrap_or_default();
    let turn = WebTurn {
        turn_id: unique_web_id("web_toolgen_turn"),
        state: "pending".to_string(),
        created_at_ms,
        user_entries,
        events: Vec::new(),
        final_answer: None,
        completion: None,
    };
    session.pending_turn_id = Some(turn.turn_id.clone());
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
            command_id,
            created_at_ms as i64,
            text.to_string(),
        )?;
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
    append_turn_user_entry_with_attachments(
        state,
        session_id,
        kind,
        text,
        Vec::new(),
        Some(&[]),
        None,
    )
}

#[cfg(test)]
fn append_turn_supplement_with_pending_attachments(
    state: &AppState,
    session_id: &str,
    text: String,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    append_turn_supplement_with_selected_attachments(state, session_id, text, None, command_id)
}

fn append_turn_supplement_with_selected_attachments(
    state: &AppState,
    session_id: &str,
    text: String,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    append_turn_user_entry_with_attachments(
        state,
        session_id,
        "supplement",
        text,
        Vec::new(),
        attachment_ids,
        command_id,
    )
}

fn append_turn_user_entry_with_attachments(
    state: &AppState,
    session_id: &str,
    kind: &str,
    text: String,
    attachments: Vec<WebAttachment>,
    attachment_ids: Option<&[String]>,
    command_id: Option<&str>,
) -> Result<WebTurn, String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "session_store_poisoned".to_string())?;
    let session = sessions
        .get_mut(session_id)
        .ok_or_else(|| "session_not_found".to_string())?;
    let previous_session = session.clone();
    let active_turn_id = current_turn_id(session)
        .map(str::to_string)
        .ok_or_else(|| "active_turn_not_found".to_string())?;
    if !session
        .turns
        .iter()
        .any(|turn| turn.turn_id == active_turn_id)
    {
        return Err("active_turn_not_found".to_string());
    }
    let attachments = if attachments.is_empty() {
        take_pending_attachments_for_ids(session, attachment_ids)?
    } else {
        attachments
    };
    let turn = session
        .turns
        .iter_mut()
        .find(|turn| turn.turn_id == active_turn_id)
        .expect("active turn existence checked before attachment consumption");
    turn.user_entries.push(WebTurnUserEntry {
        kind: kind.to_string(),
        text,
        attachments,
        created_at_ms: now_ms(),
        command_id: command_id.map(str::to_string),
        delivery_state: command_id.map(|_| ChatCommandDeliveryState::Recorded),
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
    let persist_result = append_chat_history_message(
        state,
        session_id,
        &active_turn_id,
        "user",
        history_kind,
        last_entry
            .as_ref()
            .and_then(|entry| entry.command_id.as_deref()),
        created_at_ms,
        content,
    )
    .and_then(|()| persist_web_session(state, session_id));
    if let Err(error) = persist_result {
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.insert(session_id.to_string(), previous_session);
        }
        return Err(error);
    }
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
            }
            if session.pending_turn_id.as_deref() == Some(turn_id) {
                session.pending_turn_id = None;
            }
            session.state = if session
                .workers
                .iter()
                .any(|worker| worker.state == "working")
            {
                "working"
            } else {
                "ready"
            }
            .to_string();
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
    let active_turn_id = current_turn_id(session).map(str::to_string)?;
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

#[allow(clippy::too_many_arguments)]
fn append_chat_history_message(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    role: &str,
    kind: Option<&str>,
    command_id: Option<&str>,
    created_at_ms: i64,
    content: String,
) -> Result<(), String> {
    let role = match role {
        "user" => ChatHistoryRole::User,
        "assistant" => ChatHistoryRole::Assistant,
        "system" => ChatHistoryRole::System,
        _ => ChatHistoryRole::System,
    };
    current_session_store(state)?.append_history_record(
        session_id,
        &ChatHistoryRecord::Message {
            role,
            turn_id: turn_id.to_string(),
            created_at_ms,
            kind: kind.map(ToString::to_string),
            command_id: command_id.map(str::to_string),
            delivery_state: command_id.map(|_| ChatCommandDeliveryState::Recorded),
            content,
        },
    )
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
            "Previously accumulated reusable scripts are available at: {}\nThe tool directories have semantic names. When one may help with the current task, inspect the directory and run the script's --help through run_bash as needed.",
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

fn sanitize_upload_session_component(session_id: &str) -> Result<&str, String> {
    let valid = !session_id.is_empty()
        && session_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'));
    if valid {
        Ok(session_id)
    } else {
        Err("invalid_upload_session_id".to_string())
    }
}

async fn store_upload(
    state: &AppState,
    session_id: &str,
    name: String,
    bytes: &[u8],
) -> Result<WebAttachment, String> {
    let session_dir = sanitize_upload_session_component(session_id)?;
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
    let base_dir = state
        .template
        .data_dir
        .join("web_uploads")
        .join(session_dir);
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

fn pending_attachments_for_ids(
    session: &WebSession,
    attachment_ids: Option<&[String]>,
) -> Result<Vec<WebAttachment>, String> {
    let Some(attachment_ids) = attachment_ids else {
        return Ok(session.attachments.clone());
    };
    let mut seen = BTreeSet::new();
    let mut selected = Vec::with_capacity(attachment_ids.len());
    for attachment_id in attachment_ids {
        if !seen.insert(attachment_id.as_str()) {
            return Err("duplicate_attachment_id".to_string());
        }
        let attachment = session
            .attachments
            .iter()
            .find(|attachment| attachment.id == *attachment_id)
            .cloned()
            .ok_or_else(|| "pending_attachment_not_found".to_string())?;
        selected.push(attachment);
    }
    Ok(selected)
}

fn take_pending_attachments_for_ids(
    session: &mut WebSession,
    attachment_ids: Option<&[String]>,
) -> Result<Vec<WebAttachment>, String> {
    let selected = pending_attachments_for_ids(session, attachment_ids)?;
    if attachment_ids.is_none() {
        session.attachments.clear();
    } else {
        let selected_ids = selected
            .iter()
            .map(|attachment| attachment.id.as_str())
            .collect::<BTreeSet<_>>();
        session
            .attachments
            .retain(|attachment| !selected_ids.contains(attachment.id.as_str()));
    }
    session
        .consumed_attachment_ids
        .extend(selected.iter().map(|attachment| attachment.id.clone()));
    Ok(selected)
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
    publish_core_semantic(
        state,
        session_id,
        WireEvent::WorkerActivity {
            session_id: session_id.to_string(),
            context_id: context_id.to_string(),
            worker_id: worker_id.to_string(),
            turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
            turn_event_id: turn_ref.map(|value| value.event_id),
            event,
        },
    );
}

fn mark_core_command_accepted(state: &AppState, session_id: &str, command_id: &str) {
    let turn_id = state.sessions.lock().ok().and_then(|mut sessions| {
        let session = sessions.get_mut(session_id)?;
        for turn in &mut session.turns {
            if let Some(entry) = turn
                .user_entries
                .iter_mut()
                .find(|entry| entry.command_id.as_deref() == Some(command_id))
            {
                entry.delivery_state = Some(ChatCommandDeliveryState::CoreAccepted);
                return Some(turn.turn_id.clone());
            }
        }
        None
    });
    let Some(turn_id) = turn_id else {
        publish_core_semantic(
            state,
            session_id,
            WireEvent::HostError {
                message: format!("core_command_acceptance_without_intent:{command_id}"),
            },
        );
        return;
    };

    let mut extra = BTreeMap::new();
    extra.insert("kind".to_string(), json!("command_delivery"));
    extra.insert("command_id".to_string(), json!(command_id));
    extra.insert("delivery_state".to_string(), json!("core_accepted"));
    let persist_result = current_session_store(state).and_then(|store| {
        store.append_history_record(
            session_id,
            &ChatHistoryRecord::Event {
                role: ChatHistoryRole::System,
                turn_id,
                created_at_ms: now_ms_i64(),
                kind: ChatHistoryEventKind::RuntimeNotice,
                content: "Core accepted command.".to_string(),
                extra,
            },
        )?;
        persist_web_session(state, session_id)
    });
    if let Err(error) = persist_result {
        let _ = state.events.send(command_ack(
            command_id,
            CommandAckStatus::Accepted,
            Some(format!("core_acceptance_persist_pending:{error}")),
        ));
        return;
    }
    match finish_command_dedup(
        state,
        command_id,
        CommandDedupState::Committed {
            serialized_event: None,
            event: None,
        },
    ) {
        Ok(()) => {
            let _ = state
                .events
                .send(command_ack(command_id, CommandAckStatus::Committed, None));
        }
        Err(error) => {
            let _ = state.events.send(command_ack(
                command_id,
                CommandAckStatus::Accepted,
                Some(format!("command_terminal_persist_pending:{error}")),
            ));
        }
    }
}

fn activate_core_started_turn(
    state: &AppState,
    session_id: &str,
    worker_id: &str,
    command_id: Option<&str>,
) -> Option<WebTurn> {
    let mut sessions = state.sessions.lock().ok()?;
    let session = sessions.get_mut(session_id)?;
    let turn_id = if let Some(command_id) = command_id {
        session.turns.iter().rev().find_map(|turn| {
            let matches_command = turn
                .user_entries
                .iter()
                .any(|entry| entry.command_id.as_deref() == Some(command_id));
            (matches_command && turn.final_answer.is_none() && turn.completion.is_none())
                .then(|| turn.turn_id.clone())
        })?
    } else {
        session
            .pending_turn_id
            .clone()
            .or_else(|| session.active_turn_id.clone())?
    };

    let turn_index = session
        .turns
        .iter()
        .position(|turn| turn.turn_id == turn_id)?;

    if session.pending_turn_id.as_deref() == Some(turn_id.as_str()) {
        session.pending_turn_id = None;
    }
    session.active_turn_id = Some(turn_id);
    session.state = "working".to_string();
    session.reported_session_working_worker_count = None;
    session.turns[turn_index].state = "working".to_string();

    if let Some(worker) = session
        .workers
        .iter_mut()
        .find(|worker| worker.worker_id == worker_id)
    {
        worker.state = "working".to_string();
    }

    Some(session.turns[turn_index].clone())
}
fn handle_scoped_worker_event(
    state: &AppState,
    session_id: &str,
    context_id: &str,
    worker_id: &str,
    event: CoreSessionWorkerEvent,
) {
    match event {
        CoreSessionWorkerEvent::CommandAccepted { command_id } => {
            mark_core_command_accepted(state, session_id, &command_id);
        }
        CoreSessionWorkerEvent::TurnStarted { command_id } => {
            if let Some(turn) =
                activate_core_started_turn(state, session_id, worker_id, command_id.as_deref())
            {
                publish_core_semantic(
                    state,
                    session_id,
                    WireEvent::TurnStarted {
                        session_id: session_id.to_string(),
                        context_id: context_id.to_string(),
                        worker_id: worker_id.to_string(),
                        turn,
                    },
                );
            }
        }
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
                    let reported_session_count = event
                        .payload
                        .get("global")
                        .and_then(|global| global.get("session_working_worker_count"))
                        .and_then(Value::as_u64)
                        .map(|count| count as usize);

                    if let Some(reported_session_count) = reported_session_count {
                        if let Ok(mut sessions) = state.sessions.lock() {
                            if let Some(session) = sessions.get_mut(session_id) {
                                session.reported_session_working_worker_count =
                                    Some(reported_session_count);
                            }
                        }
                    }
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
                            publish_core_semantic(
                                state,
                                session_id,
                                WireEvent::ToolRepoUpdated {
                                    session_id: session_id.to_string(),
                                    tools,
                                },
                            );
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
                publish_core_semantic(
                    state,
                    session_id,
                    WireEvent::CoreTopic {
                        turn_id: turn_ref.as_ref().map(|value| value.turn_id.clone()),
                        turn_event_id: turn_ref.map(|value| value.event_id),
                        event: wire_payload,
                    },
                );
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
        CoreSessionWorkerEvent::UnconsumedSupplements { supplements } => {
            if let Ok(mut sessions) = state.sessions.lock() {
                if let Some(session) = sessions.get_mut(session_id) {
                    session
                        .pending_unconsumed_supplements
                        .extend(supplements.clone());
                }
            }
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({ "kind": "unconsumed_supplements", "supplements": supplements }),
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
            let should_resubmit_unconsumed_supplements = outcome.stop_reason.is_none();
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
                            session.pending_turn_id = None;
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
                            let reported_session_working =
                                session.reported_session_working_worker_count.take();
                            if reported_session_working.unwrap_or(0) == 0 {
                                for worker in &mut session.workers {
                                    if worker.state == "working" {
                                        worker.state = "ready".to_string();
                                    }
                                }
                            }
                            session.state =
                                if session.workers.iter().any(|worker| worker.state == "error") {
                                    "error"
                                } else if session
                                    .workers
                                    .iter()
                                    .all(|worker| worker.state == "stopped")
                                {
                                    "stopped"
                                } else if reported_session_working.unwrap_or(0) > 0
                                    || session
                                        .workers
                                        .iter()
                                        .any(|worker| worker.state == "working")
                                {
                                    "working"
                                } else {
                                    "ready"
                                }
                                .to_string();
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
            publish_core_semantic(
                state,
                session_id,
                WireEvent::TurnFinished {
                    session_id: session_id.to_string(),
                    turn_id,
                    outcome: json!({ "text": outcome.text, "message_id": message_id, "completion": completion }),
                },
            );
            if should_resubmit_unconsumed_supplements {
                resubmit_unconsumed_supplements(state, session_id, context_id, worker_id);
            }
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

fn resubmit_unconsumed_supplements(
    state: &AppState,
    session_id: &str,
    context_id: &str,
    worker_id: &str,
) {
    let supplements = state
        .sessions
        .lock()
        .ok()
        .and_then(|mut sessions| {
            sessions
                .get_mut(session_id)
                .map(|session| std::mem::take(&mut session.pending_unconsumed_supplements))
        })
        .unwrap_or_default();
    if supplements.is_empty() {
        return;
    }
    let text = supplements.join("\n\n");
    match submit_turn(state, session_id, text.clone()) {
        Ok(turn) => {
            publish_core_semantic(
                state,
                session_id,
                WireEvent::TurnUpdated {
                    session_id: session_id.to_string(),
                    turn,
                },
            );
        }
        Err(error) => {
            if let Ok(mut sessions) = state.sessions.lock() {
                if let Some(session) = sessions.get_mut(session_id) {
                    let mut retained = supplements;
                    retained.append(&mut session.pending_unconsumed_supplements);
                    session.pending_unconsumed_supplements = retained;
                }
            }
            emit_worker_activity(
                state,
                session_id,
                context_id,
                worker_id,
                json!({
                    "kind": "unconsumed_supplements_resubmit_failed",
                    "error": error,
                    "text": text,
                }),
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
            .chain(std::iter::once(WebRuntimeOption {
                key: "TIMEM_MAX_ROUNDS".to_string(),
                value: round_budget_value(settings.max_rounds),
                applies_to: "new_sessions",
            }))
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
            bind_host: web_bind_host(state.public_access).to_string(),
            public_access: state.public_access,
            mem,
            runtime_options,
            session_env_defaults,
            workspace_dirs,
            mcp_servers: current_mem_state(state)
                .map(|mem| mcp_reports(&mem))
                .unwrap_or_default(),
        },
        sessions,
    }
}

fn validate_web_space_name(space: &str) -> Result<(), String> {
    let trimmed = space.trim();
    if trimmed.is_empty() {
        return Err("mem_space_empty".to_string());
    }
    if trimmed != space || trimmed.len() > 128 {
        return Err("mem_space_invalid".to_string());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("mem_space_invalid".to_string());
    }
    if trimmed.contains("..")
        || Path::new(trimmed).is_absolute()
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err("mem_space_must_be_name_not_path".to_string());
    }
    Ok(())
}

fn validate_web_mem_directory(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("mem_path_empty".to_string());
    }
    if !path.is_absolute() {
        return Err("mem_path_must_be_absolute".to_string());
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        return Err("mem_path_invalid".to_string());
    }
    if path.as_os_str().to_string_lossy().len() > 4096 {
        return Err("mem_path_invalid".to_string());
    }
    let space = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "mem_path_invalid".to_string())?;
    validate_web_space_name(space).map_err(|_| "mem_path_invalid".to_string())?;
    Ok(path.to_path_buf())
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
        ..
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
        let mut env = std::env::vars().collect::<HashMap<_, _>>();
        for key in RETIRED_SESSION_ENV_KEYS {
            env.remove(*key);
        }
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
        let reminder_tips_config =
            agent_core::load_reminder_tips_config(&agent_core::default_config_root());
        let session_store =
            SessionStore::new(RuntimeDataLayout::new(&data_dir, &space).memory_dir());
        if let Ok(sessions) = session_store.list_sessions() {
            if let Some(stored) = sessions.into_iter().find(|session| {
                !session.session_id.trim().is_empty() && Path::new(&session.current_dir).is_dir()
            }) {
                let cached_env = sanitize_restored_session_env(if stored.env.is_empty() {
                    stored.env_overrides.unwrap_or_default()
                } else {
                    stored.env
                });
                env.extend(cached_env);
            }
        }
        let config = model_service_config_for_web_launch(launch, &env)?;
        let response_protocol = launch
            .response_protocol
            .as_deref()
            .or_else(|| env.get("TIMEM_RESPONSE_PROTOCOL").map(String::as_str))
            .map(ResponseProtocolKind::from_name)
            .unwrap_or_default();
        let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
        let workspace_dirs =
            load_workspace_dirs_from_path(&agent_core::workspace_config_file(&data_dir))
                .into_iter()
                .map(PathBuf::from)
                .collect();
        Ok(Self {
            settings: Arc::new(Mutex::new(RuntimeSettings {
                config: ModelServiceConfig {
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
                max_rounds: env
                    .get("TIMEM_MAX_ROUNDS")
                    .and_then(|value| parse_round_budget(value).ok())
                    .unwrap_or(agent_core::UNLIMITED_ROUND_BUDGET),
            })),
            data_dir,
            initial_space: space,
            env: env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            current_dir,
            workspace_dirs,
            reminder_tips_config,
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
        core.set_max_rounds(settings.max_rounds);
        core.set_reminder_tips_config(self.reminder_tips_config.clone());
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
            if value.trim().is_empty() && key != "TIMEM_API_KEY" {
                return Err(format!("empty_session_env_value:{key}"));
            }
            if !SESSION_ENV_KEYS.contains(&key.as_str()) {
                return Err(format!("unsupported_session_env_key:{key}"));
            }
        }

        if let Some(base_url) = env_overrides.get("TIMEM_BASE_URL") {
            apply_session_runtime_field(
                &mut settings,
                agent_core::RuntimeConfigField::BaseUrl,
                base_url,
            )?;
        }
        for key in [
            "TIMEM_MODEL",
            "TIMEM_API_PROTOCOL",
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
        if let Some(value) = env_overrides.get("TIMEM_MAX_ROUNDS") {
            settings.max_rounds = parse_round_budget(value)?;
        }
        if let Some(base_url) = env_overrides.get("TIMEM_BASE_URL") {
            apply_session_runtime_field(
                &mut settings,
                agent_core::RuntimeConfigField::BaseUrl,
                base_url,
            )?;
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
            if value.is_empty() {
                settings.config.api_key.clear();
            } else {
                validate_api_key(value).map_err(|_| "invalid_session_api_key".to_string())?;
                settings.config.api_key = value.clone();
            }
        }
        for key in [
            "TIMEM_ENABLE_THINKING",
            "TIMEM_REASONING_EFFORT",
            "TIMEM_STREAM",
            "TIMEM_OPENAI_CACHE_MODE",
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
        env.insert(
            "TIMEM_MAX_ROUNDS".to_string(),
            round_budget_value(settings.max_rounds),
        );
        env.insert("TIMEM_API_KEY".to_string(), settings.config.api_key.clone());
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
        env.insert(
            "TIMEM_OPENAI_CACHE_MODE".to_string(),
            settings
                .config
                .openai_compatible
                .cache_mode
                .label()
                .to_string(),
        );
        env
    }

    fn resolve_workspace(&self, requested: Option<&str>) -> Result<PathBuf, String> {
        let selected = match requested {
            Some(path) => {
                std::fs::canonicalize(path).map_err(|_| "workspace_not_found".to_string())?
            }
            None => std::fs::canonicalize(&self.current_dir)
                .map_err(|_| "workspace_not_found".to_string())?,
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

fn model_service_config_for_web_launch(
    launch: &WebLaunchOptions,
    env: &HashMap<String, String>,
) -> Result<ModelServiceConfig, String> {
    model_service_config_from_sources_allow_missing_api_key(&launch.model_service_source(), env)
}

const SESSION_ENV_KEYS: &[&str] = &[
    "TIMEM_MODEL",
    "TIMEM_API_PROTOCOL",
    "TIMEM_RESPONSE_PROTOCOL",
    "TIMEM_BASE_URL",
    "TIMEM_API_KEY",
    "TIMEM_TIMEOUT",
    "TIMEM_MAX_LLM_INPUT",
    "TIMEM_MAX_LLM_OUTPUT",
    "TIMEM_MAX_ROUNDS",
    "TIMEM_BASH_APPROVAL",
    "TIMEM_WORK_INSTRUCTIONS",
    "TIMEM_ENABLE_THINKING",
    "TIMEM_REASONING_EFFORT",
    "TIMEM_STREAM",
    "TIMEM_OPENAI_CACHE_MODE",
];

fn parse_round_budget(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("unlimited") {
        return Ok(agent_core::UNLIMITED_ROUND_BUDGET);
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|rounds| (1..=10_000).contains(rounds))
        .ok_or_else(|| "invalid_session_max_rounds".to_string())
}

fn round_budget_value(max_rounds: u32) -> String {
    if max_rounds == agent_core::UNLIMITED_ROUND_BUDGET {
        "unlimited".to_string()
    } else {
        max_rounds.to_string()
    }
}

const RETIRED_SESSION_ENV_KEYS: &[&str] = &["TIMEM_GATEWAY_PROVIDER"];

fn sanitize_restored_session_env(mut env: BTreeMap<String, String>) -> BTreeMap<String, String> {
    for key in RETIRED_SESSION_ENV_KEYS {
        env.remove(*key);
    }
    env
}

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
            model: settings.config.model.clone(),
            api_protocol: settings.config.api_protocol.label().to_string(),
            response_protocol: settings.config.response_protocol.name().to_string(),
            base_url: settings.config.base_url.clone(),
            timeout_secs: settings.config.timeout_secs,
            max_llm_input_tokens: settings.config.max_llm_input_tokens,
            max_llm_output_tokens: settings.config.max_llm_output_tokens,
            max_rounds: round_budget_value(settings.max_rounds),
            bash_approval: agent_core::bash_approval_mode_label(settings.bash_approval_mode)
                .to_string(),
            work_instructions: agent_core::work_instruction_mode_label(
                settings.work_instruction_mode,
            )
            .to_string(),
            api_key_configured: !settings.config.api_key.is_empty(),
        }
    }
}

fn session_env_values(settings: &RuntimeSettings) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
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
            "TIMEM_MAX_ROUNDS".to_string(),
            round_budget_value(settings.max_rounds),
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
    env.insert(
        "TIMEM_OPENAI_CACHE_MODE".to_string(),
        settings
            .config
            .openai_compatible
            .cache_mode
            .label()
            .to_string(),
    );
    env
}

fn session_cached_env_values(settings: &RuntimeSettings) -> BTreeMap<String, String> {
    let mut env = session_env_values(settings);
    env.insert("TIMEM_API_KEY".to_string(), settings.config.api_key.clone());
    env
}

#[derive(Debug)]
struct WebLaunchOptions {
    port: Option<u16>,
    public_access: bool,
    public_host: Option<String>,
    space: Option<String>,
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
            public_access: false,
            public_host: None,
            space: None,
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
                "--public" => {
                    options.public_access = true;
                    index += 1;
                }
                "--public-host" => string(&mut options.public_host)?,
                "--space" => string(&mut options.space)?,
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

    fn model_service_source(&self) -> ModelServiceConfigSource {
        ModelServiceConfigSource {
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
            openai_cache_mode: None,
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

fn should_auto_open_browser() -> bool {
    let is_ssh = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()));

    #[cfg(target_os = "linux")]
    let has_graphical_session = std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let has_graphical_session = true;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let has_graphical_session = false;

    browser_auto_open_allowed_for(is_ssh, has_graphical_session)
}

fn browser_auto_open_allowed_for(is_ssh: bool, has_graphical_session: bool) -> bool {
    !is_ssh && has_graphical_session
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

async fn bind_web_listener(
    requested_port: Option<u16>,
    public_access: bool,
) -> Result<TcpListener, String> {
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
    let bind_ip = if public_access {
        Ipv4Addr::UNSPECIFIED
    } else {
        Ipv4Addr::LOCALHOST
    };
    for port in ports {
        let address = SocketAddr::new(IpAddr::V4(bind_ip), port);
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

fn web_bind_host(public_access: bool) -> &'static str {
    if public_access {
        "0.0.0.0"
    } else {
        "127.0.0.1"
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
    println!("Timem Web\n\nUsage: timem-web [options]\n\nOptions:\n  --port <n>                   web port in {PORT_START}..={PORT_END}; default auto-select\n  --public                     bind to 0.0.0.0; browser/API/WebSocket/upload require the access token\n  --public-host <host>         advertised browser host; env TIMEM_PUBLIC_HOST; auto-detected when omitted\n  --no-open                    do not open the browser automatically\n  --space <name>               memory/audit space\n  --api-protocol <protocol>    model API wire protocol\n  --response-protocol <name>   model response protocol\n  --model <name>               model\n  --api-key <key>              API key (environment is safer)\n  --base-url <url>             model API base URL\n  --data-dir <path>            data root\n  --timeout <seconds>          model request timeout\n  --max-llm-input <n>          input context limit\n  --max-llm-output <n>         output limit\n  --bash-approval <mode>       ask|approve\n  --work-instructions <mode>   silent|ask|off\n");
}

fn public_access_url(configured_host: Option<&str>, port: u16, token: &str) -> Option<String> {
    let configured_host = match configured_host {
        Some(host) => {
            let host = host.trim();
            validate_public_host(host)?;
            Some(host.to_string())
        }
        None => std::env::var("TIMEM_PUBLIC_HOST")
            .ok()
            .map(|host| host.trim().to_string())
            .filter(|host| !host.is_empty()),
    };
    let host = configured_host.or_else(detect_advertised_host)?;
    validate_public_host(&host)?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    Some(format!("http://{host}:{port}/?token={token}"))
}

fn validate_public_host(host: &str) -> Option<()> {
    let host = host.trim();
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    if host.chars().any(|ch| {
        ch.is_ascii_control()
            || ch.is_ascii_whitespace()
            || matches!(ch, '/' | '\\' | '?' | '#' | '@')
    }) {
        return None;
    }
    Some(())
}

fn detect_advertised_host() -> Option<String> {
    // UDP connect selects the local route without sending application data.
    // This avoids shelling out to platform-specific interface tools.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_unspecified() || ip.is_loopback() {
        None
    } else {
        Some(ip.to_string())
    }
}

#[cfg(test)]
#[path = "../tests/unit/web_host_tests.rs"]
mod tests;

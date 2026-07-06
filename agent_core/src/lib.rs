use rusqlite::{params_from_iter, types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod audit;
pub mod capability;
#[path = "../../resources/capabilities/tools/capmgr.rs"]
pub mod capmgr;
pub use capability::CapabilityHostProfile;
use capability::CapabilityRegistry;
pub mod config_edit;
pub mod config_report;
pub mod context;
pub mod context_policy;
pub mod data_layout;
pub mod executor;
pub mod host;
#[path = "../../resources/capabilities/tools/memmgr.rs"]
pub mod memmgr;
mod notification;
pub mod profiler;
pub mod prompt_cache;
pub mod prompt_render;
pub mod prompt_spec;
pub mod provider;
pub mod provider_config;
pub mod provider_transport;
pub mod redaction;
pub mod response_protocol;
pub mod retry_policy;
pub mod runtime_context;
#[path = "../../resources/capabilities/tools/self_tool.rs"]
pub mod self_tool;
pub mod session_runtime;
pub mod session_worker;
#[path = "../../resources/capabilities/tools/run_bash.rs"]
pub mod shell_exec;
#[path = "../../resources/capabilities/tools/shell_job_status.rs"]
pub mod shell_job_status;
pub mod status_summary;
pub mod status_view;
pub mod tool_jobs;
#[path = "../../resources/capabilities/tools/registry.rs"]
pub(crate) mod tool_registry;
pub mod work_instructions;
pub mod workspace;
pub use audit::{
    append_audit_event, host_start_audit_event, max_llm_output_increased_audit_event,
    model_repair_request_audit_event, model_retry_audit_event, read_audit_doc,
    round_limit_audit_event, stale_context_choice_audit_event, turn_error_audit_event,
    turn_final_audit_event, turn_start_audit_event, user_approval_audit_event,
    user_supplement_audit_event,
};
pub use config_edit::{
    apply_runtime_config_value, bash_approval_mode_from_sources, capabilities_dir_from_sources,
    parse_token_count, runtime_config_apply_report, runtime_config_field_value,
    runtime_config_menu_report, work_instruction_mode_label, RuntimeConfigApplyError,
    RuntimeConfigApplyMessage, RuntimeConfigApplyMessageKind, RuntimeConfigApplyReport,
    RuntimeConfigEffect, RuntimeConfigField, RuntimeConfigMenuItem, RuntimeConfigMenuReport,
    RUNTIME_CONFIG_FIELDS,
};
pub use config_report::{
    bash_approval_mode_label, runtime_config_report, RuntimeConfigReport, RuntimeConfigReportInput,
    RuntimeConfigReportItem, RuntimeConfigReportRow, RuntimeConfigRowKind, RuntimeConfigSection,
};
pub use context::estimate_prompt_context_tokens;
pub use context_policy::{
    stale_context_decision_request, stale_context_prompt_needed, StaleContextDecisionRequest,
    StaleContextPolicy, DEFAULT_STALE_CONTEXT_IDLE, DEFAULT_STALE_CONTEXT_TOKEN_THRESHOLD,
};
pub use data_layout::{
    default_data_root, layout_for_space, workspace_config_file, RuntimeDataLayout,
};
pub use host::{
    core_initialized_topic_event, core_initialized_topic_event_with_worker,
    normalize_user_supplements, resolve_topic_reply, session_worker_default_display_name,
    topic_event_status_hint, work_instruction_load_topic_event, CoreActionTopic,
    CoreDynamicContextSummary, CoreGlobalWorkerStatus, CoreHostDecisionRequestTopic,
    CoreLifecycleEvent, CoreLifecycleTopic, CoreModelResponseTopic, CoreSessionState,
    CoreSessionWorkerIdentity, CoreSessionWorkerWorkspace, CoreTopic, CoreTopicEvent,
    CoreTopicEventSink, CoreTopicStatusHint, CoreWorkInstructionLoadTopic, HostDecision,
    HostDecisionDefault, HostDecisionRequest, LongRunningCommandContinueRequest, NoopTurnUi,
    OutputExpansionRequest, OutputExpansionResolution, RoundLimitDecisionRequest,
    RoundLimitResolution, StoppedTurn, TopicReply, TopicReplyError, TurnInput, TurnOutcome,
    TurnStopDetail, TurnStopReason, TurnStopSummary, TurnUi, CORE_TOPIC_ACTION,
    CORE_TOPIC_LIFECYCLE, CORE_TOPIC_LONG_RUNNING_COMMAND_REQUEST, CORE_TOPIC_MODEL_RESPONSE,
    CORE_TOPIC_OUTPUT_EXPAND_REQUEST, CORE_TOPIC_ROUND_LIMIT_REQUEST,
    CORE_TOPIC_STALE_CONTEXT_REQUEST, CORE_TOPIC_USER_APPROVAL_REQUEST,
    CORE_TOPIC_WORK_INSTRUCTION_LOAD, DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT,
};
use notification::CoreNotification;
pub use notification::{CoreActionKind, CoreMemoryActivity};
pub use profiler::{
    collect_storage_profile, profile_cache_hit_percent_tenths, profile_wait_per_1k_output,
    runtime_profile_report, ModelProfile, ModelProfileReport, RuntimeProfileReport,
    RuntimeProfiler, StorageProfile,
};
pub use prompt_cache::{
    plan_incremental_cache, plan_prompt_cache, prompt_parts_from_rendered_prompt,
    split_old_and_new_delta, split_prompt, stable_text_fingerprint, CacheControl, PromptBlock,
    PromptBlockRole, PromptParts,
};
pub use provider::{
    build_provider_request, default_api_protocol_for_provider, default_base_url_for_provider,
    default_model_for_provider, interpret_provider_http_response, is_default_base_url_for_provider,
    is_default_model_for_provider, known_default_base_url_for_provider, parse_api_protocol,
    parse_provider_response, plan_structured_output, prepare_provider_http_request,
    prepare_provider_request, prompt_cache_plan_audit, provider_http_error_message,
    provider_prompt_blocks, provider_request_audit_event, provider_response_audit_event,
    ApiProtocol, PreparedProviderHttpRequest, PreparedProviderRequest, ProviderCacheControl,
    ProviderConfig, ProviderHttpResponseInterpretation, ProviderPromptBlock, ProviderPromptRole,
    StructuredOutputHint,
};
pub use provider_config::{
    provider_config_from_sources, validate_provider_api_key, LocalLLMKeyFile, ProviderConfigSource,
};
pub use provider_transport::{call_model, call_model_with_cancel, ProviderModelClient};
pub use redaction::{redact_value, REDACTED};
pub use response_protocol::ResponseProtocolKind;
use response_protocol::{ActionGroupOrder, ParsedAction, ParsedActionGroup};
pub use retry_policy::{
    is_retryable_model_system_error, model_retry_decision, ModelCallOutcome, ModelRetryDecision,
    ModelSystemRetryPolicy, DEFAULT_MODEL_SYSTEM_ERROR_RETRIES,
    DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY,
};
pub use runtime_context::{
    format_supporting_context, local_time_label, runtime_time_context, supporting_context,
    turn_supporting_context, LocalTimeParts, SupportingContextInput,
};
use self_tool::{SelfToolAbout, SelfToolPaths, SelfToolProcess, SelfToolState};
pub use session_runtime::{
    cancelled_turn_result, run_session_turn, run_session_turn_with_model_client, ModelClient,
};
pub use session_worker::{
    CoreSessionWorker, CoreSessionWorkerConfig, CoreSessionWorkerEvent, CoreSessionWorkerHandle,
    CoreSessionWorkerRuntime,
};
use shell_exec::FileShellJobStore;
pub use shell_exec::ShellJobRecord;
pub use status_summary::{
    context_bar_filled, context_percent, meaningful_latest_usage, runtime_token_status_view,
    token_status_summary, RuntimeTokenStatusView, TokenStatusSummary, TokenUsageBreakdown,
};
pub use status_view::{
    compact_runtime_status_text, runtime_active_elapsed_secs, runtime_retry_status_view,
    HostStatusLevel, HostStatusMessage, ModelDirection, RuntimeRetryStatus, RuntimeRetryStatusView,
    RuntimeStatusSnapshot,
};
use tool_jobs::FileToolJobStore;
pub use work_instructions::{
    combine_additional_contexts, discover_work_instruction_files, load_work_instruction_context,
    parse_work_instruction_mode, work_instruction_load_report, work_instruction_load_request,
    work_instruction_mode_from_sources, WorkInstructionContext, WorkInstructionFile,
    WorkInstructionLoadMessage, WorkInstructionLoadMessageKind, WorkInstructionLoadMode,
    WorkInstructionLoadReport, WorkInstructionLoadRequest, WorkInstructionLoadStatus,
    WORK_INSTRUCTION_FILENAMES,
};
pub use workspace::{
    apply_workspace_command_to_path, load_workspace_dirs_from_path, normalize_workspace_dir,
    save_workspace_dirs_to_path, workspace_menu_report, workspace_reference_context,
    WorkspaceChange, WorkspaceCommand, WorkspaceCommandMessage, WorkspaceCommandMessageKind,
    WorkspaceCommandOutcome, WorkspaceCommandReport, WorkspaceMenuReport, WorkspaceState,
    WorkspaceUnchangedReason,
};

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreProfile {
    pub name: String,
    pub provider: String,
    pub model: String,
}
impl CoreProfile {
    pub fn label(&self) -> String {
        format!("{}:{}:{}", self.name, self.provider, self.model)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageStats {
    pub llm_calls: u32,
    pub repair_calls: u32,
    pub tool_calls: u32,
    pub mem_reads: u32,
    pub mem_writes: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cached_tokens: u32,
    pub cache_created_tokens: u32,
    pub shrunk_tokens: u32,
}
impl UsageStats {
    pub fn zero() -> Self {
        Self {
            llm_calls: 0,
            repair_calls: 0,
            tool_calls: 0,
            mem_reads: 0,
            mem_writes: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_created_tokens: 0,
            shrunk_tokens: 0,
        }
    }
    pub fn add(&mut self, other: &UsageStats) {
        self.llm_calls += other.llm_calls;
        self.repair_calls += other.repair_calls;
        self.tool_calls += other.tool_calls;
        self.mem_reads += other.mem_reads;
        self.mem_writes += other.mem_writes;
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        self.cached_tokens += other.cached_tokens;
        self.cache_created_tokens += other.cache_created_tokens;
        self.shrunk_tokens += other.shrunk_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmResponse {
    pub content: String,
    pub model_name: String,
    pub usage: UsageStats,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnFinal {
    pub final_answer: String,
    pub stats: UsageStats,
    pub profile_label: String,
    pub repair_issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_summary: Option<TurnStopSummary>,
}

fn llm_final_answer_slice_text(final_answer: &str) -> String {
    format!("All previous pending open tasks are completed.  Final Answer:\n{final_answer}")
}

fn normalize_assistant_speaker_name(name: &str) -> String {
    let clean = name
        .trim()
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if clean.is_empty() {
        "TIMEM_ASSISTANT".to_string()
    } else {
        clean
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub action: String,
    pub command: String,
    pub reason: String,
    pub risk: String,
    pub intent: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BashApprovalMode {
    Ask,
    Approve,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CoreStep {
    NeedModel {
        prompt: String,
        rounds_remaining: u32,
    },
    NeedsUserApproval {
        request: ApprovalRequest,
    },
    RoundLimitReached {
        max_rounds: u32,
    },
    Final(TurnFinal),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptDelta {
    pub delta_id: String,
    pub time_ms: i64,
    pub(crate) slices: Vec<PromptSlice>,
    #[serde(default)]
    pub hidden_slice_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PromptSlice {
    pub(crate) delta_id: String,
    pub(crate) slice_id: String,
    pub(crate) prompt_type: String,
    pub(crate) time_ms: i64,
    pub(crate) text: String,
    pub(crate) slice_index: usize,
    pub(crate) slice_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default)]
    pub version: u64,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScratchNoteRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub scratch_type: String,
    pub label: String,
    pub content: String,
    pub prompt_delta_ids: Vec<String>,
    pub prompt_slice_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScratchContextOffload {
    content: String,
    delta_ids: Vec<String>,
    slice_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistoryRecord {
    pub session: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    pub user_input: String,
    pub assistant_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingApproval {
    request: ApprovalRequest,
    approved_action: PendingApprovedAction,
    intent: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingApprovedAction {
    RunBash {
        command: String,
        background: bool,
        timeout_ms: i64,
        interval_ms: Option<u64>,
        once_timeout_ms: u64,
        session_id: String,
        turn_id: String,
    },
}

impl PendingApprovedAction {
    fn command(&self) -> &str {
        match self {
            PendingApprovedAction::RunBash { command, .. } => command,
        }
    }

    fn audit_input(&self, approval_id: &str, risk: &str, reason: &str) -> Value {
        match self {
            PendingApprovedAction::RunBash {
                command,
                background,
                timeout_ms,
                interval_ms,
                once_timeout_ms,
                session_id,
                turn_id,
            } => json!({
                "command": command,
                "background": background,
                "timeout_ms": timeout_ms,
                "interval_ms": interval_ms,
                "loop_timeout_ms": if interval_ms.is_some() { Some(*timeout_ms) } else { None },
                "once_timeout_ms": if interval_ms.is_some() { Some(*once_timeout_ms) } else { None },
                "session_id": session_id,
                "turn_id": turn_id,
                "approval_id": approval_id,
                "risk": risk,
                "reason": reason,
            }),
        }
    }
}

const PROMPT_SLICE_TEXT_LIMIT: usize = 12_000;
const DEFAULT_ROUND_BUDGET: u32 = 50;
const MEM_GUARD_WAIT_STEP: Duration = Duration::from_millis(25);
const MEM_GUARD_TIMEOUT: Duration = Duration::from_secs(30);
const MEM_GUARD_STALE_AFTER: Duration = Duration::from_secs(60 * 60 * 6);

#[derive(Debug, Clone)]
pub struct MemGuard {
    lock_dir: PathBuf,
}

impl MemGuard {
    pub fn for_memory_dir(memory_dir: impl AsRef<Path>) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir.as_ref()).to_path_buf();
        Self::for_space_dir(space_dir)
    }

    pub fn for_space_dir(space_dir: impl AsRef<Path>) -> Self {
        let space_dir = fs::canonicalize(space_dir.as_ref())
            .unwrap_or_else(|_| space_dir.as_ref().to_path_buf());
        Self {
            lock_dir: space_dir.join(".guard").join("mem.lock.d"),
        }
    }

    pub fn for_audit_file(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let space_dir = path
            .parent()
            .and_then(|parent| {
                if parent.file_name().and_then(|name| name.to_str()) == Some("audit") {
                    parent.parent()
                } else {
                    Some(parent)
                }
            })
            .unwrap_or_else(|| Path::new("."));
        Self::for_space_dir(space_dir)
    }

    pub fn with_read<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        self.with_lock(f)
    }

    pub fn with_write<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        self.with_lock(f)
    }

    fn with_lock<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        let _lock = self.acquire()?;
        Ok(f())
    }

    fn acquire(&self) -> Result<MemGuardLock, String> {
        if let Some(parent) = self.lock_dir.parent() {
            fs::create_dir_all(parent).map_err(|_| "mem_guard_create_failed".to_string())?;
        }
        let started = Instant::now();
        loop {
            match fs::create_dir(&self.lock_dir) {
                Ok(()) => {
                    let owner = json!({
                        "pid": std::process::id(),
                        "created_at_ms": now_ms(),
                    });
                    let _ = fs::write(
                        self.lock_dir.join("owner.json"),
                        serde_json::to_string_pretty(&owner).unwrap_or_default(),
                    );
                    return Ok(MemGuardLock {
                        lock_dir: self.lock_dir.clone(),
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if self.is_stale_lock() {
                        let _ = fs::remove_dir_all(&self.lock_dir);
                        continue;
                    }
                    if started.elapsed() >= MEM_GUARD_TIMEOUT {
                        return Err("mem_guard_timeout".to_string());
                    }
                    thread::sleep(MEM_GUARD_WAIT_STEP);
                }
                Err(_) => return Err("mem_guard_lock_failed".to_string()),
            }
        }
    }

    fn is_stale_lock(&self) -> bool {
        fs::metadata(&self.lock_dir)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|age| age >= MEM_GUARD_STALE_AFTER)
            .unwrap_or(false)
    }
}

#[derive(Debug)]
struct MemGuardLock {
    lock_dir: PathBuf,
}

impl Drop for MemGuardLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.lock_dir);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ActionExecution {
    Completed(String),
    NeedsApproval(PendingApproval),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongRunningCommandDecision {
    Continue,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongRunningCommandStatus {
    pub action: String,
    pub command: String,
    pub elapsed: Duration,
    pub timeout_ms: Option<i64>,
}

pub trait ActionRuntime {
    fn should_cancel(&mut self) -> bool;

    fn on_long_running_command(
        &mut self,
        _status: &LongRunningCommandStatus,
    ) -> LongRunningCommandDecision {
        LongRunningCommandDecision::Continue
    }
}

pub(crate) struct CancelOnlyActionRuntime<'a> {
    should_cancel: &'a mut dyn FnMut() -> bool,
}

impl<'a> CancelOnlyActionRuntime<'a> {
    pub(crate) fn new(should_cancel: &'a mut dyn FnMut() -> bool) -> Self {
        Self { should_cancel }
    }
}

impl ActionRuntime for CancelOnlyActionRuntime<'_> {
    fn should_cancel(&mut self) -> bool {
        (self.should_cancel)()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditDocument {
    version: u32,
    turns: Vec<ActionAuditTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditTurn {
    turn_id: String,
    started_at_ms: i64,
    user_question: String,
    interactions: Vec<ActionAuditInteraction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditInteraction {
    round: u32,
    actions: Vec<ActionAuditEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActionAuditEntry {
    time_ms: i64,
    round: u32,
    action: String,
    intent: String,
    status: String,
    input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_summary: Option<String>,
}

#[derive(Debug, Clone)]
struct FileActionAuditStore {
    file: PathBuf,
    guard: MemGuard,
}

impl FileActionAuditStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir);
        let file = space_dir.join("audit").join("action_audit.json");
        Self {
            file,
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    fn begin_turn(&self, turn_id: &str, started_at_ms: i64, user_question: &str) {
        let _ = self.guard.with_write(|| {
            let mut doc = self.read_doc_unlocked();
            if doc.turns.iter().any(|turn| turn.turn_id == turn_id) {
                return;
            }
            doc.turns.push(ActionAuditTurn {
                turn_id: turn_id.to_string(),
                started_at_ms,
                user_question: user_question.to_string(),
                interactions: Vec::new(),
            });
            self.write_doc_unlocked(&doc);
        });
    }

    fn record_action(&self, entry: ActionAuditEntry, turn_id: &str, user_question: &str) {
        let _ = self.guard.with_write(|| {
            let mut doc = self.read_doc_unlocked();
            let turn_index =
                if let Some(index) = doc.turns.iter().position(|turn| turn.turn_id == turn_id) {
                    index
                } else {
                    doc.turns.push(ActionAuditTurn {
                        turn_id: turn_id.to_string(),
                        started_at_ms: now_ms(),
                        user_question: user_question.to_string(),
                        interactions: Vec::new(),
                    });
                    doc.turns.len().saturating_sub(1)
                };
            let turn = &mut doc.turns[turn_index];
            let interaction_index = if let Some(index) = turn
                .interactions
                .iter()
                .position(|interaction| interaction.round == entry.round)
            {
                index
            } else {
                turn.interactions.push(ActionAuditInteraction {
                    round: entry.round,
                    actions: Vec::new(),
                });
                turn.interactions.len().saturating_sub(1)
            };
            turn.interactions[interaction_index].actions.push(entry);
            self.write_doc_unlocked(&doc);
        });
    }

    fn read_doc_unlocked(&self) -> ActionAuditDocument {
        let Ok(text) = fs::read_to_string(&self.file) else {
            return Self::empty_doc();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| Self::empty_doc())
    }

    fn write_doc_unlocked(&self, doc: &ActionAuditDocument) {
        if let Some(parent) = self.file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(text) = serde_json::to_string_pretty(doc) else {
            return;
        };
        let _ = fs::write(&self.file, format!("{text}\n"));
    }

    fn empty_doc() -> ActionAuditDocument {
        ActionAuditDocument {
            version: 1,
            turns: Vec::new(),
        }
    }
}

fn space_dir_for_memory_dir(memory_dir: &Path) -> &Path {
    if memory_dir.file_name().and_then(|name| name.to_str()) == Some("memory") {
        memory_dir.parent().unwrap_or(memory_dir)
    } else {
        memory_dir
    }
}

fn default_self_tool_paths(memory_dir: &Path) -> SelfToolPaths {
    let space_dir = space_dir_for_memory_dir(memory_dir).to_path_buf();
    SelfToolPaths {
        space_dir: space_dir.clone(),
        memory_dir: memory_dir.to_path_buf(),
        memory_file: memory_dir.join("memory.jsonl"),
        scratch_file: memory_dir.join("scratch_notes.jsonl"),
        api_audit_file: space_dir.join("audit").join("api_audit.json"),
        action_audit_file: space_dir.join("audit").join("action_audit.json"),
    }
}

fn default_self_tool_about() -> SelfToolAbout {
    SelfToolAbout {
        name: "TimemAi".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        author: "TimemAi <phylimo@163.com>".to_string(),
        summary: "A lightweight local agent with Bash capability and multidimensional, time-aware memory.".to_string(),
        project: "https://github.com/moliam/TimemAi".to_string(),
        star_message: "Please star https://github.com/moliam/TimemAi".to_string(),
    }
}

fn default_self_tool_process() -> SelfToolProcess {
    SelfToolProcess {
        pid: std::process::id(),
        current_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        executable: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("timem")),
    }
}

#[derive(Debug)]
pub struct AgentCore {
    static_prompt: String,
    rendered_static_prompt: String,
    profile: CoreProfile,
    pub(crate) capabilities: CapabilityRegistry,
    response_protocol: ResponseProtocolKind,
    pub(crate) memory: FileMemoryStore,
    pub(crate) scratch: FileScratchStore,
    pub(crate) chat_history: FileChatHistoryStore,
    pub(crate) shell_jobs: FileShellJobStore,
    pub(crate) tool_jobs: FileToolJobStore,
    action_audit: FileActionAuditStore,
    pub(crate) self_tool: SelfToolState,
    deltas: Vec<PromptDelta>,
    max_llm_input_tokens: u32,
    last_observed_prompt_tokens: u32,
    configured_round_budget: u32,
    round_budget: u32,
    current_round: u32,
    pub(crate) current_stats: UsageStats,
    repair_attempted: bool,
    last_repair_issue: Option<String>,
    pending_approval: Option<PendingApproval>,
    pub(crate) bash_approval_mode: BashApprovalMode,
    current_action_turn_id: Option<String>,
    current_session_id: Option<String>,
    current_action_user_question: String,
    last_notifications: Vec<CoreNotification>,
    loaded_work_instruction_fingerprints: HashSet<String>,
    pending_next_turn_slices: Vec<(String, String)>,
    assistant_speaker_name: String,
}
impl AgentCore {
    pub fn new(
        static_prompt: impl Into<String>,
        profile: CoreProfile,
        memory_dir: impl AsRef<Path>,
    ) -> Self {
        let memory_dir = memory_dir.as_ref();
        let self_tool = SelfToolState::new(
            std::env::vars().collect::<BTreeMap<_, _>>(),
            default_self_tool_paths(memory_dir),
            default_self_tool_about(),
            default_self_tool_process(),
        );
        let static_prompt = static_prompt.into();
        let capabilities = CapabilityRegistry::builtin();
        let response_protocol = ResponseProtocolKind::default();
        let rendered_static_prompt = prompt_render::render_static_prompt(
            &static_prompt,
            &capabilities,
            response_protocol.suite(),
        );
        Self {
            static_prompt,
            rendered_static_prompt,
            profile,
            capabilities,
            response_protocol,
            memory: FileMemoryStore::new(memory_dir),
            scratch: FileScratchStore::new(memory_dir),
            chat_history: FileChatHistoryStore::new(memory_dir),
            shell_jobs: FileShellJobStore::new(memory_dir),
            tool_jobs: FileToolJobStore::new(memory_dir),
            action_audit: FileActionAuditStore::new(memory_dir),
            self_tool,
            deltas: Vec::new(),
            max_llm_input_tokens: 100_000,
            last_observed_prompt_tokens: 0,
            configured_round_budget: DEFAULT_ROUND_BUDGET,
            round_budget: DEFAULT_ROUND_BUDGET,
            current_round: 0,
            current_stats: UsageStats::zero(),
            repair_attempted: false,
            last_repair_issue: None,
            pending_approval: None,
            bash_approval_mode: BashApprovalMode::Ask,
            current_action_turn_id: None,
            current_session_id: None,
            current_action_user_question: String::new(),
            last_notifications: Vec::new(),
            loaded_work_instruction_fingerprints: HashSet::new(),
            pending_next_turn_slices: Vec::new(),
            assistant_speaker_name: "TIMEM_ASSISTANT".to_string(),
        }
    }

    pub fn set_assistant_speaker_name(&mut self, name: impl AsRef<str>) {
        self.assistant_speaker_name = normalize_assistant_speaker_name(name.as_ref());
    }

    pub fn assistant_speaker_name(&self) -> &str {
        &self.assistant_speaker_name
    }
    pub fn set_bash_approval_mode(&mut self, mode: BashApprovalMode) {
        self.bash_approval_mode = mode;
    }

    pub(crate) fn current_session_id(&self) -> String {
        self.current_session_id
            .clone()
            .unwrap_or_else(|| "default".to_string())
    }

    pub(crate) fn current_action_turn_id(&self) -> String {
        self.current_action_turn_id
            .clone()
            .unwrap_or_else(|| "unknown_turn".to_string())
    }

    pub(crate) fn cleanup_background_jobs_for_current_session(
        &mut self,
        reason: &str,
    ) -> Vec<String> {
        let session_id = self.current_session_id();
        let cancelled = self.shell_jobs.cancel_unfinished_for_session(&session_id);
        if !cancelled.is_empty() {
            self.append_delta(vec![(
                "result_of_llm_action".to_string(),
                format!(
                    "Action result: run_bash_background_cleanup\nreason: {}\nsession_id: {}\nterminated_jobs: {}",
                    reason,
                    session_id,
                    cancelled.join(",")
                ),
            )]);
        }
        cancelled
    }

    pub fn set_max_llm_input_tokens(&mut self, max_llm_input_tokens: u32) {
        self.max_llm_input_tokens = max_llm_input_tokens.max(3_000);
    }
    pub fn configure_runtime_from_host(
        &mut self,
        config: &ProviderConfig,
        bash_approval_mode: BashApprovalMode,
    ) {
        self.set_max_llm_input_tokens(config.max_llm_input_tokens);
        self.set_bash_approval_mode(bash_approval_mode);
    }
    pub fn apply_runtime_config_update(
        &mut self,
        config: &mut ProviderConfig,
        bash_approval_mode: &mut BashApprovalMode,
        work_instruction_mode: &mut WorkInstructionLoadMode,
        field: RuntimeConfigField,
        value: &str,
    ) -> Result<RuntimeConfigApplyReport, RuntimeConfigApplyError> {
        let effect = apply_runtime_config_value(
            config,
            bash_approval_mode,
            work_instruction_mode,
            field,
            value,
        )?;
        match effect {
            RuntimeConfigEffect::None => {}
            RuntimeConfigEffect::MaxInputChanged(tokens) => self.set_max_llm_input_tokens(tokens),
            RuntimeConfigEffect::BashApprovalChanged(mode) => self.set_bash_approval_mode(mode),
            RuntimeConfigEffect::WorkInstructionsChanged(_) => {}
        }
        Ok(runtime_config_apply_report(
            config,
            *bash_approval_mode,
            *work_instruction_mode,
            field,
            effect,
        ))
    }
    pub fn set_max_rounds(&mut self, max_rounds: u32) {
        self.configured_round_budget = max_rounds.max(1);
        self.round_budget = self.configured_round_budget;
    }
    fn refresh_rendered_static_prompt(&mut self) {
        self.rendered_static_prompt = prompt_render::render_static_prompt(
            &self.static_prompt,
            &self.capabilities,
            self.response_protocol.suite(),
        );
    }
    pub fn set_capability_registry(&mut self, capabilities: CapabilityRegistry) {
        self.capabilities = capabilities;
        self.refresh_rendered_static_prompt();
    }
    pub fn set_response_protocol(&mut self, protocol: ResponseProtocolKind) {
        self.response_protocol = protocol;
        self.refresh_rendered_static_prompt();
    }
    pub fn set_self_tool_state(&mut self, self_tool: SelfToolState) {
        self.self_tool = self_tool;
    }
    pub fn configure_self_tool_runtime(
        &mut self,
        env: BTreeMap<String, String>,
        paths: SelfToolPaths,
    ) {
        self.self_tool = SelfToolState::new(
            env,
            paths,
            default_self_tool_about(),
            default_self_tool_process(),
        );
    }
    pub fn profile(&self) -> &CoreProfile {
        &self.profile
    }
    pub fn response_protocol_name(&self) -> &'static str {
        self.response_protocol.name()
    }
    pub fn max_llm_input_tokens(&self) -> u32 {
        self.max_llm_input_tokens
    }
    pub fn configured_round_budget(&self) -> u32 {
        self.configured_round_budget
    }
    pub fn capability_tool_count(&self) -> usize {
        self.capabilities.tool_count()
    }
    pub fn capability_skill_count(&self) -> usize {
        self.capabilities.skill_count()
    }
    pub fn memory_file(&self) -> PathBuf {
        self.memory.file.clone()
    }
    pub fn scratch_file(&self) -> PathBuf {
        self.scratch.file.clone()
    }
    pub fn current_stats(&self) -> &UsageStats {
        &self.current_stats
    }
    pub fn last_repair_issue(&self) -> Option<&str> {
        self.last_repair_issue.as_deref()
    }
    pub fn last_topic_events(&self, session_id: &str) -> Vec<CoreTopicEvent> {
        host::notification_topic_events(session_id, &self.last_notifications)
    }
    pub fn notify_last_topic_events(&self, session_id: &str, sink: &mut dyn CoreTopicEventSink) {
        if !self.last_notifications.is_empty() {
            let events = self.last_topic_events(session_id);
            sink.on_core_topic_events(&events);
        }
    }
    pub fn init_lifecycle_topic_event(&self, session_id: &str) -> CoreTopicEvent {
        core_initialized_topic_event(
            session_id,
            &self.profile,
            self.response_protocol.name(),
            self.max_llm_input_tokens,
            self.configured_round_budget,
            self.capabilities.tool_count(),
            self.capabilities.skill_count(),
        )
    }
    pub fn dynamic_context_summary(&self) -> CoreDynamicContextSummary {
        let mut delta_ids = BTreeSet::new();
        let mut visible_slice_count = 0usize;
        let mut estimated_tokens = 0_u32;
        for delta in &self.deltas {
            let hidden = delta.hidden_slice_ids.iter().collect::<HashSet<_>>();
            let mut delta_visible = false;
            for slice in &delta.slices {
                if hidden.contains(&slice.slice_id) {
                    continue;
                }
                delta_visible = true;
                visible_slice_count += 1;
                estimated_tokens =
                    estimated_tokens.saturating_add(estimate_prompt_tokens(&slice.text));
            }
            if delta_visible {
                delta_ids.insert(delta.delta_id.clone());
            }
        }
        CoreDynamicContextSummary {
            visible_delta_count: delta_ids.len(),
            visible_slice_count,
            estimated_tokens,
        }
    }
    pub fn dynamic_context_estimated_tokens(&self) -> u32 {
        self.dynamic_context_summary().estimated_tokens
    }
    pub fn clear_dynamic_context(&mut self) {
        self.deltas.clear();
        self.last_observed_prompt_tokens = 0;
        self.current_round = 0;
        self.current_stats = UsageStats::zero();
        self.repair_attempted = false;
        self.last_repair_issue = None;
        self.pending_approval = None;
        self.current_action_turn_id = None;
        self.current_session_id = None;
        self.current_action_user_question.clear();
        self.last_notifications.clear();
        self.loaded_work_instruction_fingerprints.clear();
    }
    pub fn resolve_stale_context_with_audit(
        &mut self,
        request: StaleContextDecisionRequest,
        continue_old_context: bool,
        audit_file: &Path,
        session: &str,
    ) -> bool {
        let _ = append_audit_event(
            audit_file,
            &stale_context_choice_audit_event(
                session,
                request.idle,
                request.dynamic_context_tokens,
                continue_old_context,
            ),
        );
        if !continue_old_context {
            self.clear_dynamic_context();
        }
        continue_old_context
    }
    pub fn memory_git_commit_count(&self) -> usize {
        self.memory.git_commit_count()
    }

    fn filter_repeated_work_instructions(&mut self, supporting_context: &str) -> String {
        let Some((start, end, block)) = work_instruction_context_block(supporting_context) else {
            return supporting_context.trim().to_string();
        };
        let fingerprint = stable_text_fingerprint(block);
        if self
            .loaded_work_instruction_fingerprints
            .insert(fingerprint)
        {
            return supporting_context.trim().to_string();
        }

        let mut filtered = String::new();
        filtered.push_str(supporting_context[..start].trim_end());
        let tail = supporting_context[end..].trim_start();
        if !filtered.trim().is_empty() && !tail.is_empty() {
            filtered.push_str("\n\n");
        }
        filtered.push_str(tail);
        filtered.trim().to_string()
    }

    pub fn begin_turn(&mut self, user_input: &str, supporting_context: Option<&str>) -> CoreStep {
        self.current_round = 0;
        self.round_budget = self.configured_round_budget;
        self.current_stats = UsageStats::zero();
        self.repair_attempted = false;
        self.last_repair_issue = None;
        self.pending_approval = None;
        self.last_notifications.clear();
        let action_turn_id = unique_id("action_turn");
        self.current_action_turn_id = Some(action_turn_id.clone());
        self.current_action_user_question = user_input.trim().to_string();
        self.action_audit.begin_turn(
            &action_turn_id,
            now_ms(),
            &self.current_action_user_question,
        );
        let mut slices = std::mem::take(&mut self.pending_next_turn_slices);
        let should_memory_precheck = supporting_context
            .map(should_run_memory_precheck)
            .unwrap_or(false);
        let text = user_input.trim().to_string();
        let filtered_supporting_context = supporting_context
            .map(|ctx| self.filter_repeated_work_instructions(ctx))
            .filter(|ctx| !ctx.trim().is_empty());
        let mut system_texts = Vec::new();
        if let Some(ctx) = filtered_supporting_context.as_deref() {
            system_texts.push(ctx.trim().to_string());
        }
        let mut token_estimate_text = text.clone();
        for system_text in &system_texts {
            token_estimate_text.push('\n');
            token_estimate_text.push_str(system_text);
        }
        let incoming_prompt_tokens = estimate_prompt_tokens(&token_estimate_text);
        let pending_dynamic_tokens = estimate_prompt_tokens(&token_estimate_text)
            + slices
                .iter()
                .map(|(_, text)| estimate_prompt_tokens(text))
                .sum::<u32>();
        if let Some(shrink_review) =
            self.consume_shrink_review_if_needed(incoming_prompt_tokens, pending_dynamic_tokens)
        {
            system_texts.push(format!("Long-context maintenance:\n{shrink_review}"));
        }
        slices.push(("user_question".to_string(), text));
        for system_text in system_texts {
            slices.push(("runtime_note".to_string(), system_text));
        }
        if should_memory_precheck {
            let result = self.runtime_memory_precheck(user_input, 5);
            slices.push(("result_of_llm_action".to_string(), result));
        }
        self.append_delta(slices);
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.round_budget,
        }
    }
    pub fn append_user_supplement(&mut self, text: &str) -> Option<CoreStep> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        self.append_slice_to_latest_delta("user_supplement".to_string(), text.to_string());
        Some(CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        })
    }

    pub fn append_user_supplements_with_audit(
        &mut self,
        supplements: impl IntoIterator<Item = String>,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
    ) -> Option<CoreStep> {
        let mut step = None;
        for supplement in supplements {
            let supplement = supplement.trim();
            if supplement.is_empty() {
                continue;
            }
            let _ = append_audit_event(
                audit_file,
                &user_supplement_audit_event(session, turn_id, supplement),
            );
            step = self.append_user_supplement(supplement);
        }
        step
    }

    pub fn apply_model_response(&mut self, response: LlmResponse) -> CoreStep {
        self.apply_model_response_with_cancel(response, &mut || false)
    }

    pub fn apply_model_response_with_cancel(
        &mut self,
        response: LlmResponse,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> CoreStep {
        let mut runtime = CancelOnlyActionRuntime::new(should_cancel);
        self.apply_model_response_with_action_runtime(response, &mut runtime)
    }

    pub fn apply_model_response_with_action_runtime(
        &mut self,
        response: LlmResponse,
        runtime: &mut dyn ActionRuntime,
    ) -> CoreStep {
        self.last_notifications.clear();
        self.current_round += 1;
        self.current_stats.add(&response.usage);
        self.last_observed_prompt_tokens = self
            .last_observed_prompt_tokens
            .max(response.usage.prompt_tokens);
        if response.truncated && !self.repair_attempted {
            let protocol_suite = self.response_protocol.suite();
            return self.request_protocol_repair(
                "truncated_model_output",
                protocol_suite.repair_instruction("truncated_model_output"),
                &response.content,
            );
        }
        let protocol_suite = self.response_protocol.suite();
        let parsed = protocol_suite.parse(&response.content, &self.capabilities);
        let mut slices = Vec::new();
        if !parsed.thought.is_empty() {
            slices.push(("llm_free_talk".to_string(), parsed.thought.to_string()));
        }
        if let Some(issue) = parsed.repair_issue.clone() {
            if !self.repair_attempted {
                return self.request_protocol_repair(
                    &issue,
                    protocol_suite.repair_instruction(&issue),
                    &response.content,
                );
            }
            if issue == "invalid_json"
                && protocol_suite.can_show_plain_text_after_repair_failure(&response.content)
            {
                let final_text = response.content.trim().to_string();
                slices.push((
                    "llm_response".to_string(),
                    llm_final_answer_slice_text(&final_text),
                ));
                self.defer_next_turn_slices(slices);
                self.cleanup_background_jobs_for_current_session("final_answer");
                return CoreStep::Final(TurnFinal {
                    final_answer: final_text,
                    stats: self.current_stats.clone(),
                    profile_label: self.profile.label(),
                    repair_issue: Some("invalid_json_plain_text_fallback".to_string()),
                    stop_summary: None,
                });
            }
            let final_text = parsed.final_text();
            let first_issue = self.last_repair_issue.as_deref().unwrap_or(&issue);
            if final_text.is_empty() {
                return CoreStep::Final(TurnFinal {
                    final_answer: String::new(),
                    stats: self.current_stats.clone(),
                    profile_label: self.profile.label(),
                    repair_issue: Some(issue.clone()),
                    stop_summary: Some(TurnStopSummary::protocol_repair_failed(
                        first_issue,
                        &issue,
                        first_issue == "truncated_model_output"
                            || issue == "truncated_model_output",
                        self.current_stats.clone(),
                        Some(response.usage.clone()),
                    )),
                });
            }
            slices.push((
                "llm_response".to_string(),
                llm_final_answer_slice_text(&final_text),
            ));
            self.defer_next_turn_slices(slices);
            self.cleanup_background_jobs_for_current_session("protocol_repair_final_answer");
            return CoreStep::Final(TurnFinal {
                final_answer: final_text,
                stats: self.current_stats.clone(),
                profile_label: self.profile.label(),
                repair_issue: Some(issue),
                stop_summary: None,
            });
        }
        self.last_notifications = notification::notifications_from_envelope(&parsed);
        for compact in &parsed.context_compacts {
            let missing = self.missing_prompt_refs(&compact.delta_ids, &compact.slice_ids);
            if missing.is_empty() {
                let shrink_result = self.apply_prompt_shrink(
                    "Action result: context_compact",
                    &compact.delta_ids,
                    &compact.slice_ids,
                );
                slices.push((
                    "context_compacted".to_string(),
                    format!(
                        "Context compact summary replacing delta_ids=[{}]:\n{}",
                        compact.delta_ids.join(","),
                        compact.summary
                    ),
                ));
                slices.push(("result_of_llm_action".to_string(), shrink_result));
            } else {
                slices.push((
                    "result_of_llm_action".to_string(),
                    format!(
                        "Action result: context_compact\nerror: invalid_prompt_refs\nmissing_ids: {}",
                        missing.join(", ")
                    ),
                ));
            }
        }
        if !parsed.continue_work {
            for candidate in &parsed.memory_candidates {
                if self.memory.write(candidate).is_ok() {
                    self.current_stats.tool_calls += 1;
                    self.current_stats.mem_writes += 1;
                }
            }
            let final_text = parsed.final_text();
            slices.push((
                "llm_response".to_string(),
                llm_final_answer_slice_text(&final_text),
            ));
            self.defer_next_turn_slices(slices);
            self.cleanup_background_jobs_for_current_session("final_answer");
            return CoreStep::Final(TurnFinal {
                final_answer: final_text,
                stats: self.current_stats.clone(),
                profile_label: self.profile.label(),
                repair_issue: if self.repair_attempted
                    && parsed.runtime_note.as_deref() == Some("auto_wrapped_prose_as_final_answer")
                {
                    Some("invalid_json_plain_text_fallback".to_string())
                } else {
                    None
                },
                stop_summary: None,
            });
        }

        // Omitted status is an intentional shorthand for status:working.
        if let Some(note) = parsed.runtime_note.as_deref() {
            slices.push(("runtime_note".to_string(), note.to_string()));
        }

        if !parsed.action_groups.is_empty() {
            let result_lines = match self.execute_action_groups(parsed.action_groups, runtime) {
                Ok(result_lines) => result_lines,
                Err((result_lines, pending)) => {
                    if !result_lines.is_empty() {
                        slices.push((
                            "result_of_llm_action".to_string(),
                            result_lines.join("\n\n"),
                        ));
                    }
                    self.append_delta(slices);
                    let request = pending.request.clone();
                    self.pending_approval = Some(pending);
                    return CoreStep::NeedsUserApproval { request };
                }
            };
            if !result_lines.is_empty() {
                slices.push((
                    "result_of_llm_action".to_string(),
                    result_lines.join("\n\n"),
                ));
            }
            self.append_delta(slices);
            self.append_in_turn_shrink_review_if_needed();
            if self.remaining_rounds() == 0 {
                return CoreStep::RoundLimitReached {
                    max_rounds: self.round_budget,
                };
            }
            return CoreStep::NeedModel {
                prompt: self.render_prompt(),
                rounds_remaining: self.remaining_rounds(),
            };
        }
        if !parsed.context_compacts.is_empty() {
            self.append_delta(slices);
            self.cleanup_background_jobs_for_current_session("context_compact");
            self.append_in_turn_shrink_review_if_needed();
            if self.remaining_rounds() == 0 {
                return CoreStep::RoundLimitReached {
                    max_rounds: self.round_budget,
                };
            }
            return CoreStep::NeedModel {
                prompt: self.render_prompt(),
                rounds_remaining: self.remaining_rounds(),
            };
        }
        for candidate in &parsed.memory_candidates {
            if self.memory.write(candidate).is_ok() {
                self.current_stats.tool_calls += 1;
                self.current_stats.mem_writes += 1;
            }
        }
        let final_text = parsed.final_text();
        let final_text = if final_text.is_empty() {
            response.content
        } else {
            final_text
        };
        slices.push((
            "llm_response".to_string(),
            llm_final_answer_slice_text(&final_text),
        ));
        self.defer_next_turn_slices(slices);
        self.cleanup_background_jobs_for_current_session("final_answer");
        CoreStep::Final(TurnFinal {
            final_answer: final_text,
            stats: self.current_stats.clone(),
            profile_label: self.profile.label(),
            repair_issue: None,
            stop_summary: None,
        })
    }

    pub fn record_discarded_model_response_usage(&mut self, usage: &UsageStats) {
        self.current_round += 1;
        self.current_stats.add(usage);
        self.last_observed_prompt_tokens =
            self.last_observed_prompt_tokens.max(usage.prompt_tokens);
    }

    pub fn apply_model_response_with_repair_audit(
        &mut self,
        response: LlmResponse,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
    ) -> CoreStep {
        self.apply_model_response_with_repair_audit_and_cancel(
            response,
            audit_file,
            session,
            turn_id,
            &mut || false,
        )
    }

    pub fn apply_model_response_with_repair_audit_and_cancel(
        &mut self,
        response: LlmResponse,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> CoreStep {
        let repair_calls_before = self.current_stats().repair_calls;
        let response_model = response.model_name.clone();
        let response_usage = response.usage.clone();
        let response_truncated = response.truncated;
        let mut runtime = CancelOnlyActionRuntime::new(should_cancel);
        let step = self.apply_model_response_with_action_runtime(response, &mut runtime);
        self.record_model_repair_audit_if_needed(
            audit_file,
            session,
            turn_id,
            repair_calls_before,
            &response_model,
            &response_usage,
            response_truncated,
        );
        step
    }

    pub fn apply_model_response_with_repair_audit_and_runtime(
        &mut self,
        response: LlmResponse,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        runtime: &mut dyn ActionRuntime,
    ) -> CoreStep {
        let repair_calls_before = self.current_stats().repair_calls;
        let response_model = response.model_name.clone();
        let response_usage = response.usage.clone();
        let response_truncated = response.truncated;
        let step = self.apply_model_response_with_action_runtime(response, runtime);
        self.record_model_repair_audit_if_needed(
            audit_file,
            session,
            turn_id,
            repair_calls_before,
            &response_model,
            &response_usage,
            response_truncated,
        );
        step
    }

    fn record_model_repair_audit_if_needed(
        &self,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        repair_calls_before: u32,
        response_model: &str,
        response_usage: &UsageStats,
        response_truncated: bool,
    ) {
        let repair_calls_after = self.current_stats().repair_calls;
        if repair_calls_after > repair_calls_before {
            let _ = append_audit_event(
                audit_file,
                &model_repair_request_audit_event(
                    session,
                    turn_id,
                    self.last_repair_issue(),
                    response_model,
                    response_usage,
                    response_truncated,
                    repair_calls_after,
                    repair_calls_after.saturating_sub(repair_calls_before),
                ),
            );
        }
    }

    pub fn record_turn_start_audit(
        &mut self,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        user_input: &str,
    ) {
        self.current_session_id = Some(session.to_string());
        let _ = append_audit_event(
            audit_file,
            &turn_start_audit_event(session, turn_id, user_input),
        );
    }

    pub fn record_turn_error_audit(
        &self,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        error: &str,
    ) {
        let _ = append_audit_event(audit_file, &turn_error_audit_event(session, turn_id, error));
    }

    pub fn record_turn_final_audit(
        &self,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        outcome: &TurnOutcome,
    ) {
        let _ = append_audit_event(
            audit_file,
            &turn_final_audit_event(
                session,
                turn_id,
                &outcome.text,
                &outcome.stats,
                outcome.latest_usage.as_ref(),
                outcome.repair_issue.as_deref(),
                outcome.stop_summary.as_ref(),
                outcome.elapsed,
            ),
        );
    }

    pub fn resolve_user_approval(&mut self, approval_id: &str, approved: bool) -> CoreStep {
        self.resolve_user_approval_with_cancel(approval_id, approved, &mut || false)
    }

    pub fn resolve_user_approval_with_cancel(
        &mut self,
        approval_id: &str,
        approved: bool,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> CoreStep {
        let mut runtime = CancelOnlyActionRuntime::new(should_cancel);
        self.resolve_user_approval_with_runtime(approval_id, approved, &mut runtime)
    }

    pub fn resolve_user_approval_with_runtime(
        &mut self,
        approval_id: &str,
        approved: bool,
        runtime: &mut dyn ActionRuntime,
    ) -> CoreStep {
        let Some(pending) = self.pending_approval.take() else {
            self.append_delta(vec![(
                "result_of_llm_action".to_string(),
                format!(
                    "Action result: user_approval\napproval_id: {}\nerror: no_pending_approval",
                    approval_id
                ),
            )]);
            return CoreStep::NeedModel {
                prompt: self.render_prompt(),
                rounds_remaining: self.remaining_rounds(),
            };
        };
        if pending.request.approval_id != approval_id {
            let request = pending.request.clone();
            self.pending_approval = Some(pending);
            return CoreStep::NeedsUserApproval { request };
        }
        let result = if approved {
            match &pending.approved_action {
                PendingApprovedAction::RunBash {
                    command,
                    background,
                    timeout_ms,
                    interval_ms,
                    once_timeout_ms,
                    session_id,
                    turn_id,
                } => shell_exec::execute_approved_bash(
                    command,
                    *background,
                    *timeout_ms,
                    *interval_ms,
                    *once_timeout_ms,
                    session_id,
                    turn_id,
                    interval_ms.is_none(),
                    &pending.request,
                    &self.shell_jobs,
                    runtime,
                ),
            }
        } else {
            format!(
                "Action result: {}\ncommand: {}\napproval_id: {}\nstatus: denied_by_user\nreason: {}",
                pending.request.action,
                pending.approved_action.command(),
                pending.request.approval_id,
                pending.request.reason
            )
        };
        let result = annotate_action_result_with_intent(result, &pending.intent);
        self.record_pending_approval_audit(&pending, approved, &result);
        self.append_delta(vec![("result_of_llm_action".to_string(), result)]);
        self.append_in_turn_shrink_review_if_needed();
        if self.remaining_rounds() == 0 {
            return CoreStep::RoundLimitReached {
                max_rounds: self.round_budget,
            };
        }
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        }
    }

    pub fn resolve_user_approval_with_audit(
        &mut self,
        approval: &ApprovalRequest,
        approved: bool,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
    ) -> CoreStep {
        self.resolve_user_approval_with_audit_and_cancel(
            approval,
            approved,
            audit_file,
            session,
            turn_id,
            &mut || false,
        )
    }

    pub fn resolve_user_approval_with_audit_and_cancel(
        &mut self,
        approval: &ApprovalRequest,
        approved: bool,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> CoreStep {
        let _ = append_audit_event(
            audit_file,
            &user_approval_audit_event(session, turn_id, approval, approved),
        );
        self.resolve_user_approval_with_cancel(&approval.approval_id, approved, should_cancel)
    }

    pub fn resolve_user_approval_with_audit_and_runtime(
        &mut self,
        approval: &ApprovalRequest,
        approved: bool,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        runtime: &mut dyn ActionRuntime,
    ) -> CoreStep {
        let _ = append_audit_event(
            audit_file,
            &user_approval_audit_event(session, turn_id, approval, approved),
        );
        self.resolve_user_approval_with_runtime(&approval.approval_id, approved, runtime)
    }

    pub fn continue_after_round_limit(&mut self) -> CoreStep {
        self.current_round = 0;
        self.round_budget = DEFAULT_ROUND_BUDGET;
        self.append_delta(vec![(
            "result_of_llm_action".to_string(),
            "Runtime round budget continued by user.".to_string(),
        )]);
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        }
    }

    pub fn resolve_round_limit_with_audit(
        &mut self,
        request: RoundLimitDecisionRequest,
        should_continue: bool,
        latest_usage: Option<UsageStats>,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
    ) -> RoundLimitResolution {
        let _ = append_audit_event(
            audit_file,
            &round_limit_audit_event(session, turn_id, request.max_rounds, should_continue),
        );
        if should_continue {
            RoundLimitResolution::Continue(self.continue_after_round_limit())
        } else {
            RoundLimitResolution::Stop(TurnStopSummary::round_limit_stopped_by_user(
                request.max_rounds,
                self.current_stats().clone(),
                latest_usage,
            ))
        }
    }

    pub fn resolve_output_expansion_with_audit(
        &self,
        config: &mut ProviderConfig,
        request: OutputExpansionRequest,
        should_expand: bool,
        usage: UsageStats,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
    ) -> OutputExpansionResolution {
        if should_expand {
            config.max_llm_output_tokens = request.expanded_tokens();
            let _ = append_audit_event(
                audit_file,
                &max_llm_output_increased_audit_event(
                    session,
                    turn_id,
                    config.max_llm_output_tokens,
                ),
            );
            OutputExpansionResolution::RetryWithExpandedLimit {
                max_llm_output_tokens: config.max_llm_output_tokens,
            }
        } else {
            OutputExpansionResolution::Stop(TurnStopSummary::output_limit_stopped_by_user(
                config.max_llm_output_tokens,
                usage,
            ))
        }
    }

    pub fn render_prompt(&self) -> String {
        prompt_render::render_prompt_with_rendered_static(
            &self.rendered_static_prompt,
            &self.deltas,
            &self.assistant_speaker_name,
        )
    }
    fn render_prompt_slices(&self) -> Vec<PromptSlice> {
        prompt_render::render_prompt_slices(&self.deltas)
    }
    fn remaining_rounds(&self) -> u32 {
        self.round_budget.saturating_sub(self.current_round)
    }

    fn request_protocol_repair(
        &mut self,
        issue: &str,
        instruction: &str,
        raw_response: &str,
    ) -> CoreStep {
        self.repair_attempted = true;
        self.last_repair_issue = Some(issue.to_string());
        self.current_stats.repair_calls = self.current_stats.repair_calls.saturating_add(1);
        self.append_delta(vec![(
            "response_repair".to_string(),
            format!(
                "Protocol repair request\nshrink_priority: discard_first\nissue: {}\nreason: {}\ninstruction:\n{}\n\nPrevious model response to repair:\n[BEGIN PREVIOUS_LLM_RESPONSE]\n{}\n[END PREVIOUS_LLM_RESPONSE]",
                issue,
                self.response_protocol.suite().repair_reason(issue),
                instruction,
                self.response_protocol
                    .suite()
                    .focused_repair_text(issue, raw_response),
            ),
        )]);
        CoreStep::NeedModel {
            prompt: self.render_prompt(),
            rounds_remaining: self.remaining_rounds(),
        }
    }

    fn append_in_turn_shrink_review_if_needed(&mut self) {
        if let Some(shrink_review) = self.consume_shrink_review_if_needed(0, 0) {
            self.append_delta(vec![(
                "result_of_llm_action".to_string(),
                format!("Long-context maintenance:\n{shrink_review}"),
            )]);
        }
    }

    fn append_delta(&mut self, slice_texts: Vec<(String, String)>) {
        if slice_texts.is_empty() {
            return;
        }
        let timestamp = now_ms();
        let delta_id = format!("pd_{}_{}", timestamp, self.deltas.len() + 1);
        let chunks = slice_texts
            .into_iter()
            .flat_map(|(prompt_type, text)| {
                let slice_time_ms = now_ms();
                split_text_for_prompt_slices(&text, PROMPT_SLICE_TEXT_LIMIT)
                    .into_iter()
                    .map(move |chunk| (prompt_type.clone(), slice_time_ms, chunk))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let slice_count = chunks.len();
        let slices = chunks
            .into_iter()
            .enumerate()
            .map(|(idx, (prompt_type, time_ms, text))| {
                let slice_index = idx + 1;
                PromptSlice {
                    delta_id: delta_id.clone(),
                    slice_id: format!(
                        "ps_{}_s{:03}",
                        delta_id.trim_start_matches("pd_"),
                        slice_index
                    ),
                    prompt_type,
                    time_ms,
                    text,
                    slice_index,
                    slice_count,
                }
            })
            .collect::<Vec<_>>();
        self.deltas.push(PromptDelta {
            delta_id,
            time_ms: timestamp,
            slices,
            hidden_slice_ids: Vec::new(),
        });
    }

    fn defer_next_turn_slices(&mut self, slice_texts: Vec<(String, String)>) {
        self.pending_next_turn_slices.extend(
            slice_texts
                .into_iter()
                .filter(|(_, text)| !text.trim().is_empty()),
        );
    }

    fn append_slice_to_latest_delta(&mut self, prompt_type: String, text: String) {
        if self.deltas.is_empty() {
            self.append_delta(vec![(prompt_type, text)]);
            return;
        }
        let Some(delta) = self.deltas.last_mut() else {
            return;
        };
        let time_ms = now_ms();
        let chunks = split_text_for_prompt_slices(&text, PROMPT_SLICE_TEXT_LIMIT);
        for chunk in chunks {
            let slice_index = delta.slices.len() + 1;
            delta.slices.push(PromptSlice {
                delta_id: delta.delta_id.clone(),
                slice_id: format!(
                    "ps_{}_s{:03}",
                    delta.delta_id.trim_start_matches("pd_"),
                    slice_index
                ),
                prompt_type: prompt_type.clone(),
                time_ms,
                text: chunk,
                slice_index,
                slice_count: 0,
            });
        }
        let slice_count = delta.slices.len();
        for (idx, slice) in delta.slices.iter_mut().enumerate() {
            slice.slice_index = idx + 1;
            slice.slice_count = slice_count;
            slice.slice_id = format!(
                "ps_{}_s{:03}",
                delta.delta_id.trim_start_matches("pd_"),
                idx + 1
            );
        }
    }
    fn consume_shrink_review_if_needed(
        &mut self,
        incoming_prompt_tokens: u32,
        pending_dynamic_tokens: u32,
    ) -> Option<String> {
        let estimated_prompt_tokens = self.estimate_rendered_prompt_tokens(incoming_prompt_tokens);
        let force_threshold = self.max_llm_input_tokens.saturating_mul(90) / 100;
        let slices = self.render_prompt_slices();
        if slices.is_empty() {
            return None;
        }
        let dynamic_tokens = slices
            .iter()
            .map(|slice| estimate_prompt_tokens(&slice.text))
            .sum::<u32>()
            .saturating_add(pending_dynamic_tokens);
        if estimated_prompt_tokens < force_threshold {
            return None;
        }
        let excess_tokens = estimated_prompt_tokens.saturating_sub(force_threshold);
        let practical_shrink_capacity = dynamic_tokens.saturating_mul(8) / 10;
        if practical_shrink_capacity < excess_tokens {
            return None;
        }
        let current_count = self.deltas.len();
        let delta_refs = self
            .deltas
            .iter()
            .filter(|delta| !prompt_render::render_delta_slices(delta).is_empty())
            .rev()
            .take(12)
            .map(|delta| {
                let token_estimate = prompt_render::render_delta_slices(delta)
                    .iter()
                    .map(|slice| estimate_prompt_tokens(&slice.text))
                    .sum::<u32>();
                format!(
                    "- delta_id={} time_ms={} visible_slices={} estimated_tokens={}",
                    delta.delta_id,
                    delta.time_ms,
                    prompt_render::render_delta_slices(delta).len(),
                    token_estimate
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let instruction = "Context is above 90% of the configured input window. You must compact before continuing: summarize all dynamic prompt deltas into about 10%-20% of their current token footprint, discard useless/stale details, and preserve only active work-relevant state. The compact summary should keep: task description, working environment facts, current progress, todo/next steps, and a few high-level work principles when they still guide the task. Use memmgr type=scratch op=write kind=context_offload for important but lengthy existing deltas or kind=notes for compact checkpoints, then use memmgr type=context op=shrink on covered delta_id ranges. Do not target prompt_0.";
        Some(format!(
            "mode=force_shrink_required\nestimated_prompt_tokens={estimated_prompt_tokens}\nmax_llm_input_tokens={}\nforce_shrink_threshold_tokens={force_threshold}\ntarget_dynamic_context_ratio=10%-20%\nprompt_delta_count={current_count}\nrecent_prompt_delta_refs:\n{delta_refs}\n{instruction}",
            self.max_llm_input_tokens
        ))
    }
    fn estimate_rendered_prompt_tokens(&self, incoming_prompt_tokens: u32) -> u32 {
        self.last_observed_prompt_tokens
            .saturating_add(incoming_prompt_tokens)
            .max(estimate_prompt_tokens(&self.render_prompt()))
    }
    fn runtime_memory_precheck(&mut self, query: &str, limit: usize) -> String {
        self.current_stats.tool_calls += 1;
        self.current_stats.mem_reads += 1;
        match self.memory.query(query, limit) {
            Ok(rows) if rows.is_empty() => match self.memory.recent(limit) {
                Ok(recent) if recent.is_empty() => format!(
                    "Action result: runtime_memory_precheck\nquery: {}\nresults: none",
                    query.trim()
                ),
                Ok(recent) => {
                    let lines = recent
                        .into_iter()
                        .map(|r| format!("- {} @ {}", r.content, r.created_at_ms))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Action result: runtime_memory_precheck\nquery: {}\nlexical_results: none\nrecent_memory_evidence:\n{}",
                        query.trim(),
                        lines
                    )
                }
                Err(_) => format!(
                    "Action result: runtime_memory_precheck\nquery: {}\nerror: memory_read_failed",
                    query.trim()
                ),
            },
            Ok(rows) => {
                let lines = rows
                    .into_iter()
                    .map(|r| format!("- {} @ {}", r.content, r.created_at_ms))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "Action result: runtime_memory_precheck\nquery: {}\nresults:\n{}",
                    query.trim(),
                    lines
                )
            }
            Err(_) => format!(
                "Action result: runtime_memory_precheck\nquery: {}\nerror: memory_read_failed",
                query.trim()
            ),
        }
    }
    pub(crate) fn query_prompt_slices(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> Vec<PromptSlice> {
        let terms = search_terms(query);
        let mut rows = self
            .render_prompt_slices()
            .into_iter()
            .filter(|slice| {
                if !time_in_window(slice.time_ms, after_ms, before_ms) {
                    return false;
                }
                if terms.is_empty() {
                    return true;
                }
                let text = slice.text.to_lowercase();
                terms.iter().any(|term| text.contains(term))
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| b.time_ms.cmp(&a.time_ms));
        rows.truncate(limit.clamp(1, 50));
        rows
    }

    fn execute_action_groups(
        &mut self,
        groups: Vec<ParsedActionGroup>,
        runtime: &mut dyn ActionRuntime,
    ) -> Result<Vec<String>, (Vec<String>, PendingApproval)> {
        let mut result_lines = Vec::new();
        for group in groups {
            if group.order == ActionGroupOrder::Parallel
                && group.actions.len() > 1
                && self.can_execute_parallel_bash_group(&group.actions)
            {
                result_lines.extend(self.execute_parallel_bash_group(group.actions));
                continue;
            }
            for action in group.actions {
                match self.execute_action(action, runtime) {
                    ActionExecution::Completed(result) => result_lines.push(result),
                    ActionExecution::NeedsApproval(pending) => {
                        return Err((result_lines, pending));
                    }
                }
            }
        }
        Ok(result_lines)
    }

    fn can_execute_parallel_bash_group(&self, actions: &[ParsedAction]) -> bool {
        self.bash_approval_mode == BashApprovalMode::Approve
            && actions.iter().all(|action| {
                action.action == "run_bash"
                    && !action.background()
                    && action.timeout_ms_i64(5000) >= 0
            })
    }

    fn execute_parallel_bash_group(&mut self, actions: Vec<ParsedAction>) -> Vec<String> {
        let mut handles = Vec::new();
        for action in actions {
            let action_for_audit = action.clone();
            let shell_jobs = self.shell_jobs.clone();
            let session_id = self.current_session_id();
            let turn_id = self.current_action_turn_id();
            self.current_stats.tool_calls += 1;
            handles.push(thread::spawn(move || {
                let loop_command = action.input_str("loop_cmd");
                let is_regular_command = loop_command.is_empty();
                let cmd_command = action.input_str("cmd");
                let command = if is_regular_command {
                    cmd_command.clone()
                } else {
                    loop_command.clone()
                };
                let result = if !loop_command.is_empty() && !cmd_command.is_empty() {
                    ActionExecution::Completed(
                        "Action result: run_bash\nThe command was not executed.\nReason: The action provided both cmd and loop_cmd. Use cmd for a normal/background command, or loop_cmd with interval_ms for polling.".to_string(),
                    )
                } else {
                    let mut should_cancel = || false;
                    let mut runtime = CancelOnlyActionRuntime::new(&mut should_cancel);
                    shell_exec::execute_run_bash(
                        &command,
                        action.background(),
                        if is_regular_command {
                            action.timeout_ms_i64(5000)
                        } else {
                            action.input_i64("loop_timeout_ms").unwrap_or(600_000)
                        },
                        action.input_u64("interval_ms"),
                        action.input_u64("once_timeout_ms").unwrap_or(5000),
                        BashApprovalMode::Approve,
                        &action.intent,
                        &shell_jobs,
                        &session_id,
                        &turn_id,
                        is_regular_command,
                        &mut runtime,
                    )
                };
                let result = match result {
                    ActionExecution::Completed(result) => result,
                    ActionExecution::NeedsApproval(_) => format!(
                        "Action result: run_bash\ncommand: {}\nerror: unexpected_parallel_approval_request",
                        &command,
                    ),
                };
                (action_for_audit, result)
            }));
        }
        let mut result_lines = Vec::new();
        for handle in handles {
            match handle.join() {
                Ok((action, result)) => {
                    let result = annotate_action_result_with_intent(result, &action.intent);
                    self.record_action_audit(&action, "completed", Some(&result));
                    result_lines.push(result);
                }
                Err(_) => result_lines
                    .push("Action result: run_bash\nerror: parallel_action_panicked".to_string()),
            }
        }
        result_lines
    }

    fn execute_action(
        &mut self,
        action: ParsedAction,
        runtime: &mut dyn ActionRuntime,
    ) -> ActionExecution {
        let action_for_audit = action.clone();
        let executor_target = match executor::resolve_action(&self.capabilities, &action.action) {
            Ok(target) => target,
            Err(err) => {
                let result = annotate_action_result_with_intent(
                    format!("Action result: {}\nerror: {}", action.action, err),
                    &action.intent,
                );
                self.record_action_audit(&action_for_audit, "completed", Some(&result));
                return ActionExecution::Completed(result);
            }
        };
        if let Err(issue) = self
            .capabilities
            .validate_action_input(&action.action, &action.raw_input)
        {
            let result = annotate_action_result_with_intent(
                format!(
                    "Action result: {}\nerror: invalid_input\nmessage: {}",
                    action.action, issue
                ),
                &action.intent,
            );
            self.record_action_audit(&action_for_audit, "invalid_input", Some(&result));
            return ActionExecution::Completed(result);
        }
        if let executor::ExecutorTarget::Command { path, .. } = &executor_target {
            let result = annotate_action_result_with_intent(
                self.execute_command_capability(&action, path),
                &action.intent,
            );
            self.record_action_audit(&action_for_audit, "completed", Some(&result));
            return ActionExecution::Completed(result);
        }
        let dispatch_name = match &executor_target {
            executor::ExecutorTarget::Builtin { binding_name } => binding_name.as_str(),
            executor::ExecutorTarget::Command { .. } => {
                unreachable!("command target returned early")
            }
        };
        self.current_stats.tool_calls += 1;
        let execution =
            match tool_registry::execute_builtin_tool(self, dispatch_name, &action, runtime) {
                Some(execution) => execution,
                None => ActionExecution::Completed(format!(
                    "Action result: {}\nunsupported native action",
                    dispatch_name
                )),
            };
        match execution {
            ActionExecution::Completed(result) => {
                let result = annotate_action_result_with_intent(result, &action.intent);
                self.record_action_audit(&action_for_audit, "completed", Some(&result));
                ActionExecution::Completed(result)
            }
            ActionExecution::NeedsApproval(pending) => {
                let result = annotate_action_result_with_intent(format!(
                    "Action result: {}\ncommand: {}\napproval_id: {}\nstatus: needs_user_approval\nrisk: {}\nreason: {}",
                    action_for_audit.action,
                    pending.approved_action.command(),
                    pending.request.approval_id,
                    pending.request.risk,
                    pending.request.reason
                ), &action_for_audit.intent);
                self.record_action_audit(&action_for_audit, "needs_user_approval", Some(&result));
                ActionExecution::NeedsApproval(pending)
            }
        }
    }

    fn execute_command_capability(&mut self, action: &ParsedAction, path: &Path) -> String {
        self.current_stats.tool_calls += 1;
        let payload = json!({
            "action": action.action,
            "intent": action.intent,
            "args": action.raw_input,
        });
        if action.background() {
            return self.tool_jobs.spawn(&action.action, path, &payload);
        }
        executor::execute_command_action(&action.action, path, &payload, action.shell_timeout_ms())
    }

    fn record_action_audit(&self, action: &ParsedAction, status: &str, result: Option<&str>) {
        let turn_id = self
            .current_action_turn_id
            .as_deref()
            .unwrap_or("unknown_turn");
        self.action_audit.record_action(
            ActionAuditEntry {
                time_ms: now_ms(),
                round: self.current_round.max(1),
                action: action.action.clone(),
                intent: action.intent.clone(),
                status: status.to_string(),
                input: action.audit_input(),
                result_summary: result.map(|text| compact_text(text, 2_000)),
            },
            turn_id,
            &self.current_action_user_question,
        );
    }

    fn record_pending_approval_audit(
        &self,
        pending: &PendingApproval,
        approved: bool,
        result: &str,
    ) {
        let turn_id = self
            .current_action_turn_id
            .as_deref()
            .unwrap_or("unknown_turn");
        self.action_audit.record_action(
            ActionAuditEntry {
                time_ms: now_ms(),
                round: self.current_round.max(1),
                action: pending.request.action.clone(),
                intent: pending.intent.clone(),
                status: if approved {
                    "approved_completed".to_string()
                } else {
                    "denied_by_user".to_string()
                },
                input: pending.approved_action.audit_input(
                    &pending.request.approval_id,
                    &pending.request.risk,
                    &pending.request.reason,
                ),
                result_summary: Some(compact_text(result, 2_000)),
            },
            turn_id,
            &self.current_action_user_question,
        );
    }

    pub(crate) fn collect_prompt_context_for_scratch(
        &self,
        delta_ids: &[String],
        slice_ids: &[String],
    ) -> Result<ScratchContextOffload, String> {
        let delta_id_set = delta_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let slice_id_set = slice_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        if delta_id_set.is_empty() && slice_id_set.is_empty() {
            return Err("delta_ids_or_slice_ids_required".to_string());
        }
        if delta_id_set.contains("prompt_0") || slice_id_set.contains("prompt_0") {
            return Err("prompt_0_not_allowed".to_string());
        }

        let existing_delta_ids = self
            .deltas
            .iter()
            .map(|delta| delta.delta_id.clone())
            .collect::<HashSet<_>>();
        let mut matched_delta_ids = HashSet::new();
        let mut matched_slice_ids = HashSet::new();
        let mut sections = Vec::new();
        for delta in &self.deltas {
            let rendered = prompt_render::render_delta_slices(delta);
            if delta_id_set.contains(&delta.delta_id) {
                matched_delta_ids.insert(delta.delta_id.clone());
                sections.push(format!(
                    "[BEGIN SCRATCH OFFLOAD DELTA {} time_ms={}]",
                    delta.delta_id, delta.time_ms
                ));
                for slice in rendered {
                    matched_slice_ids.insert(slice.slice_id.clone());
                    sections.push(format_prompt_slice_for_scratch(&slice));
                }
                sections.push(format!("[END SCRATCH OFFLOAD DELTA {}]", delta.delta_id));
                continue;
            }
            for slice in rendered {
                if slice_id_set.contains(&slice.slice_id) {
                    matched_slice_ids.insert(slice.slice_id.clone());
                    sections.push(format_prompt_slice_for_scratch(&slice));
                }
            }
        }

        let mut missing = delta_id_set
            .difference(&existing_delta_ids)
            .cloned()
            .collect::<Vec<_>>();
        for id in slice_id_set {
            if !matched_slice_ids.contains(&id) {
                missing.push(id);
            }
        }
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            return Err(format!(
                "invalid_prompt_refs missing_ids={}",
                missing.join(",")
            ));
        }
        if sections.is_empty() {
            return Err("no_visible_prompt_context_to_offload".to_string());
        }

        let mut matched_delta_ids = matched_delta_ids.into_iter().collect::<Vec<_>>();
        matched_delta_ids.sort();
        let mut matched_slice_ids = matched_slice_ids.into_iter().collect::<Vec<_>>();
        matched_slice_ids.sort();
        Ok(ScratchContextOffload {
            content: sections.join("\n"),
            delta_ids: matched_delta_ids,
            slice_ids: matched_slice_ids,
        })
    }

    pub(crate) fn apply_prompt_shrink(
        &mut self,
        action_result_header: &str,
        delta_ids: &[String],
        slice_ids: &[String],
    ) -> String {
        let delta_id_set = delta_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let slice_id_set = slice_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let existing_delta_ids = self
            .deltas
            .iter()
            .map(|delta| delta.delta_id.clone())
            .collect::<HashSet<_>>();
        let mut shrunk_tokens_estimate = 0u32;
        for delta in &self.deltas {
            if delta_id_set.contains(&delta.delta_id) {
                for slice in prompt_render::render_delta_slices(delta) {
                    shrunk_tokens_estimate =
                        shrunk_tokens_estimate.saturating_add(estimate_prompt_tokens(&slice.text));
                }
            }
        }
        let before_delta_count = self.deltas.len();
        if !delta_id_set.is_empty() {
            self.deltas
                .retain(|delta| !delta_id_set.contains(&delta.delta_id));
        }
        let removed_delta_count = before_delta_count.saturating_sub(self.deltas.len());

        let mut hidden_slice_count = 0usize;
        let mut matched_slice_ids = HashSet::new();
        if !slice_id_set.is_empty() {
            for delta in &mut self.deltas {
                let slices = prompt_render::render_delta_slices(delta);
                for slice in slices {
                    if slice_id_set.contains(&slice.slice_id) {
                        matched_slice_ids.insert(slice.slice_id.clone());
                        if !delta.hidden_slice_ids.contains(&slice.slice_id) {
                            shrunk_tokens_estimate = shrunk_tokens_estimate
                                .saturating_add(estimate_prompt_tokens(&slice.text));
                            delta.hidden_slice_ids.push(slice.slice_id);
                            hidden_slice_count += 1;
                        }
                    }
                }
            }
        }
        let mut missing = delta_id_set
            .into_iter()
            .filter(|id| !existing_delta_ids.contains(id))
            .collect::<Vec<_>>();
        for id in slice_id_set {
            if !matched_slice_ids.contains(&id) {
                missing.push(id);
            }
        }
        missing.sort();
        missing.dedup();

        self.current_stats.shrunk_tokens = self
            .current_stats
            .shrunk_tokens
            .saturating_add(shrunk_tokens_estimate);
        if shrunk_tokens_estimate > 0 {
            self.last_observed_prompt_tokens = 0;
        }
        let missing_text = if missing.is_empty() {
            "none".to_string()
        } else {
            missing.join(", ")
        };
        format!(
            "{}\nremoved_delta_count: {}\nhidden_slice_count: {}\nshrunk_tokens_estimate: {}\nmissing_ids: {}",
            action_result_header,
            removed_delta_count,
            hidden_slice_count,
            shrunk_tokens_estimate,
            missing_text
        )
    }

    fn missing_prompt_refs(&self, delta_ids: &[String], slice_ids: &[String]) -> Vec<String> {
        let existing_delta_ids = self
            .deltas
            .iter()
            .map(|delta| delta.delta_id.clone())
            .collect::<HashSet<_>>();
        let existing_slice_ids = self
            .render_prompt_slices()
            .into_iter()
            .map(|slice| slice.slice_id)
            .collect::<HashSet<_>>();
        let mut missing = Vec::new();
        for id in delta_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
        {
            if !existing_delta_ids.contains(id) {
                missing.push(id.to_string());
            }
        }
        for id in slice_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
        {
            if !existing_slice_ids.contains(id) {
                missing.push(id.to_string());
            }
        }
        missing.sort();
        missing.dedup();
        missing
    }
}

#[derive(Debug, Clone)]
struct FileMemoryStore {
    dir: PathBuf,
    file: PathBuf,
    guard: MemGuard,
}
impl FileMemoryStore {
    fn new(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            dir: dir.to_path_buf(),
            file: dir.join("memory.jsonl"),
            guard: MemGuard::for_memory_dir(dir),
        }
    }
    fn write(&self, content: &str) -> std::io::Result<()> {
        let clean = content.trim();
        if clean.is_empty() {
            return Ok(());
        }
        self.guard
            .with_write(|| self.write_clean_unlocked(clean))
            .map_err(std::io::Error::other)?
    }

    fn write_clean_unlocked(&self, clean: &str) -> std::io::Result<()> {
        let time_ms = now_ms();
        let record = MemoryRecord {
            id: unique_id("mem"),
            created_at_ms: time_ms,
            updated_at_ms: time_ms,
            version: 1,
            content: clean.to_string(),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&record).unwrap_or_default()
        )?;
        self.snapshot_with_git("memory write");
        Ok(())
    }

    fn query(&self, query: &str, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        self.guard
            .with_read(|| self.query_unlocked(query, limit))
            .map_err(std::io::Error::other)?
    }

    fn query_unlocked(&self, query: &str, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        let terms = search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<MemoryRecord>(&line) {
                let record = normalize_memory_record(record);
                let normalized = record.content.to_lowercase();
                if terms.iter().any(|term| normalized.contains(term)) {
                    rows.push(record);
                }
            }
        }
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }
    fn recent(&self, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        self.guard
            .with_read(|| {
                let mut rows = self.read_all_unlocked()?;
                rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
                rows.truncate(limit.clamp(1, 50));
                Ok(rows)
            })
            .map_err(std::io::Error::other)?
    }

    fn update(
        &self,
        operation: &str,
        id: &str,
        content: &str,
        expected_version: Option<u64>,
    ) -> Result<String, String> {
        self.guard
            .with_write(|| self.update_unlocked(operation, id, content, expected_version))
            .map_err(|err| err.to_string())?
    }

    fn update_unlocked(
        &self,
        operation: &str,
        id: &str,
        content: &str,
        expected_version: Option<u64>,
    ) -> Result<String, String> {
        let op = operation.trim().to_lowercase();
        match op.as_str() {
            "insert" | "upsert" if id.trim().is_empty() => {
                let clean = content.trim();
                if clean.is_empty() {
                    return Err("content_required".to_string());
                }
                self.write_clean_unlocked(clean)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memmgr\ntype: durable\nop: insert\nstored: {}",
                    clean
                ))
            }
            "update" | "upsert" => {
                let clean_id = id.trim();
                let clean = content.trim();
                if clean_id.is_empty() {
                    return Err("id_required".to_string());
                }
                if clean.is_empty() {
                    return Err("content_required".to_string());
                }
                let mut rows = self
                    .read_all_unlocked()
                    .map_err(|_| "memory_read_failed".to_string())?;
                let mut found = false;
                for row in &mut rows {
                    if row.id == clean_id {
                        if let Some(expected) = expected_version {
                            if row.version != expected {
                                return Err(memory_conflict_result(
                                    clean_id,
                                    expected,
                                    row.version,
                                    &row.content,
                                ));
                            }
                        } else {
                            return Err(memory_missing_expected_version_result(
                                clean_id,
                                row.version,
                                &row.content,
                            ));
                        }
                        row.content = clean.to_string();
                        row.updated_at_ms = now_ms();
                        row.version = row.version.saturating_add(1).max(1);
                        found = true;
                        break;
                    }
                }
                if !found {
                    if expected_version.is_some() && op == "update" {
                        return Err("id_not_found".to_string());
                    }
                    let time_ms = now_ms();
                    rows.push(MemoryRecord {
                        id: clean_id.to_string(),
                        created_at_ms: time_ms,
                        updated_at_ms: time_ms,
                        version: 1,
                        content: clean.to_string(),
                    });
                }
                self.write_all_unlocked(&rows)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memmgr\ntype: durable\nop: {}\nid: {}\nversion: {}\nstored: {}",
                    if found { "update" } else { "insert" },
                    clean_id,
                    rows.iter()
                        .find(|row| row.id == clean_id)
                        .map(|row| row.version)
                        .unwrap_or(1),
                    clean
                ))
            }
            "delete" => {
                let clean_id = id.trim();
                if clean_id.is_empty() {
                    return Err("id_required".to_string());
                }
                let mut rows = self
                    .read_all_unlocked()
                    .map_err(|_| "memory_read_failed".to_string())?;
                let before = rows.len();
                if let Some(row) = rows.iter().find(|row| row.id == clean_id) {
                    if let Some(expected) = expected_version {
                        if row.version != expected {
                            return Err(memory_conflict_result(
                                clean_id,
                                expected,
                                row.version,
                                &row.content,
                            ));
                        }
                    } else {
                        return Err(memory_missing_expected_version_result(
                            clean_id,
                            row.version,
                            &row.content,
                        ));
                    }
                }
                rows.retain(|row| row.id != clean_id);
                if rows.len() == before {
                    return Err("id_not_found".to_string());
                }
                self.write_all_unlocked(&rows)
                    .map_err(|_| "write_failed".to_string())?;
                Ok(format!(
                    "Action result: memmgr\ntype: durable\nop: delete\nid: {}\ndeleted: true",
                    clean_id
                ))
            }
            _ => Err("operation_must_be_insert_update_upsert_or_delete".to_string()),
        }
    }

    fn write_all_unlocked(&self, rows: &[MemoryRecord]) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.file)?;
        for row in rows {
            writeln!(file, "{}", serde_json::to_string(row).unwrap_or_default())?;
        }
        self.snapshot_with_git("memory update");
        Ok(())
    }

    fn snapshot_with_git(&self, message: &str) {
        if !self.file.exists() {
            return;
        }
        if Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .arg("init")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| !status.success())
            .unwrap_or(true)
        {
            return;
        }
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["config", "user.name", "timem-memory"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["config", "user.email", "timem-memory@example.invalid"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["add", "memory.jsonl"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| !status.success())
            .unwrap_or(true)
        {
            return;
        }
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["commit", "-m", message])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    fn read_all_unlocked(&self) -> std::io::Result<Vec<MemoryRecord>> {
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<MemoryRecord>(&line) {
                rows.push(normalize_memory_record(record));
            }
        }
        Ok(rows)
    }

    fn git_commit_count(&self) -> usize {
        Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["rev-list", "--count", "HEAD"])
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).ok()
                } else {
                    None
                }
            })
            .and_then(|text| text.trim().parse::<usize>().ok())
            .unwrap_or_default()
    }

    fn schema_text(&self, chat_history: &FileChatHistoryStore) -> String {
        format!(
            "Action result: memmgr\ntype: durable\nop: schema\ntables:\n- memories(id TEXT, created_at_ms INTEGER, updated_at_ms INTEGER, version INTEGER, content TEXT)\n- chat_messages(id TEXT, session_id TEXT, turn_id TEXT, role TEXT, content TEXT, created_at_ms INTEGER, source TEXT, profile_name TEXT, model_name TEXT, source_message_id TEXT)\n- scratch_notes(id TEXT, created_at_ms INTEGER, scratch_type TEXT, label TEXT, content TEXT, prompt_delta_ids ARRAY, prompt_slice_ids ARRAY)\nsafe_interface: memmgr\nops:\n- durable: query|schema|sql|insert|update|upsert|delete\n- raw_chat: query|sql|delete\n- scratch: query|write|read|delete\n- context: shrink\nrules: memmgr sql ops accept SELECT, WITH ... SELECT, or PRAGMA table_info(memories/chat_messages); SQL writes are forbidden; use memmgr type=durable for durable memory insert/update/delete; use expected_version from query results when updating/deleting an existing durable memory to avoid multi-CLI conflicts; use memmgr type=raw_chat op=delete for explicit chat transcript deletion; scratch write requires kind=notes with content or kind=context_offload with delta_ids plus label; scratch read requires id and returns full scratch content. Empty raw_chat query lists recent chat records. loaded_chat_records={}.",
            chat_history.read_all().map(|rows| rows.len()).unwrap_or_default()
        )
    }

    fn sql_read(
        &self,
        chat_history: &FileChatHistoryStore,
        sql: &str,
        params: &[String],
        limit: usize,
    ) -> Result<Vec<Vec<(String, String)>>, String> {
        self.guard
            .with_read(|| self.sql_read_unlocked(chat_history, sql, params, limit))?
    }

    fn sql_read_unlocked(
        &self,
        chat_history: &FileChatHistoryStore,
        sql: &str,
        params: &[String],
        limit: usize,
    ) -> Result<Vec<Vec<(String, String)>>, String> {
        validate_memory_sql(sql)?;
        let placeholder_count = sql.matches('?').count();
        if params.len() != placeholder_count {
            return Err(format!(
                "SQL placeholder count does not match `params`: expected={placeholder_count} actual={}",
                params.len()
            ));
        }
        let conn = Connection::open_in_memory().map_err(|_| "sqlite_open_failed".to_string())?;
        conn.execute(
            "CREATE TABLE memories(id TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, version INTEGER NOT NULL, content TEXT NOT NULL)",
            [],
        )
        .map_err(|_| "sqlite_schema_failed".to_string())?;
        conn.execute(
            "CREATE TABLE chat_messages(id TEXT NOT NULL, session_id TEXT NOT NULL, turn_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at_ms INTEGER NOT NULL, source TEXT NOT NULL, profile_name TEXT, model_name TEXT, source_message_id TEXT)",
            [],
        )
        .map_err(|_| "sqlite_schema_failed".to_string())?;
        for record in self
            .read_all_unlocked()
            .map_err(|_| "memory_read_failed".to_string())?
        {
            conn.execute(
                "INSERT INTO memories(id, created_at_ms, updated_at_ms, version, content) VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    &record.id,
                    record.created_at_ms,
                    record.updated_at_ms,
                    record.version,
                    &record.content,
                ),
            )
            .map_err(|_| "sqlite_load_failed".to_string())?;
        }
        for record in chat_history
            .read_all_unlocked()
            .map_err(|_| "chat_history_read_failed".to_string())?
        {
            if !record.user_input.trim().is_empty() {
                conn.execute(
                    "INSERT INTO chat_messages(id, session_id, turn_id, role, content, created_at_ms, source, profile_name, model_name, source_message_id) VALUES (?1, ?2, ?3, 'user', ?4, ?5, 'shell_audit', NULL, NULL, NULL)",
                    (
                        format!("{}_user", record.turn_id),
                        &record.session,
                        &record.turn_id,
                        &record.user_input,
                        record.started_at_ms,
                    ),
                )
                .map_err(|_| "sqlite_load_failed".to_string())?;
            }
            if !record.assistant_output.trim().is_empty() {
                conn.execute(
                    "INSERT INTO chat_messages(id, session_id, turn_id, role, content, created_at_ms, source, profile_name, model_name, source_message_id) VALUES (?1, ?2, ?3, 'assistant', ?4, ?5, 'shell_audit', NULL, NULL, ?6)",
                    (
                        format!("{}_assistant", record.turn_id),
                        &record.session,
                        &record.turn_id,
                        &record.assistant_output,
                        record.started_at_ms,
                        format!("{}_user", record.turn_id),
                    ),
                )
                .map_err(|_| "sqlite_load_failed".to_string())?;
            }
        }
        let mut stmt = conn
            .prepare(sql)
            .map_err(|_| "sql_prepare_failed".to_string())?;
        let column_names = stmt
            .column_names()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let column_count = column_names.len();
        let mut rows = stmt
            .query(params_from_iter(params.iter().map(String::as_str)))
            .map_err(|_| "sql_query_failed".to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|_| "sql_row_failed".to_string())? {
            let mut cells = Vec::new();
            #[allow(clippy::needless_range_loop)]
            for idx in 0..column_count {
                let value = match row
                    .get_ref(idx)
                    .map_err(|_| "sql_value_failed".to_string())?
                {
                    ValueRef::Null => "NULL".to_string(),
                    ValueRef::Integer(v) => v.to_string(),
                    ValueRef::Real(v) => v.to_string(),
                    ValueRef::Text(v) => String::from_utf8_lossy(v).to_string(),
                    ValueRef::Blob(_) => "<blob>".to_string(),
                };
                cells.push((column_names[idx].clone(), value));
            }
            out.push(cells);
            if out.len() >= limit.clamp(1, 200) {
                break;
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
struct FileScratchStore {
    file: PathBuf,
    guard: MemGuard,
}

impl FileScratchStore {
    fn new(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            file: dir.join("scratch_notes.jsonl"),
            guard: MemGuard::for_memory_dir(dir),
        }
    }

    fn write_record(
        &self,
        scratch_type: &str,
        label: &str,
        content: &str,
        prompt_delta_ids: &[String],
        prompt_slice_ids: &[String],
    ) -> Result<ScratchNoteRecord, String> {
        let clean_type = memmgr::normalize_scratch_kind(scratch_type);
        let clean_label = label.trim();
        let clean_content = content.trim();
        if !matches!(clean_type.as_str(), "notes" | "context_offload") {
            return Err("type_unsupported".to_string());
        }
        if clean_label.is_empty() {
            return Err("label_required".to_string());
        }
        if clean_content.is_empty() {
            return Err("content_required".to_string());
        }
        self.guard.with_write(|| {
            self.write_clean_unlocked(
                &clean_type,
                clean_label,
                clean_content,
                prompt_delta_ids,
                prompt_slice_ids,
            )
        })?
    }

    fn write_clean_unlocked(
        &self,
        scratch_type: &str,
        label: &str,
        clean: &str,
        prompt_delta_ids: &[String],
        prompt_slice_ids: &[String],
    ) -> Result<ScratchNoteRecord, String> {
        let created_at_ms = now_ms();
        let mut clean_delta_ids = prompt_delta_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        clean_delta_ids.sort();
        clean_delta_ids.dedup();
        let mut clean_slice_ids = prompt_slice_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        clean_slice_ids.sort();
        clean_slice_ids.dedup();
        let record = ScratchNoteRecord {
            id: scratch_hash_id(
                scratch_type,
                label,
                clean,
                &clean_delta_ids,
                &clean_slice_ids,
            ),
            created_at_ms,
            scratch_type: scratch_type.to_string(),
            label: label.to_string(),
            content: clean.to_string(),
            prompt_delta_ids: clean_delta_ids,
            prompt_slice_ids: clean_slice_ids,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)
            .map_err(|_| "scratch_open_failed".to_string())?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&record).unwrap_or_default()
        )
        .map_err(|_| "scratch_write_failed".to_string())?;
        Ok(record)
    }

    fn read(&self, id: &str) -> Result<Option<ScratchNoteRecord>, String> {
        let clean_id = id.trim();
        if clean_id.is_empty() {
            return Err("id_required".to_string());
        }
        self.guard.with_read(|| {
            Ok(self
                .read_all_unlocked()?
                .into_iter()
                .find(|record| record.id == clean_id))
        })?
    }

    fn query(&self, query: &str, limit: usize) -> Result<Vec<ScratchNoteRecord>, String> {
        self.guard.with_read(|| self.query_unlocked(query, limit))?
    }

    fn query_unlocked(&self, query: &str, limit: usize) -> Result<Vec<ScratchNoteRecord>, String> {
        let terms = search_terms(query);
        let mut rows = self.read_all_unlocked()?;
        if !terms.is_empty() {
            rows.retain(|record| {
                let normalized = format!("{} {}", record.label, record.content).to_lowercase();
                terms.iter().any(|term| normalized.contains(term))
            });
        }
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }

    fn delete(&self, id: &str) -> Result<bool, String> {
        let clean_id = id.trim();
        if clean_id.is_empty() {
            return Err("id_required".to_string());
        }
        self.guard.with_write(|| {
            let mut rows = self.read_all_unlocked()?;
            let before = rows.len();
            rows.retain(|record| record.id != clean_id);
            if rows.len() == before {
                return Ok(false);
            }
            self.write_all_unlocked(&rows)?;
            Ok(true)
        })?
    }

    fn read_all_unlocked(&self) -> Result<Vec<ScratchNoteRecord>, String> {
        let file = match OpenOptions::new().read(true).open(&self.file) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("scratch_read_failed".to_string()),
        };
        let mut rows = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(record) = serde_json::from_str::<ScratchNoteRecord>(&line) {
                rows.push(record);
            }
        }
        Ok(rows)
    }

    fn write_all_unlocked(&self, rows: &[ScratchNoteRecord]) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.file)
            .map_err(|_| "scratch_open_failed".to_string())?;
        for row in rows {
            writeln!(file, "{}", serde_json::to_string(row).unwrap_or_default())
                .map_err(|_| "scratch_write_failed".to_string())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct FileChatHistoryStore {
    audit_file: PathBuf,
    legacy_audit_file: PathBuf,
    guard: MemGuard,
}
impl FileChatHistoryStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir);
        let audit_file = space_dir.join("audit").join("api_audit.json");
        let legacy_audit_file = space_dir.join("api_audit.jsonl");
        Self {
            audit_file,
            legacy_audit_file,
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    fn audit_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.audit_file.clone()];
        let audit_dir_jsonl = self.audit_file.with_extension("jsonl");
        if audit_dir_jsonl != self.audit_file {
            files.push(audit_dir_jsonl);
        }
        if self.legacy_audit_file != self.audit_file {
            files.push(self.legacy_audit_file.clone());
        }
        files
    }

    fn query(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> std::io::Result<Vec<ChatHistoryRecord>> {
        self.guard
            .with_read(|| self.query_unlocked(query, limit, after_ms, before_ms))
            .map_err(std::io::Error::other)?
    }

    fn query_unlocked(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> std::io::Result<Vec<ChatHistoryRecord>> {
        let terms = search_terms(query);
        let mut rows = self.read_all_unlocked()?;
        rows.retain(|record| time_in_window(record.started_at_ms, after_ms, before_ms));
        if !terms.is_empty() {
            rows.retain(|record| chat_record_matches(record, &terms));
        }
        rows.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }

    fn delete(
        &self,
        id: &str,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> Result<usize, String> {
        let clean_id = id.trim();
        self.guard.with_write(|| {
            let targets = if clean_id.is_empty() {
                self.query_unlocked(query, limit, after_ms, before_ms)
                    .map_err(|_| "chat_history_read_failed".to_string())?
                    .into_iter()
                    .map(|record| record.turn_id)
                    .collect::<HashSet<_>>()
            } else {
                let mut ids = HashSet::new();
                ids.insert(clean_id.to_string());
                ids
            };
            if targets.is_empty() {
                return Ok(0);
            }
            let mut deleted_turn_ids = HashSet::new();
            for audit_file in self.audit_files() {
                if !audit_file.exists() {
                    continue;
                }
                let events = read_audit_events_unlocked(&audit_file)
                    .map_err(|_| "chat_history_read_failed".to_string())?;
                let mut retained = Vec::new();
                for value in events {
                    let turn_id = value
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !turn_id.is_empty() && targets.contains(&turn_id) {
                        deleted_turn_ids.insert(turn_id);
                        continue;
                    }
                    retained.push(value);
                }
                write_audit_events_unlocked(&audit_file, &retained)
                    .map_err(|_| "chat_history_write_failed".to_string())?;
            }
            Ok(deleted_turn_ids.len())
        })?
    }

    fn read_all(&self) -> std::io::Result<Vec<ChatHistoryRecord>> {
        self.guard
            .with_read(|| self.read_all_unlocked())
            .map_err(std::io::Error::other)?
    }

    fn read_all_unlocked(&self) -> std::io::Result<Vec<ChatHistoryRecord>> {
        let mut rows = Vec::<ChatHistoryRecord>::new();
        for audit_file in self.audit_files() {
            for value in read_audit_events_unlocked(&audit_file)? {
                let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
                let turn_id = value
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if turn_id.is_empty() {
                    continue;
                }
                match event_type {
                    "turn_start" => {
                        let user_input = value
                            .get("user_input")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim();
                        if user_input.is_empty() {
                            continue;
                        }
                        rows.push(ChatHistoryRecord {
                            session: value
                                .get("session")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            turn_id: turn_id.to_string(),
                            started_at_ms: turn_id_millis(turn_id)
                                .or_else(|| value.get("created_at").and_then(Value::as_i64))
                                .unwrap_or_default(),
                            user_input: user_input.to_string(),
                            assistant_output: String::new(),
                        });
                    }
                    "turn_final" => {
                        let assistant_output = value
                            .get("assistant_output")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim();
                        if assistant_output.is_empty() {
                            continue;
                        }
                        if let Some(existing) =
                            rows.iter_mut().rev().find(|row| row.turn_id == turn_id)
                        {
                            existing.assistant_output = assistant_output.to_string();
                        } else {
                            rows.push(ChatHistoryRecord {
                                session: value
                                    .get("session")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                turn_id: turn_id.to_string(),
                                started_at_ms: turn_id_millis(turn_id)
                                    .or_else(|| value.get("created_at").and_then(Value::as_i64))
                                    .unwrap_or_default(),
                                user_input: String::new(),
                                assistant_output: assistant_output.to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(rows
            .into_iter()
            .filter(|row| {
                !row.user_input.trim().is_empty() || !row.assistant_output.trim().is_empty()
            })
            .collect())
    }
}

fn read_audit_events_unlocked(path: &Path) -> std::io::Result<Vec<Value>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(events) = value.get("events").and_then(Value::as_array) {
            return Ok(events.clone());
        }
    }
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect())
}

fn write_audit_events_unlocked(path: &Path, events: &[Value]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        for event in events {
            writeln!(file, "{}", serde_json::to_string(event).unwrap_or_default())?;
        }
        return Ok(());
    }
    let doc = json!({"version": 1, "events": events});
    let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
    fs::write(path, format!("{text}\n"))
}

fn validate_memory_sql(sql: &str) -> Result<(), String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("empty_sql".to_string());
    }
    let lowered = trimmed.to_lowercase();
    let lowered = lowered.trim_end_matches(';').trim().to_string();
    let first_keyword = lowered.split_whitespace().next().unwrap_or("");
    if !matches!(first_keyword, "select" | "with" | "pragma") {
        return Err("read_only_sql_required".to_string());
    }
    if lowered.contains(';') {
        return Err("semicolon_not_allowed".to_string());
    }
    if lowered.contains("sqlite_") || lowered.contains("sqlite_master") {
        return Err("only_declared_tables_are_allowed".to_string());
    }
    let tokens = lowered
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let forbidden = [
        "insert", "update", "delete", "alter", "drop", "attach", "detach", "replace", "create",
        "vacuum", "reindex", "analyze", "truncate",
    ];
    if tokens.iter().any(|token| forbidden.contains(token)) {
        return Err("write_or_ddl_not_allowed".to_string());
    }
    if first_keyword == "pragma" {
        let compact = lowered.split_whitespace().collect::<String>();
        if compact == "pragmatable_info(memories)"
            || compact == "pragmatable_info('memories')"
            || compact == "pragmatable_info(\"memories\")"
            || compact == "pragmatable_info(chat_messages)"
            || compact == "pragmatable_info('chat_messages')"
            || compact == "pragmatable_info(\"chat_messages\")"
        {
            return Ok(());
        }
        return Err("only_declared_tables_are_allowed".to_string());
    }
    let allowed_read = lowered.contains(" from memories")
        || lowered.contains(" from chat_messages")
        || lowered.contains(" join memories")
        || lowered.contains(" join chat_messages")
        || lowered.contains(" from (select");
    if !allowed_read {
        return Err("only_declared_tables_are_allowed".to_string());
    }
    Ok(())
}

fn split_text_for_prompt_slices(text: &str, limit: usize) -> Vec<String> {
    let safe_limit = limit.max(1);
    if text.len() <= safe_limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + safe_limit).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| start + idx)
                .unwrap_or(text.len());
        }
        chunks.push(text[start..end].to_string());
        start = end;
    }
    chunks
}

fn work_instruction_context_block(supporting_context: &str) -> Option<(usize, usize, &str)> {
    let start = supporting_context.find("work_directory_instructions:")?;
    let relative_end_marker =
        supporting_context[start..].rfind("[END WORK_DIRECTORY_INSTRUCTION")?;
    let marker_start = start + relative_end_marker;
    let after_marker = supporting_context[marker_start..]
        .find(']')
        .map(|idx| marker_start + idx + 1)?;
    let end = supporting_context[after_marker..]
        .find('\n')
        .map(|idx| after_marker + idx + 1)
        .unwrap_or(supporting_context.len());
    let block = supporting_context[start..end].trim();
    if block.contains("[BEGIN WORK_DIRECTORY_INSTRUCTION") {
        Some((start, end, block))
    } else {
        None
    }
}

fn estimate_prompt_tokens(text: &str) -> u32 {
    text.chars().count().div_ceil(4).min(u32::MAX as usize) as u32
}

fn search_terms(query: &str) -> Vec<String> {
    let lowered = query.to_lowercase();
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for token in lowered.split(|c: char| !c.is_alphanumeric()) {
        push_search_term(token.trim(), &mut seen, &mut terms);
    }
    terms
}

fn push_search_term(token: &str, seen: &mut HashSet<String>, terms: &mut Vec<String>) {
    if token.is_empty() || !seen.insert(token.to_string()) {
        return;
    }
    terms.push(token.to_string());
    if token
        .chars()
        .all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        && token.chars().count() >= 4
    {
        let chars: Vec<char> = token.chars().collect();
        for pair in chars.windows(2) {
            let gram = pair.iter().collect::<String>();
            if seen.insert(gram.clone()) {
                terms.push(gram);
            }
        }
    }
}

fn turn_id_millis(turn_id: &str) -> Option<i64> {
    turn_id
        .strip_prefix("turn_")
        .and_then(|value| value.parse::<i64>().ok())
}

fn chat_record_matches(record: &ChatHistoryRecord, terms: &[String]) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        record.session, record.turn_id, record.user_input, record.assistant_output
    )
    .to_lowercase();
    terms.iter().any(|term| haystack.contains(term))
}

fn time_in_window(time_ms: i64, after_ms: Option<i64>, before_ms: Option<i64>) -> bool {
    after_ms.is_none_or(|after| time_ms >= after) && before_ms.is_none_or(|before| time_ms < before)
}

fn normalize_memory_record(mut record: MemoryRecord) -> MemoryRecord {
    if record.version == 0 {
        record.version = 1;
    }
    if record.updated_at_ms == 0 {
        record.updated_at_ms = record.created_at_ms;
    }
    record
}

fn memory_conflict_result(
    id: &str,
    expected_version: u64,
    current_version: u64,
    current_content: &str,
) -> String {
    format!(
        "memory_conflict id={} expected_version={} current_version={} current_content={}",
        id,
        expected_version,
        current_version,
        compact_text(current_content, 240)
    )
}

fn memory_missing_expected_version_result(
    id: &str,
    current_version: u64,
    current_content: &str,
) -> String {
    format!(
        "missing_expected_version id={} current_version={} current_content={} hint=query with memmgr type=durable op=query or op=sql first, then retry memmgr type=durable op=update with expected_version=current_version",
        id,
        current_version,
        compact_text(current_content, 240)
    )
}

fn should_run_memory_precheck(supporting_context: &str) -> bool {
    supporting_context.contains("memory_lookup_hint:")
}
fn annotate_action_result_with_intent(result: String, intent: &str) -> String {
    let intent = intent.trim();
    if intent.is_empty() || result.lines().any(|line| line.starts_with("intent: ")) {
        return result;
    }
    if let Some((head, tail)) = result.split_once('\n') {
        format!("{head}\nintent: {intent}\n{tail}")
    } else {
        format!("{result}\nintent: {intent}")
    }
}

pub(crate) fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>();
        out.push('…');
    }
    out
}

pub(crate) fn scratch_label_for_display(record: &ScratchNoteRecord) -> String {
    if record.label.trim().is_empty() {
        "(unlabeled)".to_string()
    } else {
        record.label.trim().to_string()
    }
}

pub(crate) fn format_scratch_write_result(record: &ScratchNoteRecord) -> String {
    format!(
        "Action result: memmgr\ntype: scratch\nop: write\nid: {}\nlabel: {}\nscratch_type: {}\nprompt_delta_ids: {}\ncontent_preview: {}",
        record.id,
        scratch_label_for_display(record),
        memmgr::normalize_scratch_kind(&record.scratch_type),
        comma_or_none(&record.prompt_delta_ids),
        compact_text(&record.content, 320)
    )
}

pub(crate) fn format_scratch_read_result(record: &ScratchNoteRecord) -> String {
    format!(
        "Action result: memmgr\ntype: scratch\nop: read\nid: {}\nfound: true\nlabel: {}\nscratch_type: {}\nprompt_delta_ids: {}\ncontent:\n{}",
        record.id,
        scratch_label_for_display(record),
        memmgr::normalize_scratch_kind(&record.scratch_type),
        comma_or_none(&record.prompt_delta_ids),
        record.content
    )
}

fn prompt_type_role_for_scratch(prompt_type: &str) -> &'static str {
    match prompt_type {
        "user_question" | "user_supplement" => "USER",
        "llm_response" | "llm_free_talk" => "ASSISTANT",
        "result_of_llm_action" => "SYSTEM",
        _ => "SYSTEM",
    }
}

fn format_prompt_slice_for_scratch(slice: &PromptSlice) -> String {
    format!(
        "[BEGIN SCRATCH OFFLOAD BLOCK]\ndelta_id: {}\ntime_ms: {}\nrole: {}\n{}\n[END SCRATCH OFFLOAD BLOCK]",
        slice.delta_id,
        slice.time_ms,
        prompt_type_role_for_scratch(&slice.prompt_type),
        slice.text
    )
}

fn comma_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(",")
    }
}

fn scratch_hash_id(
    scratch_type: &str,
    label: &str,
    content: &str,
    delta_ids: &[String],
    slice_ids: &[String],
) -> String {
    let mut hasher = DefaultHasher::new();
    scratch_type.hash(&mut hasher);
    label.hash(&mut hasher);
    content.hash(&mut hasher);
    delta_ids.hash(&mut hasher);
    slice_ids.hash(&mut hasher);
    now_ms().hash(&mut hasher);
    ID_COUNTER.fetch_add(1, Ordering::SeqCst).hash(&mut hasher);
    format!("scratch_{:016x}", hasher.finish())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn unique_id(prefix: &str) -> String {
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, now_ms(), seq)
}

#[derive(Debug, Deserialize)]
struct FfiCoreConfig {
    static_prompt: String,
    memory_dir: String,
    profile: CoreProfile,
}

#[derive(Debug, Deserialize)]
struct FfiLlmResponse {
    content: String,
    model_name: Option<String>,
    usage: Option<UsageStats>,
}

pub struct AgentCoreHandle {
    core: AgentCore,
}

#[no_mangle]
pub extern "C" fn timem_core_new(config_json: *const c_char) -> *mut AgentCoreHandle {
    let Some(config_text) = read_c_string(config_json) else {
        return std::ptr::null_mut();
    };
    let Ok(config) = serde_json::from_str::<FfiCoreConfig>(&config_text) else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(AgentCoreHandle {
        core: AgentCore::new(config.static_prompt, config.profile, config.memory_dir),
    }))
}

#[no_mangle]
pub extern "C" fn timem_core_begin_turn(
    handle: *mut AgentCoreHandle,
    user_input: *const c_char,
    supporting_context: *const c_char,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    let Some(input) = read_c_string(user_input) else {
        return json_string(json_error("null_user_input"));
    };
    let context = read_c_string(supporting_context);
    json_string(step_to_json(
        handle.core.begin_turn(&input, context.as_deref()),
    ))
}

#[no_mangle]
pub extern "C" fn timem_core_apply_model_response(
    handle: *mut AgentCoreHandle,
    response_json: *const c_char,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    let Some(response_text) = read_c_string(response_json) else {
        return json_string(json_error("null_response"));
    };
    let response = match serde_json::from_str::<FfiLlmResponse>(&response_text) {
        Ok(value) => LlmResponse {
            content: value.content,
            model_name: value
                .model_name
                .unwrap_or_else(|| handle.core.profile.model.clone()),
            usage: value.usage.unwrap_or_else(UsageStats::zero),
            truncated: false,
        },
        Err(err) => return json_string(json_error(&format!("invalid_response_json:{err}"))),
    };
    json_string(step_to_json(handle.core.apply_model_response(response)))
}

#[no_mangle]
pub extern "C" fn timem_core_resolve_user_approval(
    handle: *mut AgentCoreHandle,
    approval_id: *const c_char,
    approved: bool,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    let Some(approval_id) = read_c_string(approval_id) else {
        return json_string(json_error("null_approval_id"));
    };
    json_string(step_to_json(
        handle
            .core
            .resolve_user_approval(approval_id.trim(), approved),
    ))
}

#[no_mangle]
pub extern "C" fn timem_core_continue_after_round_limit(
    handle: *mut AgentCoreHandle,
) -> *mut c_char {
    let Some(handle) = handle_mut(handle) else {
        return json_string(json_error("null_handle"));
    };
    json_string(step_to_json(handle.core.continue_after_round_limit()))
}

#[no_mangle]
/// # Safety
/// The caller must ensure that the pointer is valid and was obtained from a corresponding allocation function, or is null.
pub unsafe extern "C" fn timem_core_free(handle: *mut AgentCoreHandle) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

#[no_mangle]
/// # Safety
/// The caller must ensure that `value` is a valid pointer obtained from a previous call to a function that returns a C string, or is null.
pub unsafe extern "C" fn timem_core_free_string(value: *mut c_char) {
    if !value.is_null() {
        unsafe {
            drop(CString::from_raw(value));
        }
    }
}

#[no_mangle]
pub extern "C" fn timem_core_version() -> *mut c_char {
    json_string(serde_json::json!({"agent_core":"rust","version":env!("CARGO_PKG_VERSION")}))
}

fn read_c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(value).to_str().ok().map(ToString::to_string) }
}

fn handle_mut<'a>(handle: *mut AgentCoreHandle) -> Option<&'a mut AgentCoreHandle> {
    if handle.is_null() {
        None
    } else {
        unsafe { handle.as_mut() }
    }
}

fn json_string(value: serde_json::Value) -> *mut c_char {
    let text = serde_json::to_string(&value)
        .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"encode_failed\"}".to_string());
    CString::new(text)
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"nul_byte\"}").unwrap())
        .into_raw()
}

fn json_error(error: &str) -> serde_json::Value {
    serde_json::json!({"ok":false,"error":error})
}

fn step_to_json(step: CoreStep) -> serde_json::Value {
    match step {
        CoreStep::NeedModel {
            prompt,
            rounds_remaining,
        } => serde_json::json!({
            "ok": true,
            "step": "need_model",
            "prompt": prompt,
            "rounds_remaining": rounds_remaining
        }),
        CoreStep::NeedsUserApproval { request } => serde_json::json!({
            "ok": true,
            "step": "needs_user_approval",
            "approval": request
        }),
        CoreStep::RoundLimitReached { max_rounds } => serde_json::json!({
            "ok": true,
            "step": "round_limit_reached",
            "max_rounds": max_rounds
        }),
        CoreStep::Final(turn) => serde_json::json!({
            "ok": true,
            "step": "final",
            "final_answer": turn.final_answer,
            "stats": turn.stats,
            "profile_label": turn.profile_label,
            "repair_issue": turn.repair_issue,
            "stop_summary": turn.stop_summary
        }),
    }
}

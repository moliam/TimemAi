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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub use timem_ui_contract::preferences::{AssistantResponseFormat, InterfacePreferences};

type ParallelActionResult = (usize, ParsedAction, ActionOutcome, Option<Duration>);
type ApprovedParallelBashResult = (
    usize,
    ParsedAction,
    PendingApproval,
    ActionOutcome,
    Option<Duration>,
);
type ParallelActionHandle = thread::JoinHandle<ParallelActionResult>;
type ApprovedParallelBashHandle = thread::JoinHandle<ApprovedParallelBashResult>;

pub mod audit;
pub mod capability;
#[path = "../../../resources/capabilities/tools/capmgr.rs"]
pub mod capmgr;
pub mod chat_library;
mod command_output;
pub mod mcp;
pub use capability::CapabilityHostProfile;
use capability::CapabilityRegistry;
pub mod config_edit;
pub mod config_report;
pub mod context;
#[path = "../../../resources/capabilities/tools/context_compress.rs"]
pub mod context_compress;
pub mod context_policy;
pub mod data_layout;
pub mod executor;
pub mod host;
pub mod interaction;
#[path = "../../../resources/capabilities/tools/memmgr.rs"]
pub mod memmgr;
#[path = "../../../resources/capabilities/tools/memo.rs"]
pub mod memo;
pub mod model_api;
pub mod model_catalog;
mod model_payload;
pub mod model_requirements;
pub mod model_service_config;
pub mod model_stream;
pub mod model_transport;
pub mod negotiation;
mod notification;
pub mod reasoning;
pub use timem_platform as os;
pub mod profiler;
pub mod prompt_cache;
pub mod prompt_components;
pub mod prompt_render;
pub mod prompt_spec;
#[path = "../../../resources/capabilities/tools/readfile.rs"]
pub mod readfile;
pub mod redaction;
pub mod reminder_config;
pub mod response_protocol;
pub mod retry_policy;
pub mod rolling_file_store;
pub mod runtime_context;
mod runtime_info;
mod schema_optimizer;
#[path = "../../../resources/capabilities/tools/self_tool.rs"]
pub mod self_tool;
pub mod session_runtime;
pub mod session_store;
#[path = "../../../resources/capabilities/tools/run_bash.rs"]
pub mod shell_exec;
pub mod status_summary;
pub mod status_view;
#[path = "../../../resources/capabilities/tools/task_finished.rs"]
pub mod task_finished;
pub mod tool_jobs;
#[path = "../../../resources/capabilities/tools/registry.rs"]
pub(crate) mod tool_registry;
pub mod tool_repo;
mod tool_result_gate;
pub use tool_result_gate::{
    validate_model_tool_result_bytes, DEFAULT_MODEL_TOOL_RESULT_BYTES, MAX_MODEL_TOOL_RESULT_BYTES,
};
mod tool_schema_renderer;
#[path = "../../../resources/capabilities/tools/toolgen.rs"]
pub mod toolgen;
pub mod turn_state;
pub mod work_instructions;
pub mod workspace;
pub use audit::{
    api_audit_maintenance_hint_path, api_audit_stream_path, append_audit_event,
    append_repair_output_event, configure_audit_storage, host_start_audit_event,
    max_llm_output_increased_audit_event, model_input_overflow_recovery_audit_event,
    model_repair_output_event, model_repair_request_audit_event, model_retry_audit_event,
    prune_api_audit_before, read_api_audit_doc, round_limit_audit_event,
    stale_context_choice_audit_event, turn_error_audit_event, turn_final_audit_event,
    turn_start_audit_event, user_approval_audit_event, user_supplement_audit_event,
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
    create_memory_dir, default_data_root, default_memory_dir, layout_for_space, resolve_memory_dir,
    web_instance_registry_dir, web_instance_registry_dir_from_default_memory,
    workspace_config_file, RuntimeDataLayout, WebInstanceRegistryRecord,
};
pub use host::{
    capability_negotiation_topic_event, context_compress_requested_topic_event,
    context_compress_topic_event, core_initialized_topic_event,
    core_initialized_topic_event_with_worker, normalize_user_supplements,
    normalize_user_supplements_with_context, resolve_topic_reply,
    running_shell_job_exit_topic_event, runtime_root_repair_help_topic_event,
    session_worker_default_display_name, toolgen_topic_event, topic_event_status_hint,
    work_instruction_load_topic_event, CoreActionTopic, CoreContextCompressTopic,
    CoreDynamicContextSummary, CoreGlobalWorkerStatus, CoreHostDecisionRequestTopic,
    CoreLifecycleEvent, CoreLifecycleTopic, CoreModelRepairTopic, CoreModelResponseTopic,
    CoreSessionState, CoreSessionWorkerIdentity, CoreSessionWorkerWorkspace, CoreTopic,
    CoreTopicEvent, CoreTopicEventSink, CoreTopicStatusHint, CoreWorkInstructionLoadTopic,
    HostDecision, HostDecisionDefault, HostDecisionRequest, LongRunningCommandContinueRequest,
    NoopTurnUi, OutputExpansionRequest, OutputExpansionResolution, RoundLimitDecisionRequest,
    RoundLimitResolution, StoppedTurn, TopicReply, TopicReplyError, TurnInput, TurnOutcome,
    TurnStopDetail, TurnStopReason, TurnStopSummary, TurnUi, UserSupplement, CORE_TOPIC_ACTION,
    CORE_TOPIC_CONTEXT_COMPRESS, CORE_TOPIC_LIFECYCLE, CORE_TOPIC_LONG_RUNNING_COMMAND_REQUEST,
    CORE_TOPIC_MEMO, CORE_TOPIC_MODEL_CAPABILITY_NEGOTIATION, CORE_TOPIC_MODEL_REPAIR,
    CORE_TOPIC_MODEL_RESPONSE, CORE_TOPIC_OUTPUT_EXPAND_REQUEST, CORE_TOPIC_ROUND_LIMIT_REQUEST,
    CORE_TOPIC_RUNTIME_ROOT_REPAIR_HELP, CORE_TOPIC_STALE_CONTEXT_REQUEST, CORE_TOPIC_TOOLGEN,
    CORE_TOPIC_USER_APPROVAL_REQUEST, CORE_TOPIC_WORK_INSTRUCTION_LOAD,
    DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT, USER_SUPPLEMENT_MODEL_DISPATCH_TIMEOUT,
};
pub use interaction::{
    parse_parallel_tool_calls, parse_tool_call_mode, CapabilityProbeIdentity,
    CapabilityProbeSource, InteractionConfig, InteractionProfile, ModelImagePart,
    ModelInteractionRequest, NativeExchange, NativeToolCall, NativeToolChoice, NativeToolResult,
    ParallelToolCalls, PersistedCapabilityProbe, ToolCallMode, ToolDefinition,
    DEFAULT_MAX_TOOL_CALLS_PER_RESPONSE,
};
pub use model_api::{
    build_model_request, build_model_request_with_reasoning, default_api_protocol,
    default_base_url, default_model, interpret_model_http_response, is_default_base_url,
    is_default_model, model_http_error_message, model_prompt_blocks, model_request_audit_event,
    model_response_audit_event, parse_api_protocol, parse_model_response,
    parse_openai_compatible_cache_mode, plan_structured_output, prepare_model_http_request,
    prepare_model_interaction_http_request, prepare_model_request,
    prepare_model_request_with_reasoning, prompt_cache_plan_audit, validate_model_http_headers,
    validate_model_request_fields, without_openai_compatible_cache_control, ApiProtocol,
    ModelCacheControl, ModelHttpResponseInterpretation, ModelHttpTransportOptions,
    ModelPromptBlock, ModelPromptRole, ModelServiceConfig, OpenAiCompatibleCacheMode,
    OpenAiCompatibleOptions, PreparedModelHttpRequest, PreparedModelRequest, StructuredOutputHint,
};
pub use model_service_config::{
    apply_openai_compatible_env_value, model_service_config_from_sources,
    model_service_config_from_sources_allow_missing_api_key, validate_api_key, LocalLLMKeyFile,
    ModelServiceConfigSource,
};
pub use model_transport::{
    call_model, call_model_with_cancel, validate_model_private_ca_pem, HttpModelClient,
};
pub use negotiation::{
    capability_probe_identity, force_reprobe_interaction, negotiate_interaction,
    negotiate_interaction_outcome, NegotiationOutcome,
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
pub use prompt_components::{PromptComponent, PromptComponentRole};
pub use redaction::{redact_value, REDACTED};
pub use reminder_config::{
    default_config_root, default_resources_dir, load_reminder_tips_config,
    reminder_tips_config_path, ReminderScheduleConfig, ReminderTipsConfig, REMINDER_TIPS_FILE_NAME,
    TIMEM_RESOURCES_DIR_ENV,
};
pub use response_protocol::ResponseProtocolKind;
use response_protocol::{
    ActionGroupOrder, ParsedAction, ParsedActionGroup, ParsedContextCompress, ParsedEnvelope,
};
pub use retry_policy::{
    is_explicit_native_tools_unsupported, is_model_input_too_large_error,
    is_retryable_model_system_error, model_retry_decision, ModelCallOutcome, ModelRetryDecision,
    ModelSystemRetryPolicy, DEFAULT_MODEL_SYSTEM_ERROR_RETRIES,
    DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY,
};
pub use runtime_context::{
    local_datetime_label, local_time_label, runtime_time_context, LocalTimeParts,
};
use self_tool::{SelfToolAbout, SelfToolPaths, SelfToolProcess, SelfToolState};
pub use session_runtime::{
    cancelled_turn_result, run_direct_resume_turn, run_direct_resume_turn_with_model_client,
    run_session_turn, run_session_turn_with_model_client, ModelClient,
};
use shell_exec::ShellJobManager;
pub use shell_exec::{RunningShellJob, ShellJobExitUpdate};
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
pub use tool_repo::{
    SessionToolRepo, ToolDetail, ToolFileEntry, ToolManifest, ToolPublishResult, ToolSelfTest,
    ToolSummary,
};
pub use turn_state::{
    ActiveTurnProjection, FinishedTurnProjection, TurnActivity, TurnInputAdmission, TurnProjection,
    TurnProjectionOutcome, TurnToken,
};
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
const ACTION_OUTPUT_CONTEXT_SAFETY_PERCENT: u32 = 95;
const PROMPT_DELTA_RENDER_OVERHEAD_TOKENS: u32 = 64;

fn action_counts_as_tool_call(action: &str) -> bool {
    !matches!(action, "task_finished" | "turn_finished")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreProfile {
    pub model: String,
}
impl CoreProfile {
    pub fn label(&self) -> String {
        self.model.clone()
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
    #[serde(default)]
    pub tool_calls: Vec<NativeToolCall>,
    pub model_name: String,
    pub usage: UsageStats,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AssistantReplayMode {
    RawOutput,
    ExtractedFields,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnFinal {
    pub final_answer: String,
    pub toolgen_retrospect: String,
    pub stats: UsageStats,
    pub profile_label: String,
    pub repair_issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_summary: Option<TurnStopSummary>,
}

fn llm_final_answer_slice_text(final_answer: &str) -> String {
    format!(
        "All previous pending open tasks are completed. Do not repeat this previous answer unless the user asks to quote it. Final Answer:\n{final_answer}"
    )
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

fn role_for_prompt_type(prompt_type: &str, assistant_speaker_name: &str) -> PromptComponentRole {
    match prompt_type {
        "user_question" | "user_supplement" | "user_resume_directly" => PromptComponentRole::user(),
        "llm_response"
        | "llm_response_raw_xml"
        | "llm_free_talk"
        | "context_compression_summary" => {
            PromptComponentRole::assistant(assistant_speaker_name.to_string())
        }
        _ => PromptComponentRole::system(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpServerUpdate {
    Enabled,
    Disabled,
}

fn mcp_server_update_line(server: &mcp::McpServerConfig, update: McpServerUpdate) -> String {
    let label = mcp_server_label(server);
    let state = match update {
        McpServerUpdate::Enabled => "IS ENABLED",
        McpServerUpdate::Disabled => "IS DISABLED",
    };
    format!("MCP update: MCP {label} {state} by user !!!")
}

fn mcp_server_label(server: &mcp::McpServerConfig) -> String {
    if server.name.trim().is_empty() || server.name == server.id {
        server.id.clone()
    } else {
        format!("{} ({})", server.name, server.id)
    }
}

fn bounded_mcp_server_instructions(instructions: &str) -> String {
    let instructions = instructions.trim();
    if instructions.chars().count() <= MAX_MCP_SERVER_INSTRUCTIONS_CHARS {
        return instructions.to_string();
    }
    let retained = instructions
        .chars()
        .take(MAX_MCP_SERVER_INSTRUCTIONS_CHARS)
        .collect::<String>();
    format!("{retained}\n...[MCP server instructions truncated by Timem]")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub action: String,
    pub command: String,
    pub reason: String,
    pub risk: String,
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
pub struct ModelInputOverflowRecovery {
    pub step: CoreStep,
    pub removed_delta_id: String,
    pub removed_action_output_bytes: usize,
}
pub const WORKER_ROLE_CONTEXT_PREFIX: &str = "TIMEM_WORKER_ROLE_CONTEXT: ";

pub const DIRECT_RESUME_USER_INPUT: &str = "user resume directly";

pub fn worker_role_supporting_context(name: &str, description: &str) -> String {
    format!(
        "{WORKER_ROLE_CONTEXT_PREFIX}{}",
        serde_json::json!({
            "name": name,
            "description": description,
        })
    )
}

fn worker_role_display_name(name: &str) -> String {
    name.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn worker_role_full_instruction(
    name: &str,
    description: &str,
    spec: &crate::response_protocol::PromptBoundarySpec,
) -> String {
    format!(
        "{}\nUser involves the worker role ‘{}’ for this round input's related task. When you work for this task, comply this worker's methodology: {}",
        spec.runtime_heading_line(),
        worker_role_display_name(name),
        description
    )
}

fn worker_role_reference_instruction(
    name: &str,
    spec: &crate::response_protocol::PromptBoundarySpec,
) -> String {
    format!(
        "{}\nUser involves the worker role ‘{}’ for this round input's related task (also used in the above). Refer to this role's description above for working methodology.",
        spec.runtime_heading_line(),
        worker_role_display_name(name)
    )
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
    #[serde(default)]
    pub(crate) component_id: String,
    pub(crate) prompt_type: String,
    pub(crate) time_ms: i64,
    pub(crate) text: String,
    pub(crate) slice_index: usize,
    pub(crate) slice_count: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DynamicContextSnapshot {
    pub deltas: Vec<PromptDelta>,
    pub native_exchanges: Vec<NativeExchange>,
    pub last_observed_prompt_tokens: u32,
    /// The runtime-held memo at snapshot time. It is part of the same
    /// consistency unit as the context: restoring one without the other
    /// would desynchronize the long-task reminder from its context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_memo: Option<String>,
    /// Runtime-authority memo closure notices that were still pending at
    /// snapshot time. They describe runtime state the model must hear about
    /// in the next turn, so they travel with the snapshot to survive a
    /// restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_forcible_memo_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_interrupted_memo_note: Option<String>,
}

impl DynamicContextSnapshot {
    /// Whether the snapshot carries no model-visible history or runtime state
    /// that must survive a restart. Token observations are metadata about the
    /// carried context and do not make an otherwise empty snapshot restorable.
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
            && self.native_exchanges.is_empty()
            && self.active_memo.is_none()
            && self.pending_forcible_memo_note.is_none()
            && self.pending_interrupted_memo_note.is_none()
    }
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
pub(crate) struct RawChatHistoryRecord {
    session: String,
    turn_id: String,
    started_at_ms: i64,
    user_input: String,
    assistant_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingApproval {
    request: ApprovalRequest,
    approved_action: PendingApprovedAction,
    action_name: Option<String>,
    action_call_id: String,
    continuation: Option<PendingApprovalContinuation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingApprovalContinuation {
    ParallelGroup {
        actions: Vec<ParsedAction>,
        current_index: usize,
        approved: Vec<(usize, PendingApproval)>,
        denied_results: Vec<(usize, String)>,
        completed_results: Vec<(usize, String)>,
    },
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
        tool_call_id: String,
        cwd: PathBuf,
        tail_out: bool,
        edited_files: Vec<String>,
    },
    ToolgenPublish {
        repo: SessionToolRepo,
        draft_path: PathBuf,
    },
}

impl PendingApprovedAction {
    fn command(&self) -> &str {
        match self {
            PendingApprovedAction::RunBash { command, .. } => command,
            PendingApprovedAction::ToolgenPublish { draft_path, .. } => {
                draft_path.to_str().unwrap_or("<invalid-path>")
            }
        }
    }

    fn tail_out(&self) -> bool {
        matches!(self, PendingApprovedAction::RunBash { tail_out: true, .. })
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
                tool_call_id,
                cwd,
                tail_out,
                edited_files,
            } => json!({
                "command": command,
                "background": background,
                "timeout_ms": timeout_ms,
                "interval_ms": interval_ms,
                "loop_timeout_ms": if interval_ms.is_some() { Some(*timeout_ms) } else { None },
                "once_timeout_ms": if interval_ms.is_some() { Some(*once_timeout_ms) } else { None },
                "session_id": session_id,
                "turn_id": turn_id,
                "tool_call_id": tool_call_id,
                "cwd": cwd,
                "tail_out": tail_out,
                "edit": edited_files,
                "approval_id": approval_id,
                "risk": risk,
                "reason": reason,
            }),
            PendingApprovedAction::ToolgenPublish { draft_path, .. } => json!({
                "draft_path": draft_path,
                "approval_id": approval_id,
                "risk": risk,
                "reason": reason,
            }),
        }
    }
}

const PROMPT_SLICE_TEXT_LIMIT: usize = 12_000;
const MAX_MCP_SERVER_INSTRUCTIONS_CHARS: usize = 32_000;
pub const UNLIMITED_ROUND_BUDGET: u32 = u32::MAX;
pub const CONTEXT_COMPRESS_THRESHOLD_PERCENT_OPTIONS: [u8; 5] = [80, 85, 90, 95, 100];
pub const DEFAULT_CONTEXT_COMPRESS_THRESHOLD_PERCENT: u8 = 90;
const DEFAULT_ROUND_BUDGET: u32 = UNLIMITED_ROUND_BUDGET;
const MAX_CONFIGURED_ROUND_BUDGET: u32 = 10_000;

pub fn validate_context_compress_threshold_percent(percent: u8) -> Result<u8, String> {
    CONTEXT_COMPRESS_THRESHOLD_PERCENT_OPTIONS
        .contains(&percent)
        .then_some(percent)
        .ok_or_else(|| "context_compress_threshold_percent_invalid".to_string())
}
pub const MAX_PROTOCOL_REPAIR_ATTEMPTS: u32 = 20;
const RUNTIME_CONFIG_CHANGED_NOTICE: &str =
    "User changes some runtime config, retrieve again when you need it.";
const MEM_GUARD_WAIT_STEP: Duration = Duration::from_millis(25);
const MEM_GUARD_TIMEOUT: Duration = Duration::from_secs(30);
const MEM_GUARD_STALE_AFTER: Duration = Duration::from_secs(60 * 60 * 6);

fn configured_round_budget(value: Option<&str>) -> u32 {
    let Some(value) = value.map(str::trim) else {
        return DEFAULT_ROUND_BUDGET;
    };
    if value.eq_ignore_ascii_case("unlimited") {
        return UNLIMITED_ROUND_BUDGET;
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|rounds| (1..=MAX_CONFIGURED_ROUND_BUDGET).contains(rounds))
        .unwrap_or(DEFAULT_ROUND_BUDGET)
}

pub(crate) fn runtime_process_owner_id() -> &'static str {
    static OWNER_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OWNER_ID
        .get_or_init(|| {
            let started_at_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{}-{started_at_ns}", std::process::id())
        })
        .as_str()
}

fn configured_round_budget_from_env() -> u32 {
    configured_round_budget(std::env::var("TIMEM_MAX_ROUNDS").ok().as_deref())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkspaceInstanceOwner {
    pub schema_version: u32,
    pub pid: u32,
    #[serde(default)]
    pub process_identity: Option<String>,
    pub host: String,
    pub acquired_at_ms: i64,
}

#[derive(Debug)]
pub struct WorkspaceInstanceLock {
    file: fs::File,
    path: PathBuf,
}

impl WorkspaceInstanceLock {
    pub fn lock_path(memory_dir: impl AsRef<Path>) -> PathBuf {
        let memory_dir = fs::canonicalize(memory_dir.as_ref())
            .unwrap_or_else(|_| memory_dir.as_ref().to_path_buf());
        memory_dir.join(".guard").join("workspace-instance.lock")
    }

    pub fn read_owner(memory_dir: impl AsRef<Path>) -> Option<WorkspaceInstanceOwner> {
        fs::read(Self::lock_path(memory_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    pub fn acquire(memory_dir: impl AsRef<Path>, host: &str) -> Result<Self, String> {
        let memory_dir = fs::canonicalize(memory_dir.as_ref())
            .unwrap_or_else(|_| memory_dir.as_ref().to_path_buf());
        let guard_dir = memory_dir.join(".guard");
        fs::create_dir_all(&guard_dir)
            .map_err(|error| format!("workspace_instance_lock_dir_failed:{error}"))?;
        let path = Self::lock_path(&memory_dir);
        let mut file = os::open_diagnostic_file_lease(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                "workspace_already_in_use".to_string()
            } else {
                format!("workspace_instance_lock_open_failed:{error}")
            }
        })?;
        let owner = WorkspaceInstanceOwner {
            schema_version: 1,
            pid: std::process::id(),
            process_identity: os::process_identity(std::process::id()),
            host: host.to_string(),
            acquired_at_ms: now_ms(),
        };
        let encoded = serde_json::to_vec_pretty(&owner)
            .map_err(|error| format!("workspace_instance_owner_serialize_failed:{error}"))?;
        use std::io::{Seek, SeekFrom};
        file.set_len(0)
            .and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|_| file.write_all(&encoded))
            .and_then(|_| file.sync_data())
            .map_err(|error| format!("workspace_instance_owner_write_failed:{error}"))?;
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkspaceInstanceLock {
    fn drop(&mut self) {
        let _ = self.file.sync_data();
        // Keep the inode: removing a locked file can let another process lock a
        // replacement inode before this handle is dropped.
    }
}

#[derive(Debug, Clone)]
pub struct MemGuard {
    lock_dir: PathBuf,
}

fn sanitize_mem_guard_domain(domain: &str) -> String {
    let mut clean = domain
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while clean.contains("--") {
        clean = clean.replace("--", "-");
    }
    clean = clean.trim_matches(['-', '.']).to_string();
    if clean.is_empty() {
        "default".to_string()
    } else {
        clean
    }
}

impl MemGuard {
    pub fn for_memory_dir(memory_dir: impl AsRef<Path>) -> Self {
        Self::for_memory_domain(memory_dir, "legacy")
    }

    pub fn for_memory_domain(memory_dir: impl AsRef<Path>, domain: impl AsRef<str>) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir.as_ref()).to_path_buf();
        Self::for_space_domain(space_dir, domain)
    }

    pub fn for_space_dir(space_dir: impl AsRef<Path>) -> Self {
        Self::for_space_domain(space_dir, "legacy")
    }

    pub fn for_space_domain(space_dir: impl AsRef<Path>, domain: impl AsRef<str>) -> Self {
        let space_dir = fs::canonicalize(space_dir.as_ref())
            .unwrap_or_else(|_| space_dir.as_ref().to_path_buf());
        let domain = sanitize_mem_guard_domain(domain.as_ref());
        Self {
            lock_dir: space_dir.join(".guard").join(format!("{domain}.lock.d")),
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
        Self::for_space_domain(space_dir, "audit-storage")
    }

    /// Reads do not acquire a cross-process lock. Writers publish complete
    /// snapshots atomically or append complete JSONL records, so a reader may
    /// observe an older consistent state without blocking unrelated work.
    pub fn with_read<T>(&self, f: impl FnOnce() -> T) -> Result<T, String> {
        Ok(f())
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
                // Windows may transiently report PermissionDenied or NotFound while
                // another writer removes and recreates the lock directory. Treat
                // that hand-off window as contention rather than a lock failure.
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                    ) =>
                {
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
        if let Some(owner_alive) = self.lock_owner_alive() {
            return !owner_alive;
        }
        fs::metadata(&self.lock_dir)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|age| age >= MEM_GUARD_STALE_AFTER)
            .unwrap_or(false)
    }

    fn lock_owner_alive(&self) -> Option<bool> {
        let owner = fs::read_to_string(self.lock_dir.join("owner.json")).ok()?;
        let pid = serde_json::from_str::<Value>(&owner)
            .ok()?
            .get("pid")?
            .as_u64()?;
        process_is_alive(pid)
    }
}

fn process_is_alive(pid: u64) -> Option<bool> {
    os::process_is_alive(pid)
}

pub fn atomic_write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot");
    let temporary = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        unique_id("write")
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionStatus {
    Completed,
    Failed,
    Timeout,
    Cancelled,
    BackgroundRunning,
    BackgroundFinished,
}

impl ActionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::BackgroundRunning => "background_running",
            Self::BackgroundFinished => "background_finished",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamCaptureTruncation {
    pub original_bytes: usize,
    pub retained_bytes: usize,
    pub retained: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BashResultEvidence {
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncation: Option<StreamCaptureTruncation>,
    pub stderr_truncation: Option<StreamCaptureTruncation>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub pid: Option<u32>,
    pub timed_out: bool,
    pub pid_kind: Option<String>,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadfileResultEvidence {
    pub path: String,
    pub matcher: Option<String>,
    pub start_line: Option<u64>,
    pub end_line: Option<u64>,
    pub total_lines: Option<u64>,
    pub encoding: Option<String>,
    pub file_bytes: Option<u64>,
    pub content_bytes: Option<usize>,
    pub limited: Option<bool>,
    pub tail_out: Option<bool>,
    pub content: String,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemmgrResultEvidence {
    pub memory_type: String,
    pub op: String,
    pub content: String,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelfToolResultEvidence {
    pub self_type: String,
    pub cwd: Option<String>,
    pub content: String,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionOutcome {
    pub status: ActionStatus,
    pub text: String,
    pub elapsed_ms: Option<u64>,
    pub runtime_metadata: serde_json::Map<String, Value>,
    pub bash_result: Option<BashResultEvidence>,
    pub readfile_result: Option<ReadfileResultEvidence>,
    pub memmgr_result: Option<MemmgrResultEvidence>,
    pub self_tool_result: Option<SelfToolResultEvidence>,
}

impl ActionOutcome {
    pub(crate) fn new(status: ActionStatus, text: impl Into<String>) -> Self {
        Self {
            status,
            text: text.into(),
            elapsed_ms: None,
            runtime_metadata: serde_json::Map::new(),
            bash_result: None,
            readfile_result: None,
            memmgr_result: None,
            self_tool_result: None,
        }
    }

    pub(crate) fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self
    }

    pub(crate) fn with_runtime_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Into<Value>,
    ) -> Self {
        self.runtime_metadata.insert(key.into(), value.into());
        self
    }

    pub(crate) fn with_bash_result(mut self, bash_result: BashResultEvidence) -> Self {
        self.bash_result = Some(bash_result);
        self
    }

    pub(crate) fn with_readfile_result(mut self, readfile_result: ReadfileResultEvidence) -> Self {
        self.readfile_result = Some(readfile_result);
        self
    }

    pub(crate) fn with_memmgr_result(mut self, memmgr_result: MemmgrResultEvidence) -> Self {
        self.memmgr_result = Some(memmgr_result);
        self
    }

    pub(crate) fn with_self_tool_result(
        mut self,
        self_tool_result: SelfToolResultEvidence,
    ) -> Self {
        self.self_tool_result = Some(self_tool_result);
        self
    }

    pub(crate) fn completed(text: impl Into<String>) -> Self {
        Self::new(ActionStatus::Completed, text)
    }

    pub(crate) fn failed(text: impl Into<String>) -> Self {
        Self::new(ActionStatus::Failed, text)
    }

    pub(crate) fn timeout(text: impl Into<String>) -> Self {
        Self::new(ActionStatus::Timeout, text)
    }

    pub(crate) fn cancelled(text: impl Into<String>) -> Self {
        Self::new(ActionStatus::Cancelled, text)
    }

    pub(crate) fn background_running(text: impl Into<String>) -> Self {
        Self::new(ActionStatus::BackgroundRunning, text)
    }

    pub(crate) fn background_finished(text: impl Into<String>) -> Self {
        Self::new(ActionStatus::BackgroundFinished, text)
    }

    /// True when the action has not fully finished yet, so `elapsed_ms` only
    /// reflects time already spent, not the total tool time.
    #[cfg(test)]
    pub(crate) fn still_running(&self) -> bool {
        matches!(
            self.status,
            ActionStatus::Timeout | ActionStatus::BackgroundRunning
        )
    }
}

/// Human readable wall-clock duration: 0.3s, 9.8s, 10s, 2m3s, 1h3m3s.
///
/// Sub-10-second durations keep one decimal place, rounded up, so short tool
/// calls never report a misleading "0s". Longer durations stay integral.
pub(crate) fn format_time_elapsed_hms(ms: u64) -> String {
    const SUB_TEN_SECONDS_CEILING_MS: u64 = 10_000;
    if ms < SUB_TEN_SECONDS_CEILING_MS {
        let tenths = ms.div_ceil(100);
        return format!("{}.{}s", tenths / 10, tenths % 10);
    }
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h{minutes}m{seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ActionExecution {
    Completed(ActionOutcome),
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
    pub pid: u32,
    pub elapsed: Duration,
    pub timeout_ms: Option<i64>,
}

fn thread_cpu_time() -> Option<Duration> {
    #[cfg(unix)]
    {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) };
        if result != 0 || value.tv_sec < 0 || value.tv_nsec < 0 {
            return None;
        }
        Some(Duration::new(value.tv_sec as u64, value.tv_nsec as u32))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn elapsed_thread_cpu(start: Option<Duration>) -> Option<Duration> {
    thread_cpu_time().and_then(|end| end.checked_sub(start?))
}

pub trait ActionRuntime {
    fn should_cancel(&mut self) -> bool;

    /// Returns true when an accepted user supplement has waited past its
    /// dispatch deadline and the next model interaction must be built now.
    /// Long-running executors should hand off to background instead of
    /// finishing; polling executors should stop with interrupted evidence.
    fn should_force_handoff(&mut self) -> bool {
        false
    }

    fn on_core_topic_events(&mut self, _events: &[host::CoreTopicEvent]) {}

    /// Complete protocol validation result, before any response actions execute.
    /// Provisional display is never permission to execute an action.
    fn on_model_response_validated(&mut self, _accepted: bool, _final_response: bool) {}

    fn on_model_response_parsed(
        &mut self,
        _tool_count: usize,
        _has_free_talk: bool,
        _has_tool_call: bool,
    ) {
    }

    fn on_long_running_command(
        &mut self,
        _status: &LongRunningCommandStatus,
    ) -> LongRunningCommandDecision {
        LongRunningCommandDecision::Continue
    }

    /// Returns true if the host signaled "always allow" during the last approval.
    /// The flag is consumed (reset to false) on each call.
    fn take_bash_always_allow(&mut self) -> bool {
        false
    }

    /// Returns the newest pending model-visible tool-result budget, if any.
    /// Called immediately before action-result envelope formatting.
    fn take_model_tool_result_bytes_update(&mut self) -> Option<usize> {
        None
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
    status: String,
    input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_summary: Option<String>,
}

const ACTION_AUDIT_ACTIVE_TURN_MAX_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone)]
struct FileActionAuditStore {
    file: PathBuf,
    active_dir: PathBuf,
    legacy_turns_dir: PathBuf,
    guard: MemGuard,
}

impl FileActionAuditStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir);
        let file = space_dir.join("audit").join("action_audit.json");
        Self {
            active_dir: file.with_file_name("action_audit.active"),
            legacy_turns_dir: file.with_file_name("action_audit.json.turns"),
            guard: MemGuard::for_audit_file(&file),
            file,
        }
    }

    fn begin_turn(&self, turn_id: &str, started_at_ms: i64, user_question: &str) {
        let _ = self.guard.with_write(|| {
            if !self.ensure_archive_unlocked() {
                return false;
            }
            self.recover_stale_active_turns_unlocked();
            let path = self.active_turn_path(turn_id);
            if path.exists() {
                return self
                    .read_turn_unlocked(&path)
                    .is_some_and(|turn| turn.turn_id == turn_id);
            }
            self.write_turn_unlocked(
                &path,
                &ActionAuditTurn {
                    turn_id: turn_id.to_string(),
                    started_at_ms,
                    user_question: user_question.to_string(),
                    interactions: Vec::new(),
                },
            )
        });
    }

    fn record_action(&self, entry: ActionAuditEntry, turn_id: &str, user_question: &str) {
        let _ = self.guard.with_write(|| {
            if !self.ensure_archive_unlocked() {
                return false;
            }
            let path = self.active_turn_path(turn_id);
            let existing = self.read_turn_unlocked(&path);
            if existing
                .as_ref()
                .is_some_and(|turn| turn.turn_id != turn_id)
            {
                return false;
            }
            let mut turn = existing.unwrap_or_else(|| ActionAuditTurn {
                turn_id: turn_id.to_string(),
                started_at_ms: now_ms(),
                user_question: user_question.to_string(),
                interactions: Vec::new(),
            });
            let interaction_index = turn
                .interactions
                .iter()
                .position(|interaction| interaction.round == entry.round)
                .unwrap_or_else(|| {
                    turn.interactions.push(ActionAuditInteraction {
                        round: entry.round,
                        actions: Vec::new(),
                    });
                    turn.interactions.len() - 1
                });
            turn.interactions[interaction_index].actions.push(entry);
            self.write_turn_unlocked(&path, &turn)
        });
    }

    fn finish_turn(&self, turn_id: &str) {
        let _ = self.guard.with_write(|| {
            if !self.ensure_archive_unlocked() {
                return false;
            }
            let path = self.active_turn_path(turn_id);
            let Some(turn) = self.read_turn_unlocked(&path) else {
                return true;
            };
            if turn.turn_id != turn_id {
                return false;
            }
            // The compatibility view is written only after the canonical
            // segmented append. If checkpoint deletion failed after that
            // commit, a same-process retry must remove the checkpoint rather
            // than append the completed Turn a second time.
            let already_committed = self
                .read_doc_unlocked()
                .turns
                .last()
                .is_some_and(|archived| archived.turn_id == turn_id)
                && rolling_file_store::segmented_directory(&self.file).exists();
            if !already_committed && !self.archive_turn_unlocked(&turn) {
                return false;
            }
            fs::remove_file(path).is_ok()
        });
    }

    fn active_turn_path(&self, turn_id: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        turn_id.hash(&mut hasher);
        self.active_dir.join(format!(
            "active-{}-{:016x}.json",
            std::process::id(),
            hasher.finish()
        ))
    }

    fn archive_capacity() -> Option<rolling_file_store::RollingCapacity> {
        rolling_file_store::RollingCapacity::with_slice_bytes(
            audit::ACTION_AUDIT_MAX_BYTES
                .saturating_add(rolling_file_store::AUDIT_ROLLING_SLICE_BYTES),
            rolling_file_store::AUDIT_ROLLING_SLICE_BYTES,
        )
        .ok()
    }

    fn ensure_archive_unlocked(&self) -> bool {
        if fs::create_dir_all(&self.active_dir).is_err() {
            return false;
        }
        let segmented = rolling_file_store::segmented_directory(&self.file);
        if segmented.exists() {
            return self.migrate_legacy_sources_unlocked();
        }
        let mut turns = self.read_doc_unlocked().turns;
        let mut seen = turns
            .iter()
            .map(|turn| turn.turn_id.clone())
            .collect::<BTreeSet<_>>();
        if let Ok(entries) = fs::read_dir(&self.legacy_turns_dir) {
            let mut entries = entries.flatten().collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                if let Some(turn) = self.read_turn_unlocked(&entry.path()) {
                    if seen.insert(turn.turn_id.clone()) {
                        turns.push(turn);
                    }
                }
            }
        }
        let records = turns
            .iter()
            .filter_map(Self::turn_record)
            .collect::<Vec<_>>();
        let Some(capacity) = Self::archive_capacity() else {
            return false;
        };
        if rolling_file_store::rewrite_segmented_records(
            &self.file,
            &records,
            capacity,
            rolling_file_store::AUDIT_ROLLING_SLICE_BYTES,
        )
        .is_err()
        {
            return false;
        }
        // Clean up only legacy files that are valid and now confirmed in the
        // canonical archive. Unreadable files are deliberately retained so an
        // upgrade never destroys the user’s only recoverable copy.
        if !self.migrate_legacy_sources_unlocked() {
            return false;
        }
        turns.last().map_or_else(
            || self.write_empty_view_unlocked(),
            |turn| self.write_latest_view_unlocked(turn),
        )
    }

    fn migrate_legacy_sources_unlocked(&self) -> bool {
        let mut archived = rolling_file_store::read_segmented_records(&self.file)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| serde_json::from_slice::<ActionAuditTurn>(&record).ok())
            .map(|turn| turn.turn_id)
            .collect::<BTreeSet<_>>();

        // A previous new-version run, a downgrade, or an interrupted upgrade
        // can leave the compatibility JSON document beside the segmented
        // archive. Treat every Turn in it as a legacy candidate; turn_id makes
        // this safe and idempotent when the document is merely our latest-Turn
        // compatibility view.
        if let Ok(bytes) = fs::read(&self.file) {
            if let Ok(doc) = serde_json::from_slice::<ActionAuditDocument>(&bytes) {
                for turn in doc.turns {
                    if archived.insert(turn.turn_id.clone()) && !self.archive_turn_unlocked(&turn) {
                        return false;
                    }
                }
            }
        }

        if !self.legacy_turns_dir.exists() {
            return true;
        }
        let Ok(entries) = fs::read_dir(&self.legacy_turns_dir) else {
            return false;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Some(turn) = self.read_turn_unlocked(&path) else {
                // Preserve malformed or unknown legacy files. They may be the
                // user’s only recoverable copy and must not be erased merely
                // because a newer version cannot parse them.
                continue;
            };
            if archived.insert(turn.turn_id.clone()) && !self.archive_turn_unlocked(&turn) {
                return false;
            }
            if fs::remove_file(path).is_err() {
                return false;
            }
        }
        match fs::remove_dir(&self.legacy_turns_dir) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        }
    }

    fn recover_stale_active_turns_unlocked(&self) {
        let Ok(entries) = fs::read_dir(&self.active_dir) else {
            return;
        };
        let mut archived_turn_ids: Option<BTreeSet<String>> = None;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pid) = name
                .strip_prefix("active-")
                .and_then(|rest| rest.split('-').next())
                .and_then(|pid| pid.parse::<u64>().ok())
            else {
                continue;
            };
            if process_is_alive(pid) != Some(false) {
                continue;
            }
            let path = entry.path();
            let Some(turn) = self.read_turn_unlocked(&path) else {
                continue;
            };
            let archived = archived_turn_ids.get_or_insert_with(|| {
                rolling_file_store::read_segmented_records(&self.file)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|record| serde_json::from_slice::<ActionAuditTurn>(&record).ok())
                    .map(|turn| turn.turn_id)
                    .collect()
            });
            if archived.contains(&turn.turn_id) || self.archive_turn_unlocked(&turn) {
                archived.insert(turn.turn_id.clone());
                let _ = fs::remove_file(path);
            }
        }
    }

    fn turn_record(turn: &ActionAuditTurn) -> Option<Vec<u8>> {
        let text = bounded_action_audit_text(
            &ActionAuditDocument {
                version: 1,
                turns: vec![turn.clone()],
            },
            ACTION_AUDIT_ACTIVE_TURN_MAX_BYTES,
        )
        .ok()?;
        let bounded = serde_json::from_str::<ActionAuditDocument>(&text)
            .ok()?
            .turns
            .into_iter()
            .next()?;
        let mut bytes = serde_json::to_vec(&bounded).ok()?;
        bytes.push(b'\n');
        Some(bytes)
    }

    fn archive_turn_unlocked(&self, turn: &ActionAuditTurn) -> bool {
        let (Some(record), Some(capacity)) = (Self::turn_record(turn), Self::archive_capacity())
        else {
            return false;
        };
        if rolling_file_store::append_rolling_record(
            &self.file,
            &record,
            capacity,
            rolling_file_store::AUDIT_ROLLING_SLICE_BYTES,
        )
        .is_err()
        {
            return false;
        }
        // The segmented archive is canonical. A compatibility-view write
        // failure must not retain the active checkpoint and duplicate the Turn
        // on retry; a later successful Turn will refresh the view.
        let _ = self.write_latest_view_unlocked(turn);
        true
    }

    fn read_doc_unlocked(&self) -> ActionAuditDocument {
        let Ok(text) = fs::read_to_string(&self.file) else {
            return Self::empty_doc();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| Self::empty_doc())
    }

    fn read_turn_unlocked(&self, path: &Path) -> Option<ActionAuditTurn> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    fn write_turn_unlocked(&self, path: &Path, turn: &ActionAuditTurn) -> bool {
        if fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("."))).is_err() {
            return false;
        }
        Self::turn_record(turn).is_some_and(|bytes| atomic_write_file(path, &bytes).is_ok())
    }

    fn write_latest_view_unlocked(&self, turn: &ActionAuditTurn) -> bool {
        self.write_doc_unlocked(&ActionAuditDocument {
            version: 1,
            turns: vec![turn.clone()],
        })
    }

    fn write_empty_view_unlocked(&self) -> bool {
        self.write_doc_unlocked(&Self::empty_doc())
    }

    fn write_doc_unlocked(&self, doc: &ActionAuditDocument) -> bool {
        if let Some(parent) = self.file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(text) = bounded_action_audit_text(doc, ACTION_AUDIT_ACTIVE_TURN_MAX_BYTES) else {
            return false;
        };
        atomic_write_file(&self.file, format!("{text}\n").as_bytes()).is_ok()
    }

    fn empty_doc() -> ActionAuditDocument {
        ActionAuditDocument {
            version: 1,
            turns: Vec::new(),
        }
    }
}

fn bounded_action_audit_text(
    doc: &ActionAuditDocument,
    max_bytes: u64,
) -> Result<String, serde_json::Error> {
    let mut retained = doc.clone();
    loop {
        let text = serde_json::to_string_pretty(&retained)?;
        if text.len() as u64 <= max_bytes {
            return Ok(text);
        }
        if retained.turns.len() > 1 {
            retained.turns.remove(0);
            continue;
        }
        let Some(turn) = retained.turns.first_mut() else {
            return Ok(text);
        };
        if remove_oldest_action_from_turn(turn) {
            continue;
        }
        summarize_oversized_action_turn(turn);
        return serde_json::to_string_pretty(&retained);
    }
}

fn remove_oldest_action_from_turn(turn: &mut ActionAuditTurn) -> bool {
    let action_count = turn
        .interactions
        .iter()
        .map(|interaction| interaction.actions.len())
        .sum::<usize>();
    if action_count <= 1 {
        return false;
    }
    if let Some(interaction) = turn
        .interactions
        .iter_mut()
        .find(|interaction| !interaction.actions.is_empty())
    {
        interaction.actions.remove(0);
    }
    turn.interactions
        .retain(|interaction| !interaction.actions.is_empty());
    true
}

fn summarize_oversized_action_turn(turn: &mut ActionAuditTurn) {
    truncate_string_with_marker(&mut turn.turn_id, 512);
    truncate_string_with_marker(&mut turn.user_question, 4_096);
    for interaction in &mut turn.interactions {
        for action in &mut interaction.actions {
            truncate_string_with_marker(&mut action.action, 512);
            truncate_string_with_marker(&mut action.status, 512);
            if let Some(summary) = &mut action.result_summary {
                truncate_string_with_marker(summary, 4_096);
            }
            let input_bytes = serde_json::to_vec(&action.input)
                .map(|encoded| encoded.len())
                .unwrap_or_default();
            if input_bytes > 1024 * 1024 {
                action.input = json!({
                    "payload_omitted": true,
                    "payload_bytes": input_bytes,
                    "payload_limit_bytes": 1024 * 1024,
                });
            }
        }
    }
}

fn truncate_string_with_marker(value: &mut String, max_chars: usize) {
    if value.chars().count() <= max_chars {
        return;
    }
    let original_chars = value.chars().count();
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(&format!("…[truncated; original_chars={original_chars}]"));
    *value = truncated;
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
        config_paths: Vec::new(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThresholdCompactionFollowupState {
    Available,
    FollowupPending,
    Exhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostCompactionVerification {
    Initial,
    Followup,
}

#[derive(Debug)]
pub struct AgentCore {
    memory_dir: PathBuf,
    static_prompt: String,
    runtime_system_context: String,
    rendered_static_prompt: String,
    interface_preferences: InterfacePreferences,
    profile: CoreProfile,
    pub(crate) capabilities: CapabilityRegistry,
    mcp_runtime: mcp::McpRuntime,
    mcp_servers: BTreeMap<String, mcp::McpServerConfig>,
    mcp_tools: BTreeMap<String, mcp::McpTool>,
    mcp_instructions: BTreeMap<String, String>,
    configured_inline_response_protocol: ResponseProtocolKind,
    response_protocol: ResponseProtocolKind,
    pub(crate) memory: FileMemoryStore,
    pub(crate) scratch: FileScratchStore,
    pub(crate) chat_history: FileChatHistoryStore,
    pub(crate) shell_jobs: ShellJobManager,
    pub(crate) disk_pressure: runtime_info::DiskPressureTracker,
    /// Test-only override for the disk sample (total free, total capacity)
    /// so disk pressure windows can be simulated without mutating a real
    /// filesystem. None in production, where the real sample is taken.
    #[cfg(test)]
    pub(crate) disk_free_override: Option<(u64, u64)>,
    #[cfg(test)]
    pub(crate) disk_sample_count: usize,
    pub(crate) tool_jobs: FileToolJobStore,
    action_audit: FileActionAuditStore,
    pub(crate) self_tool: SelfToolState,
    deltas: Vec<PromptDelta>,
    max_llm_input_tokens: u32,
    model_tool_result_bytes: usize,
    context_compress_threshold_percent: u8,
    last_observed_prompt_tokens: u32,
    context_compress_required: bool,
    /// True only when automatic context sizing crossed the forced-compaction
    /// threshold. This is the sole Core scheduling signal for an H1 request;
    /// manual compaction remains at the normal H0 baseline.
    threshold_compaction_reasoning_required: bool,
    /// Set for a user-initiated compaction request: the next request carries
    /// the manual-compaction trailer wording instead of the forced-shrink one.
    manual_compact_trailer_pending: bool,
    /// Set when the runtime first crosses the forced-shrink threshold or a
    /// provider-usage quality check schedules the bounded follow-up. The turn
    /// loop drains it into a `core.context.compress` phase="requested" topic.
    /// Tuple fields are observed prompt tokens, the configured force threshold,
    /// and an optional post-compression quality target.
    pending_compact_request_notice: Option<(u32, u32, Option<u32>)>,
    /// Bounded automatic compression cycle: one initial threshold compression,
    /// at most one forced follow-up, then an exhausted latch until occupancy
    /// drops below the configured trigger threshold.
    threshold_compaction_followup_state: ThresholdCompactionFollowupState,
    /// When the local post-compaction estimate reaches the quality target,
    /// verify it against the provider's prompt-token usage on the next valid
    /// response. This catches estimator undercounts without rejecting or
    /// discarding that response.
    post_compaction_verification: Option<PostCompactionVerification>,
    configured_round_budget: u32,
    round_budget: u32,
    reminder_tips_config: ReminderTipsConfig,
    runtime_config_changed_notice_pending: bool,
    current_round: u32,
    pub(crate) current_stats: UsageStats,
    repair_attempted: bool,
    repair_attempts: u32,
    last_repair_issue: Option<String>,
    pending_approval: Option<PendingApproval>,
    pub(crate) bash_approval_mode: BashApprovalMode,
    current_action_turn_id: Option<String>,
    current_session_id: Option<String>,
    /// Session whose current Runtime/Session aggregate process scopes have
    /// already been persisted into the visible dynamic context.
    process_scope_prompted_session: Option<String>,
    #[cfg(test)]
    process_scope_snapshot_override: Option<os::ProcessAggregateScopeSnapshot>,
    current_action_user_question: String,
    last_notifications: Vec<CoreNotification>,
    loaded_work_instruction_fingerprints: HashSet<String>,
    pending_prompt_components: Vec<PromptComponent>,
    pending_user_interruption_note: bool,
    prompt_component_sequence: u64,
    next_delta_sequence: u64,
    assistant_speaker_name: String,
    assistant_replay_mode: AssistantReplayMode,
    current_prompt_cwd: PathBuf,
    cwd_note_pending: bool,
    touched_paths: HashSet<PathBuf>,
    tool_repo_session_id: String,
    resolved_tool_call_mode: ToolCallMode,
    native_parallel_tool_calls: bool,
    send_native_parallel_tool_control: bool,
    native_exchanges: Vec<NativeExchange>,
    turn_finished_summary: Option<String>,
    active_memo: Option<String>,
    /// Rechargeable budget of memo-finish-guard interceptions for the
    /// current turn. Each model response that does not attempt to finish
    /// recharges it by 1 (capped); each guarded finish attempt consumes 1.
    /// The turn may only finish with an active memo when the budget is 0.
    memo_finish_guard_tokens: u32,
    /// Memo forcibly closed by the runtime when a turn finished while the
    /// memo-finish-guard budget was exhausted. The note is injected once at
    /// the start of the next turn.
    pending_forcible_memo_note: Option<String>,
    /// Memo forcibly closed by the runtime when the user stopped/interrupted
    /// the turn. The note is injected once at the start of the next turn.
    pending_interrupted_memo_note: Option<String>,
    /// Memo text deleted during the current turn. Task finish in the same
    /// turn is challenged once (models may delete and immediately declare
    /// victory without genuinely re-checking the goal).
    memo_deleted_this_turn: Option<String>,
    memo_deleted_trailer_shown: bool,
    pending_native_exchange: Option<(String, String, Vec<NativeToolCall>, Vec<String>)>,
}
impl AgentCore {
    pub fn new(
        static_prompt: impl Into<String>,
        profile: CoreProfile,
        memory_dir: impl AsRef<Path>,
    ) -> Self {
        Self::new_with_interface_preferences(
            static_prompt,
            profile,
            memory_dir,
            InterfacePreferences::default(),
        )
    }

    pub fn new_with_interface_preferences(
        static_prompt: impl Into<String>,
        profile: CoreProfile,
        memory_dir: impl AsRef<Path>,
        interface_preferences: InterfacePreferences,
    ) -> Self {
        let memory_dir = memory_dir.as_ref();
        let self_tool = SelfToolState::new(
            std::env::vars().collect::<BTreeMap<_, _>>(),
            default_self_tool_paths(memory_dir),
            default_self_tool_about(),
            default_self_tool_process(),
        );
        let static_prompt = static_prompt.into();
        let capabilities = CapabilityRegistry::builtin().without_tool("toolgen");
        let response_protocol = ResponseProtocolKind::default();
        let configured_round_budget = configured_round_budget_from_env();
        let assistant_speaker_name = "TIMEM_ASSISTANT".to_string();
        let current_prompt_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let model_tool_result_bytes = tool_result_gate::DEFAULT_MODEL_TOOL_RESULT_BYTES;
        let rendered_static_prompt = prompt_render::render_static_prompt_for_mode_with_preferences(
            &static_prompt,
            &capabilities,
            response_protocol.suite(),
            &assistant_speaker_name,
            ToolCallMode::Inline,
            interface_preferences,
        );
        let mut core = Self {
            memory_dir: memory_dir.to_path_buf(),
            static_prompt,
            runtime_system_context: String::new(),
            rendered_static_prompt,
            interface_preferences,
            profile,
            capabilities,
            mcp_runtime: mcp::McpRuntime::default(),
            mcp_servers: BTreeMap::new(),
            mcp_tools: BTreeMap::new(),
            mcp_instructions: BTreeMap::new(),
            configured_inline_response_protocol: response_protocol,
            response_protocol,
            memory: FileMemoryStore::new(memory_dir),
            scratch: FileScratchStore::new(memory_dir),
            chat_history: FileChatHistoryStore::new(memory_dir),
            shell_jobs: ShellJobManager::new(memory_dir),
            disk_pressure: runtime_info::DiskPressureTracker::new(),
            #[cfg(test)]
            disk_free_override: None,
            #[cfg(test)]
            disk_sample_count: 0,
            tool_jobs: FileToolJobStore::new(memory_dir),
            action_audit: FileActionAuditStore::new(memory_dir),
            self_tool,
            deltas: Vec::new(),
            max_llm_input_tokens: 100_000,
            model_tool_result_bytes,
            context_compress_threshold_percent: DEFAULT_CONTEXT_COMPRESS_THRESHOLD_PERCENT,
            last_observed_prompt_tokens: 0,
            context_compress_required: false,
            threshold_compaction_reasoning_required: false,
            manual_compact_trailer_pending: false,
            pending_compact_request_notice: None,
            threshold_compaction_followup_state: ThresholdCompactionFollowupState::Available,
            post_compaction_verification: None,
            configured_round_budget,
            round_budget: configured_round_budget,
            reminder_tips_config: ReminderTipsConfig::default(),
            runtime_config_changed_notice_pending: false,
            current_round: 0,
            current_stats: UsageStats::zero(),
            repair_attempted: false,
            repair_attempts: 0,
            last_repair_issue: None,
            pending_approval: None,
            bash_approval_mode: BashApprovalMode::Approve,
            current_action_turn_id: None,
            current_session_id: None,
            process_scope_prompted_session: None,
            #[cfg(test)]
            process_scope_snapshot_override: None,
            current_action_user_question: String::new(),
            last_notifications: Vec::new(),
            loaded_work_instruction_fingerprints: HashSet::new(),
            pending_prompt_components: Vec::new(),
            pending_user_interruption_note: false,
            prompt_component_sequence: 0,
            next_delta_sequence: 1,
            assistant_speaker_name,
            assistant_replay_mode: AssistantReplayMode::RawOutput,
            current_prompt_cwd,
            cwd_note_pending: true,
            touched_paths: HashSet::new(),
            tool_repo_session_id: "default".to_string(),
            resolved_tool_call_mode: ToolCallMode::Inline,
            native_parallel_tool_calls: false,
            send_native_parallel_tool_control: false,
            native_exchanges: Vec::new(),
            turn_finished_summary: None,
            active_memo: None,
            memo_finish_guard_tokens: MEMO_FINISH_GUARD_TOKEN_CAP,
            pending_forcible_memo_note: None,
            pending_interrupted_memo_note: None,
            memo_deleted_this_turn: None,
            memo_deleted_trailer_shown: false,
            pending_native_exchange: None,
        };
        // Runtime startup: seed the disk pressure baseline from the real
        // sample immediately, so the first sampling window after a restart
        // compares against startup free space instead of being blind.
        let sample = runtime_info::DiskSample::from_filesystems(&Self::filesystems_for_info(&[]));
        core.disk_pressure.seed_baseline(sample);
        core
    }

    pub fn set_interaction_profile(&mut self, profile: &InteractionProfile) {
        if self.resolved_tool_call_mode == ToolCallMode::Native
            && profile.resolved_mode != ToolCallMode::Native
        {
            self.materialize_native_exchanges();
        }
        self.resolved_tool_call_mode = profile.resolved_mode;
        self.response_protocol = if profile.resolved_mode == ToolCallMode::Native {
            ResponseProtocolKind::Json
        } else {
            self.configured_inline_response_protocol
        };
        self.native_parallel_tool_calls = profile.parallel_enabled;
        self.send_native_parallel_tool_control =
            profile.reason != negotiation::PARALLEL_CONTROL_UNSUPPORTED_REASON;
        self.refresh_rendered_static_prompt();
    }

    pub fn model_interaction_request(
        &self,
        rendered_prompt: impl Into<String>,
    ) -> ModelInteractionRequest {
        if self.resolved_tool_call_mode != ToolCallMode::Native {
            let mut request = ModelInteractionRequest::inline(rendered_prompt);
            request.critical_reasoning = self.reasoning_critical();
            return request;
        }
        let mut tools = self.capabilities.native_builtin_tool_definitions();
        let static_tool_count = tools.len();
        let mut dynamic_tools = self.capabilities.native_dynamic_tool_definitions();
        self.attach_mcp_instructions_to_native_tools(&mut dynamic_tools);
        tools.extend(dynamic_tools);
        ModelInteractionRequest {
            rendered_prompt: rendered_prompt.into(),
            images: Vec::new(),
            static_tool_count,
            tools,
            native_exchanges: self.native_exchanges.clone(),
            resolved_mode: ToolCallMode::Native,
            parallel_tool_calls: self.native_parallel_tool_calls,
            send_parallel_tool_calls: self.send_native_parallel_tool_control,
            tool_choice: if self.context_compress_required {
                NativeToolChoice::Required
            } else {
                NativeToolChoice::Auto
            },
            critical_reasoning: self.reasoning_critical(),
        }
    }

    pub fn reasoning_critical(&self) -> bool {
        self.context_compress_required && self.threshold_compaction_reasoning_required
    }

    fn register_native_exchange(&mut self, exchange: NativeExchange) {
        self.native_exchanges.push(exchange);
    }

    fn attach_mcp_instructions_to_native_tools(&self, tools: &mut [ToolDefinition]) {
        let mut described_servers = HashSet::new();
        for tool in tools {
            let Some(mcp_tool) = self.mcp_tools.get(&tool.name) else {
                continue;
            };
            let server_id = &mcp_tool.server_id;
            if described_servers.contains(server_id) {
                continue;
            }
            let Some(instructions) = self.mcp_instructions.get(server_id) else {
                continue;
            };
            let server_label = self
                .mcp_servers
                .get(server_id)
                .map(mcp_server_label)
                .unwrap_or_else(|| server_id.clone());
            tool.description.push_str(&format!(
                "\n\nMCP server-wide instructions for {server_label}; apply these instructions to every tool from this server:\n{instructions}"
            ));
            described_servers.insert(server_id.clone());
        }
    }

    pub fn set_tool_repo_session_id(&mut self, session_id: impl Into<String>) {
        let session_id = session_id.into();
        if !session_id.trim().is_empty() {
            self.tool_repo_session_id = session_id;
        }
    }

    pub fn tool_repo(&self) -> SessionToolRepo {
        SessionToolRepo::new(&self.memory_dir, &self.tool_repo_session_id)
    }

    pub fn fork_ephemeral_context(&self, cwd: impl AsRef<Path>) -> Self {
        let mut fork = Self::new_with_interface_preferences(
            self.static_prompt.clone(),
            self.profile.clone(),
            &self.memory_dir,
            self.interface_preferences,
        );
        fork.capabilities = self.capabilities.clone();
        fork.configured_inline_response_protocol = self.configured_inline_response_protocol;
        fork.response_protocol = self.response_protocol;
        fork.max_llm_input_tokens = self.max_llm_input_tokens;
        fork.model_tool_result_bytes = self.model_tool_result_bytes;
        fork.context_compress_threshold_percent = self.context_compress_threshold_percent;
        fork.configured_round_budget = self.configured_round_budget;
        fork.round_budget = self.configured_round_budget;
        fork.bash_approval_mode = self.bash_approval_mode;
        fork.assistant_speaker_name = self.assistant_speaker_name.clone();
        fork.runtime_system_context = self.runtime_system_context.clone();
        fork.assistant_replay_mode = self.assistant_replay_mode;
        fork.current_prompt_cwd = cwd.as_ref().to_path_buf();
        fork.tool_repo_session_id = self.tool_repo_session_id.clone();
        fork.refresh_rendered_static_prompt();
        fork
    }

    pub fn set_round_budget(&mut self, rounds: u32) {
        let rounds = rounds.max(1);
        self.configured_round_budget = rounds;
        self.round_budget = rounds;
        self.current_round = 0;
    }

    pub fn set_assistant_speaker_name(&mut self, name: impl AsRef<str>) {
        self.assistant_speaker_name = normalize_assistant_speaker_name(name.as_ref());
        self.refresh_rendered_static_prompt();
    }

    pub fn set_runtime_system_context(&mut self, context: impl AsRef<str>) {
        let context = context.as_ref().trim();
        if self.runtime_system_context == context {
            return;
        }
        self.runtime_system_context = context.to_string();
        self.refresh_rendered_static_prompt();
    }

    pub fn assistant_speaker_name(&self) -> &str {
        &self.assistant_speaker_name
    }

    pub fn set_assistant_replay_mode(&mut self, mode: AssistantReplayMode) {
        self.assistant_replay_mode = mode;
    }

    pub fn set_model_tool_result_bytes(&mut self, max_bytes: usize) -> Result<(), String> {
        tool_result_gate::validate_model_tool_result_bytes(max_bytes)?;
        self.model_tool_result_bytes = max_bytes;
        Ok(())
    }

    pub fn model_tool_result_bytes(&self) -> usize {
        self.model_tool_result_bytes
    }

    pub fn set_claude_codex_tool_discovery(&mut self, enabled: bool) {
        if self.interface_preferences.claude_codex_tool_discovery == enabled {
            return;
        }
        self.interface_preferences.claude_codex_tool_discovery = enabled;
        self.refresh_rendered_static_prompt();
    }

    pub fn assistant_replay_mode(&self) -> AssistantReplayMode {
        self.assistant_replay_mode
    }

    pub fn current_prompt_cwd(&self) -> &Path {
        &self.current_prompt_cwd
    }

    pub fn change_prompt_cwd(&mut self, new_path: impl AsRef<str>) -> Result<PathBuf, String> {
        let new_path = new_path.as_ref().trim();
        if new_path.is_empty() {
            return Err("new_path_required".to_string());
        }
        let candidate = Path::new(new_path);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.current_prompt_cwd.join(candidate)
        };
        let canonical = fs::canonicalize(&candidate).map_err(|_| "path_not_found".to_string())?;
        if !canonical.is_dir() {
            return Err("path_is_not_directory".to_string());
        }
        self.current_prompt_cwd = canonical.clone();
        self.cwd_note_pending = true;
        Ok(canonical)
    }

    fn cwd_prompt_note(&self) -> String {
        format!(
            "[!!!NOTE] cwd now set to: {} , you can save/shorten `cd` command based on this path.",
            self.current_prompt_cwd.display()
        )
    }

    fn submit_cwd_note_if_pending(&mut self) {
        if !self.cwd_note_pending {
            return;
        }
        self.cwd_note_pending = false;
        self.submit_cwd_note_at(now_ms());
    }

    fn submit_cwd_note_at(&mut self, logical_time_ms: i64) {
        self.submit_prompt_component(
            PromptComponentRole::system(),
            "runtime_note",
            self.cwd_prompt_note(),
            "runtime_cwd",
        );
        if let Some(component) = self.pending_prompt_components.last_mut() {
            component.created_at_ms = logical_time_ms;
        }
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

    pub fn finish_action_audit_turn(&mut self) {
        if let Some(turn_id) = self.current_action_turn_id.take() {
            self.action_audit.finish_turn(&turn_id);
        }
    }

    /// Returns a thread-safe cancellation callback for detached resources belonging to one Session.
    /// The callback does not retain the mutable Agent state and can be used by an external Session
    /// coordinator while the Agent is running on its worker thread.
    pub fn background_resource_cancel_callback(
        &self,
        session_id: impl Into<String>,
    ) -> Arc<dyn Fn() + Send + Sync> {
        let shell_jobs = self.shell_jobs.clone();
        let tool_jobs = self.tool_jobs.clone();
        let session_id = session_id.into();
        Arc::new(move || {
            shell_jobs.cancel_unfinished_for_session(&session_id);
            tool_jobs.cancel_unfinished_for_session(&session_id);
        })
    }

    /// Cancels detached background resources belonging to one Session.
    /// Foreground work is interrupted by the worker cancellation token.
    pub fn cancel_background_resources_for_session(&self, session_id: &str) -> usize {
        self.shell_jobs
            .cancel_unfinished_for_session(session_id)
            .len()
            + self
                .tool_jobs
                .cancel_unfinished_for_session(session_id)
                .len()
    }

    pub fn query_running_shell_jobs_for_session(&self, session_id: &str) -> Vec<RunningShellJob> {
        self.shell_jobs.query_running_for_session(session_id)
    }

    /// Registers an event-driven callback fired immediately when a
    /// background shell job's supervisor observes its exit. Used by session
    /// workers to push finish topics to the UI without waiting for harvest.
    pub fn set_shell_job_exit_listener(
        &self,
        listener: impl Fn(&ShellJobExitUpdate) + Send + Sync + 'static,
    ) {
        self.shell_jobs.set_exit_listener(listener);
    }

    pub fn consume_completed_shell_jobs_for_session(
        &mut self,
        session_id: &str,
    ) -> Vec<RunningShellJob> {
        self.consume_completed_shell_jobs_for_session_with_runtime(session_id, None)
    }

    /// Refreshes detached shell jobs and reports exit events through the active Agent runtime.
    pub fn consume_completed_shell_jobs_for_session_with_runtime(
        &mut self,
        session_id: &str,
        runtime: Option<&mut dyn ActionRuntime>,
    ) -> Vec<RunningShellJob> {
        let (running, updates) = self.shell_jobs.consume_completed_for_session(session_id);
        self.submit_running_job_updates_with_runtime(updates, runtime);
        running
    }

    fn submit_running_job_updates_for_session(
        &mut self,
        session_id: &str,
        runtime: &mut dyn ActionRuntime,
    ) {
        let (_, updates) = self.shell_jobs.consume_completed_for_session(session_id);
        self.submit_running_job_updates_with_runtime(updates, Some(runtime));
    }

    fn submit_running_job_updates_with_runtime(
        &mut self,
        updates: Vec<ShellJobExitUpdate>,
        runtime: Option<&mut dyn ActionRuntime>,
    ) {
        if let Some(runtime) = runtime {
            let events = updates
                .iter()
                // Jobs whose finish topic was already published through the
                // manager's exit listener must not emit a duplicate topic;
                // the textual RUNNING_JOB_UPDATE below still goes to the model.
                .filter(|update| !update.topic_published)
                .map(host::running_shell_job_exit_topic_event)
                .collect::<Vec<_>>();
            if !events.is_empty() {
                runtime.on_core_topic_events(&events);
            }
        }
        self.submit_running_job_updates(updates, true);
    }

    fn format_running_job_updates(updates: &[ShellJobExitUpdate]) -> Option<String> {
        (!updates.is_empty()).then(|| {
            updates
                .iter()
                .map(|update| {
                    let orphan_hint = {
                        let members = os::list_live_process_group_members(update.pid);
                        if members.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "\nORPHAN_PROCESS: the exited job pid={} left these programs still running: [{}]. They will keep running until stopped. Check what they are (e.g. `ps -fp <pid>`) and stop them with `kill <pid>` if they are leftovers.",
                                update.pid,
                                members
                                    .iter()
                                    .map(|pid| pid.to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    };
                    format!(
                        "RUNNING_JOB_UPDATE: pid={}, {}, cmd={}, now exits. elapsed time={}ms\nExit status: {}{}\n{}:\n{}{}",
                        update.pid,
                        update.description(),
                        compact_text(&update.command, 500),
                        update.elapsed_ms,
                        update.status,
                        update.capture_error.as_ref().map(|error| format!("\nCapture error: {error}; output is partial; out-of-scope processes may still be running.")).unwrap_or_default(),
                        if update.capture_error.is_some() { "Captured output (partial)" } else { "Final output" },
                        compact_text(&update.output, 4000),
                        orphan_hint,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn submit_running_job_updates(
        &mut self,
        updates: Vec<ShellJobExitUpdate>,
        persist_killed_notice: bool,
    ) {
        // Kill-looking exits (e.g. SIGKILL/OOM) are facts the model cannot
        // diagnose from the job output alone. Emit them wherever the exit
        // lands — including the async exit-listener path that never passes
        // through the request-building snapshots — as a persistent sysstat
        // notice so it is consumed, never dropped.
        if persist_killed_notice {
            let killed_notice = runtime_info::killed_jobs_notice(
                &updates
                    .iter()
                    .map(|update| runtime_info::JobExitSnapshot {
                        pid: update.pid,
                        tool_call_id: update.tool_call_id.clone(),
                        command: update.command.clone(),
                        elapsed_ms: update.elapsed_ms,
                        status: update.status.clone(),
                    })
                    .collect::<Vec<_>>(),
            );
            if let Some(notice) = killed_notice {
                self.submit_prompt_component(
                    PromptComponentRole::system(),
                    "job_killed",
                    notice,
                    "runtime",
                );
            }
        }
        let Some(text) = Self::format_running_job_updates(&updates) else {
            return;
        };
        self.submit_prompt_component(
            PromptComponentRole::system(),
            "running_job_update",
            text,
            "runtime",
        );
    }

    /// One observation point for the disk pressure tracker. The tracker
    /// invokes the filesystem sampler only when its count or time gate is
    /// due, keeping ordinary tool completions and model requests free of
    /// mount enumeration and stat calls.
    fn observe_disk_pressure(
        &mut self,
        running: &[runtime_info::RunningJobSnapshot],
    ) -> Option<String> {
        #[cfg(test)]
        let override_sample = self.disk_free_override.map(|(free, capacity)| {
            self.disk_pressure
                .sample_with_totals_for_test(free, capacity)
        });
        let mut filesystems = Vec::new();
        #[cfg(test)]
        let mut sampled = false;
        let event = self.disk_pressure.observe_with(|| {
            #[cfg(test)]
            {
                sampled = true;
            }
            #[cfg(test)]
            if let Some(sample) = override_sample {
                return Some(sample);
            }
            filesystems = Self::filesystems_for_info(running);
            runtime_info::DiskSample::from_filesystems(&filesystems)
        });
        #[cfg(test)]
        if sampled {
            self.disk_sample_count = self.disk_sample_count.saturating_add(1);
        }
        // Persist immediately: even when the current request path takes an
        // early return, the notice rides the next request instead of being
        // dropped. Exit-event-like notices must be consumed, not lost.
        let notice = event.map(|event| event.render(&filesystems));
        if let Some(notice) = &notice {
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "disk_pressure",
                notice.clone(),
                "runtime",
            );
        }
        notice
    }

    /// Sample filesystem usage for every disk the current work may write
    /// to: the session working directory plus each running job's cwd,
    /// deduplicated by device id so one disk reports once.
    fn filesystems_for_info(
        running: &[runtime_info::RunningJobSnapshot],
    ) -> Vec<runtime_info::FilesystemUsage> {
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(dir) = std::env::current_dir() {
            paths.push(dir);
        }
        for job in running {
            if job.cwd.trim().is_empty() {
                continue;
            }
            paths.push(std::path::PathBuf::from(&job.cwd));
        }
        os::filesystem_usage_snapshot(&paths)
    }

    pub fn build_model_request_prompt(&mut self, current_prompt: &str) -> String {
        self.build_model_request_prompt_inner(current_prompt, None)
    }

    pub(crate) fn build_model_request_prompt_with_runtime(
        &mut self,
        current_prompt: &str,
        runtime: &mut dyn ActionRuntime,
    ) -> String {
        self.build_model_request_prompt_inner(current_prompt, Some(runtime))
    }

    fn build_model_request_prompt_inner(
        &mut self,
        current_prompt: &str,
        runtime: Option<&mut dyn ActionRuntime>,
    ) -> String {
        let session_id = self.current_session_id();
        let first_snapshot = self.shell_jobs.consume_completed_for_session(&session_id);
        let shell_jobs = self.shell_jobs.clone();
        self.build_model_request_prompt_from_job_snapshots(
            current_prompt,
            runtime,
            first_snapshot,
            move || shell_jobs.consume_completed_for_session(&session_id),
        )
    }

    fn build_model_request_prompt_from_job_snapshots<F>(
        &mut self,
        current_prompt: &str,
        runtime: Option<&mut dyn ActionRuntime>,
        (mut running, mut updates): (Vec<RunningShellJob>, Vec<ShellJobExitUpdate>),
        final_scan: F,
    ) -> String
    where
        F: FnOnce() -> (Vec<RunningShellJob>, Vec<ShellJobExitUpdate>),
    {
        let (body, trailer) = prompt_render::split_formatted_response_trailer(current_prompt);
        let mut prompt = body.trim_end().to_string();

        // Capture state changes that race the first snapshot. Preserve jobs seen
        // in the first scan so a job that finishes between scans is rendered in
        // historical order (running, then exit), and merge newly registered jobs
        // from the final scan so their still-running state is never omitted.
        let (final_running, final_updates) = final_scan();
        let mut known_running_pids = running
            .iter()
            .map(|job| job.pid)
            .collect::<std::collections::HashSet<_>>();
        running.extend(
            final_running
                .into_iter()
                .filter(|job| known_running_pids.insert(job.pid)),
        );
        running.sort_by_key(|job| (job.created_at_ms, job.pid));
        updates.extend(final_updates);

        let has_still_running = !running.is_empty();
        let running_snapshot_for_info: Vec<runtime_info::RunningJobSnapshot> = running
            .iter()
            .map(|job| runtime_info::RunningJobSnapshot {
                pid: job.pid,
                tool_call_id: job.tool_call_id.clone(),
                command: job.command.clone(),
                cwd: job.cwd.clone(),
                created_at_ms: job.created_at_ms,
                elapsed_ms: job.elapsed_ms(),
                notes: job.notes.clone(),
            })
            .collect();
        let updates_snapshot_for_info: Vec<runtime_info::JobExitSnapshot> = updates
            .iter()
            .map(|update| runtime_info::JobExitSnapshot {
                pid: update.pid,
                tool_call_id: update.tool_call_id.clone(),
                command: update.command.clone(),
                elapsed_ms: update.elapsed_ms,
                status: update.status.clone(),
            })
            .collect();
        // RUNTIME_INFO is request-local. It is built only when a registered
        // reporter has important state and is never persisted into history.
        // Model API request observation point for disk pressure sampling.
        let api_disk_notice = self.observe_disk_pressure(&running_snapshot_for_info);
        let runtime_info = {
            // Adoption/reap transitions are internal lifecycle bookkeeping.
            // Drain them so the bounded queue cannot accumulate, but expose
            // only current actionable state below (live/zombie fallback
            // children and stale process scopes).
            let _ = os::take_orphan_process_events();
            let session_id = self.current_session_id();
            let inputs = runtime_info::RuntimeInfoInputs {
                running: running_snapshot_for_info,
                updates: updates_snapshot_for_info,
                stale_process_scopes: os::stale_process_scope_snapshots(&session_id)
                    .into_iter()
                    .map(|scope| runtime_info::StaleProcessScopeSnapshot {
                        owner_pid: scope.owner_pid,
                        notes: scope.observation_note,
                    })
                    .collect(),
                fallback_processes: os::fallback_process_snapshots()
                    .into_iter()
                    .map(|process| runtime_info::FallbackProcessSnapshot {
                        pid: process.pid,
                        notes: os::process_observation_note(process.pid),
                        process_name: process.process_name,
                        zombie: process.zombie,
                    })
                    .collect(),
                disk_pressure_notice: api_disk_notice,
            };
            runtime_info::default_registry().render(&inputs)
        };
        if let Some(runtime_info) = runtime_info.as_ref() {
            prompt.push_str("\n\n");
            prompt.push_str(runtime_info);
        }
        if let Some(update_text) = Self::format_running_job_updates(&updates) {
            prompt.push_str("\n\n");
            prompt.push_str(&update_text);
        }
        if let Some(runtime) = runtime {
            let events = updates
                .iter()
                // Jobs whose finish topic was already published through the
                // manager's exit listener must not emit a duplicate topic;
                // the textual RUNNING_JOB_UPDATE below still goes to the model.
                .filter(|update| !update.topic_published)
                .map(host::running_shell_job_exit_topic_event)
                .collect::<Vec<_>>();
            if !events.is_empty() {
                runtime.on_core_topic_events(&events);
            }
        }
        // Persist terminal updates for later prompts, but do not re-render that delta into this
        // request: the request-local copy above has the authoritative ordering.
        self.submit_running_job_updates(updates.clone(), false);
        self.flush_pending_prompt_components();

        if !has_still_running
            && updates.is_empty()
            && runtime_info.is_none()
            && !self.context_compress_required
        {
            if let Some(trailer) = self.take_memo_deleted_trailer() {
                let (body, response_trailer) =
                    prompt_render::split_formatted_response_trailer(current_prompt);
                let mut prompt = body.trim_end().to_string();
                prompt.push_str(&trailer);
                if let Some(response_trailer) = response_trailer {
                    prompt.push_str("\n\n");
                    prompt.push_str(&response_trailer);
                }
                return prompt;
            }
            return current_prompt.to_string();
        }
        if let Some(trailer) = self.take_memo_deleted_trailer() {
            prompt.push_str(&trailer);
        }
        prompt.push_str("\n\n");
        // The manual wording persists across retries (like the threshold
        // wording) until the compaction succeeds: the context may not be over
        // the limit, so retries must not fall back to "Context is too long".
        if self.context_compress_required {
            if self.manual_compact_trailer_pending {
                prompt.push_str(prompt_render::MANUAL_CONTEXT_COMPRESS_TRAILER);
            } else {
                prompt.push_str(prompt_render::CONTEXT_COMPRESS_REQUIRED_TRAILER);
            }
        } else if let Some(trailer) = trailer {
            prompt.push_str(&trailer);
        }
        prompt
    }

    pub fn should_suppress_model_response(&self, response: &LlmResponse) -> bool {
        if !self.context_compress_required {
            return false;
        }
        let mut parsed = if self.resolved_tool_call_mode == ToolCallMode::Native {
            self.parse_native_response(response)
        } else {
            self.response_protocol
                .suite()
                .parse(&response.content, &self.capabilities)
        };
        self.normalize_intrinsic_actions(&mut parsed);
        parsed.context_compresses.len() != 1 || parsed.repair_issue.is_some()
    }

    pub fn set_max_llm_input_tokens(&mut self, max_llm_input_tokens: u32) {
        self.max_llm_input_tokens = max_llm_input_tokens.max(3_000);
    }

    pub fn context_compress_threshold_percent(&self) -> u8 {
        self.context_compress_threshold_percent
    }

    pub fn set_context_compress_threshold_percent(&mut self, percent: u8) -> Result<(), String> {
        self.context_compress_threshold_percent =
            validate_context_compress_threshold_percent(percent)?;
        // Runtime updates take effect before the next model request. Re-evaluate
        // already accumulated context immediately instead of waiting for new
        // user/tool text to happen to trigger another shrink review. Preserve a
        // user-requested compact and the single already-scheduled quality
        // follow-up: those are explicit/in-flight maintenance, not threshold
        // admission decisions.
        if !self.manual_compact_trailer_pending
            && self.threshold_compaction_followup_state
                != ThresholdCompactionFollowupState::FollowupPending
        {
            self.context_compress_required = false;
            self.threshold_compaction_reasoning_required = false;
            self.pending_compact_request_notice = None;
            self.require_context_compress_if_needed(0);
        }
        Ok(())
    }
    pub fn configure_runtime_from_host(
        &mut self,
        config: &ModelServiceConfig,
        bash_approval_mode: BashApprovalMode,
    ) {
        self.set_max_llm_input_tokens(config.max_llm_input_tokens);
        self.set_bash_approval_mode(bash_approval_mode);
    }
    pub fn apply_runtime_config_update(
        &mut self,
        config: &mut ModelServiceConfig,
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
        let report = runtime_config_apply_report(
            config,
            *bash_approval_mode,
            *work_instruction_mode,
            field,
            effect,
        );
        self.self_tool
            .set_env_value(report.key, report.value.clone());
        if field == RuntimeConfigField::ApiProtocol {
            self.self_tool
                .set_env_value("TIMEM_BASE_URL", config.base_url.clone());
        }
        self.notify_runtime_config_changed();
        Ok(report)
    }
    pub fn set_max_rounds(&mut self, max_rounds: u32) {
        self.configured_round_budget = max_rounds.max(1);
        self.round_budget = self.configured_round_budget;
        self.self_tool.set_env_value(
            "TIMEM_MAX_ROUNDS",
            if self.configured_round_budget == UNLIMITED_ROUND_BUDGET {
                "unlimited".to_string()
            } else {
                self.configured_round_budget.to_string()
            },
        );
    }
    pub fn set_reminder_tips_config(&mut self, config: ReminderTipsConfig) {
        self.reminder_tips_config = config;
    }
    pub fn reminder_tips_config(&self) -> &ReminderTipsConfig {
        &self.reminder_tips_config
    }
    pub fn notify_runtime_config_changed(&mut self) {
        self.runtime_config_changed_notice_pending = true;
    }
    pub fn set_self_tool_runtime_param(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.self_tool.set_env_value(key, value);
    }
    fn refresh_rendered_static_prompt(&mut self) {
        let static_prompt = if self.runtime_system_context.is_empty() {
            self.static_prompt.clone()
        } else {
            format!(
                "{}\n\n## Session Runtime Identity\n\n{}",
                self.static_prompt.trim_end(),
                self.runtime_system_context
            )
        };
        self.rendered_static_prompt = prompt_render::render_static_prompt_for_mode_with_preferences(
            &static_prompt,
            &self.capabilities,
            self.response_protocol.suite(),
            &self.assistant_speaker_name,
            self.resolved_tool_call_mode,
            self.interface_preferences,
        );
    }
    pub fn set_capability_registry(&mut self, capabilities: CapabilityRegistry) {
        self.capabilities = capabilities.without_tool("toolgen");
        self.refresh_rendered_static_prompt();
    }

    pub fn configure_mcp(
        &mut self,
        base_capabilities: CapabilityRegistry,
        runtime: mcp::McpRuntime,
        servers: Vec<mcp::McpServerConfig>,
        tools: Vec<mcp::McpTool>,
    ) -> Result<(), String> {
        self.configure_mcp_with_instructions(
            base_capabilities,
            runtime,
            servers,
            tools,
            BTreeMap::new(),
        )
    }

    pub fn configure_mcp_with_instructions(
        &mut self,
        base_capabilities: CapabilityRegistry,
        runtime: mcp::McpRuntime,
        servers: Vec<mcp::McpServerConfig>,
        tools: Vec<mcp::McpTool>,
        instructions: BTreeMap<String, String>,
    ) -> Result<(), String> {
        self.replace_mcp_state(base_capabilities, runtime, servers, tools, instructions)?;
        self.append_initial_mcp_state_delta();
        Ok(())
    }

    fn replace_mcp_state(
        &mut self,
        base_capabilities: CapabilityRegistry,
        runtime: mcp::McpRuntime,
        servers: Vec<mcp::McpServerConfig>,
        tools: Vec<mcp::McpTool>,
        instructions: BTreeMap<String, String>,
    ) -> Result<(), String> {
        self.capabilities = base_capabilities
            .with_mcp_tools(&tools)?
            .without_tool("toolgen");
        self.mcp_runtime = runtime;
        self.mcp_servers = servers
            .into_iter()
            .map(|server| (server.id.clone(), server))
            .collect();
        self.mcp_tools = tools
            .into_iter()
            .map(|tool| (tool.action_name.clone(), tool))
            .collect();
        self.mcp_instructions = instructions
            .into_iter()
            .filter_map(|(server_id, instructions)| {
                let instructions = instructions.trim();
                (!instructions.is_empty() && self.mcp_servers.contains_key(&server_id))
                    .then(|| (server_id, bounded_mcp_server_instructions(instructions)))
            })
            .collect();
        self.refresh_rendered_static_prompt();
        Ok(())
    }

    fn append_initial_mcp_state_delta(&mut self) {
        let visible_server_ids = self
            .mcp_tools
            .values()
            .map(|tool| tool.server_id.as_str())
            .chain(self.mcp_instructions.keys().map(String::as_str))
            .collect::<HashSet<_>>();
        let lines = self
            .mcp_servers
            .values()
            .filter(|server| visible_server_ids.contains(server.id.as_str()))
            .map(|server| mcp_server_update_line(server, McpServerUpdate::Enabled))
            .collect::<Vec<_>>();
        if lines.is_empty() {
            return;
        }
        self.append_delta(vec![(
            "mcp_capability_update".to_string(),
            lines.join("\n"),
        )]);
    }

    fn current_inline_mcp_section(&self) -> Option<String> {
        if self.resolved_tool_call_mode == ToolCallMode::Native {
            return None;
        }
        let tools = self
            .capabilities
            .render_mcp_tool_catalog_markdown_for_protocol(self.response_protocol.name());
        if tools.trim().is_empty() && self.mcp_instructions.is_empty() {
            return None;
        }

        let mut sections = vec![
            "## Current MCP Capabilities".to_string(),
            "These are the MCP capabilities currently available. Use the current definitions below, and apply each server-wide instruction to that server's tools.".to_string(),
        ];
        if !self.mcp_instructions.is_empty() {
            sections.push("### MCP server-wide instructions".to_string());
            sections.extend(
                self.mcp_instructions
                    .iter()
                    .map(|(server_id, instructions)| {
                        let label = self
                            .mcp_servers
                            .get(server_id)
                            .map(mcp_server_label)
                            .unwrap_or_else(|| server_id.clone());
                        format!("#### {label}\n\n{instructions}")
                    }),
            );
        }
        if !tools.trim().is_empty() {
            sections.push("### Available MCP tools".to_string());
            sections.push(tools);
        }
        Some(sections.join("\n\n"))
    }

    fn render_prompt_from_deltas(&self, deltas: &[PromptDelta]) -> String {
        let rendered = prompt_render::render_prompt_with_rendered_static_for_mode(
            &self.rendered_static_prompt,
            deltas,
            &self.assistant_speaker_name,
            self.response_protocol.suite(),
            self.resolved_tool_call_mode,
        );
        let Some(mcp_section) = self.current_inline_mcp_section() else {
            return rendered;
        };
        let (body, trailer) = prompt_render::split_formatted_response_trailer(&rendered);
        let mut prompt = format!("{}\n\n{}", body.trim_end(), mcp_section);
        if let Some(trailer) = trailer {
            prompt.push_str("\n\n");
            prompt.push_str(&trailer);
        }
        prompt
    }

    pub fn apply_mcp_update(
        &mut self,
        base_capabilities: CapabilityRegistry,
        runtime: mcp::McpRuntime,
        servers: Vec<mcp::McpServerConfig>,
        tools: Vec<mcp::McpTool>,
    ) -> Result<bool, String> {
        self.apply_mcp_update_with_instructions(
            base_capabilities,
            runtime,
            servers,
            tools,
            self.mcp_instructions.clone(),
        )
    }

    pub fn apply_mcp_update_with_instructions(
        &mut self,
        base_capabilities: CapabilityRegistry,
        runtime: mcp::McpRuntime,
        servers: Vec<mcp::McpServerConfig>,
        tools: Vec<mcp::McpTool>,
        instructions: BTreeMap<String, String>,
    ) -> Result<bool, String> {
        let previous_tools = self.mcp_tools.clone();
        let previous_servers = self.mcp_servers.clone();
        let previous_instructions = self.mcp_instructions.clone();
        let previous_model_tools = self
            .capabilities
            .native_dynamic_tool_definitions()
            .into_iter()
            .map(|tool| (tool.name.clone(), tool))
            .collect::<BTreeMap<_, _>>();
        self.replace_mcp_state(base_capabilities, runtime, servers, tools, instructions)?;
        let current_model_tools = self
            .capabilities
            .native_dynamic_tool_definitions()
            .into_iter()
            .map(|tool| (tool.name.clone(), tool))
            .collect::<BTreeMap<_, _>>();
        if previous_model_tools == current_model_tools
            && previous_instructions == self.mcp_instructions
        {
            return Ok(false);
        }

        let previous_names = previous_model_tools
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let current_names = current_model_tools.keys().cloned().collect::<BTreeSet<_>>();
        let added = current_names
            .difference(&previous_names)
            .cloned()
            .collect::<Vec<_>>();
        let removed = previous_names
            .difference(&current_names)
            .cloned()
            .collect::<Vec<_>>();
        let updated = current_names
            .intersection(&previous_names)
            .filter(|name| previous_model_tools.get(*name) != current_model_tools.get(*name))
            .cloned()
            .collect::<Vec<_>>();
        let previous_visible_server_ids = previous_tools
            .values()
            .map(|tool| tool.server_id.as_str())
            .chain(previous_instructions.keys().map(String::as_str))
            .collect::<HashSet<_>>();
        let current_visible_server_ids = self
            .mcp_tools
            .values()
            .map(|tool| tool.server_id.as_str())
            .chain(self.mcp_instructions.keys().map(String::as_str))
            .collect::<HashSet<_>>();
        let removed_servers = previous_servers
            .keys()
            .filter(|id| {
                previous_visible_server_ids.contains(id.as_str())
                    && !self.mcp_servers.contains_key(*id)
            })
            .map(|id| &previous_servers[id])
            .collect::<Vec<_>>();
        let added_servers = self
            .mcp_servers
            .keys()
            .filter(|id| {
                current_visible_server_ids.contains(id.as_str())
                    && !previous_servers.contains_key(*id)
            })
            .map(|id| &self.mcp_servers[id])
            .collect::<Vec<_>>();

        let mut lines = Vec::new();
        for server in removed_servers {
            lines.push(mcp_server_update_line(server, McpServerUpdate::Disabled));
        }
        for server in added_servers {
            lines.push(mcp_server_update_line(server, McpServerUpdate::Enabled));
        }
        if !added.is_empty() {
            lines.push(format!(
                "MCP update: newly available actions: {}.",
                added.join(", ")
            ));
        }
        if !updated.is_empty() {
            lines.push(format!(
                "MCP update: updated action definitions: {}.",
                updated.join(", ")
            ));
        }
        if !removed.is_empty() {
            lines.push(format!(
                "MCP update: actions no longer available: {}.",
                removed.join(", ")
            ));
        }
        let removed_instruction_ids = previous_instructions
            .keys()
            .filter(|id| !self.mcp_instructions.contains_key(*id))
            .filter(|id| self.mcp_servers.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        let changed_instruction_ids = self
            .mcp_instructions
            .keys()
            .filter(|id| previous_instructions.get(*id) != self.mcp_instructions.get(*id))
            .cloned()
            .collect::<Vec<_>>();
        for server_id in &removed_instruction_ids {
            let label = self
                .mcp_servers
                .get(server_id)
                .map(mcp_server_label)
                .unwrap_or_else(|| server_id.clone());
            lines.push(format!(
                "MCP update: instructions for MCP {label} ARE NO LONGER ACTIVE !!!"
            ));
        }
        for server_id in &changed_instruction_ids {
            let label = self
                .mcp_servers
                .get(server_id)
                .map(mcp_server_label)
                .unwrap_or_else(|| server_id.clone());
            lines.push(format!(
                "MCP update: instructions for MCP {label} ARE UPDATED."
            ));
        }
        if !lines.is_empty() {
            self.append_delta(vec![(
                "mcp_capability_update".to_string(),
                lines.join("\n"),
            )]);
        }
        Ok(true)
    }

    /// Temporarily enables the ToolGen capability for a Session-owned ToolGen use-case.
    pub fn enable_toolgen_capability(&mut self) -> Result<(), String> {
        self.capabilities.enable_toolgen()?;
        self.refresh_rendered_static_prompt();
        Ok(())
    }
    /// Restores the normal Agent capability set after a Session-owned ToolGen use-case.
    pub fn disable_toolgen_capability(&mut self) {
        self.capabilities.disable_toolgen();
        self.refresh_rendered_static_prompt();
    }
    pub fn set_response_protocol(&mut self, protocol: ResponseProtocolKind) {
        self.configured_inline_response_protocol = protocol;
        if self.resolved_tool_call_mode != ToolCallMode::Native {
            self.response_protocol = protocol;
        }
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
        let mut self_tool = SelfToolState::new(
            env,
            paths,
            default_self_tool_about(),
            default_self_tool_process(),
        );
        self_tool.set_env_value(
            "TIMEM_MAX_ROUNDS",
            if self.configured_round_budget == UNLIMITED_ROUND_BUDGET {
                "unlimited".to_string()
            } else {
                self.configured_round_budget.to_string()
            },
        );
        self.self_tool = self_tool;
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
    pub fn capability_contains_tool(&self, action: &str) -> bool {
        self.capabilities.contains_tool(action)
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
    fn dynamic_context_token_estimate(&self) -> DynamicContextTokenEstimate {
        let mut visible_delta_ids = BTreeSet::new();
        let mut visible_slice_count = 0usize;
        let mut text_tokens = 0_u32;
        for delta in &self.deltas {
            // Native rendering retains delta boundaries even without text slices:
            // their owned tool exchanges still enter the model request.
            if self.resolved_tool_call_mode == ToolCallMode::Native {
                visible_delta_ids.insert(delta.delta_id.clone());
            }
            for slice in
                prompt_render::render_delta_slices_for_mode(delta, self.resolved_tool_call_mode)
            {
                visible_delta_ids.insert(delta.delta_id.clone());
                visible_slice_count += 1;
                text_tokens = text_tokens.saturating_add(estimate_prompt_tokens(&slice.text));
            }
        }
        let native_tokens = self
            .native_exchanges
            .iter()
            .filter(|exchange| visible_delta_ids.contains(&exchange.delta_id))
            .map(estimate_native_exchange_tokens)
            .fold(0_u32, u32::saturating_add);
        DynamicContextTokenEstimate {
            visible_delta_count: visible_delta_ids.len(),
            visible_slice_count,
            text_tokens,
            native_tokens,
        }
    }

    pub fn dynamic_context_summary(&self) -> CoreDynamicContextSummary {
        let estimate = self.dynamic_context_token_estimate();
        CoreDynamicContextSummary {
            visible_delta_count: estimate.visible_delta_count,
            visible_slice_count: estimate.visible_slice_count,
            estimated_tokens: estimate.total_tokens(),
        }
    }
    pub fn dynamic_context_estimated_tokens(&self) -> u32 {
        self.dynamic_context_summary().estimated_tokens
    }
    pub fn export_dynamic_context(&self) -> DynamicContextSnapshot {
        DynamicContextSnapshot {
            deltas: self.deltas.clone(),
            native_exchanges: self.native_exchanges.clone(),
            last_observed_prompt_tokens: self.last_observed_prompt_tokens,
            active_memo: self.active_memo.clone(),
            pending_forcible_memo_note: self.pending_forcible_memo_note.clone(),
            pending_interrupted_memo_note: self.pending_interrupted_memo_note.clone(),
        }
    }

    pub fn import_dynamic_context(&mut self, snapshot: DynamicContextSnapshot) {
        // Import is a wholesale replacement, including an empty snapshot. A
        // reused worker must not retain text, native tool exchanges, pending
        // components, or one-shot notices from the context being replaced.
        self.clear_dynamic_context();
        // The memo is intentionally NOT reactivated: after a restart the
        // reminder must go inactive and the model is told to recreate it if
        // still necessary. The snapshot value is consumed by the Host for
        // the resume notice instead.
        self.deltas = snapshot.deltas;
        self.native_exchanges = snapshot.native_exchanges;
        self.last_observed_prompt_tokens = snapshot.last_observed_prompt_tokens;
        // Pending runtime-authority memo notices are runtime state for the
        // next turn; they must survive a restart with the context they
        // belong to.
        self.pending_forcible_memo_note = snapshot.pending_forcible_memo_note;
        self.pending_interrupted_memo_note = snapshot.pending_interrupted_memo_note;
        if let Some(max_seq) = self
            .deltas
            .iter()
            .map(|delta| delta.delta_id.rsplit('_').next())
            .filter_map(|tail| tail.and_then(|t| t.parse::<u64>().ok()))
            .max()
        {
            self.next_delta_sequence = self.next_delta_sequence.max(max_seq + 1);
        }
    }

    pub fn clear_dynamic_context(&mut self) {
        self.deltas.clear();
        self.native_exchanges.clear();
        self.pending_native_exchange = None;
        self.pending_prompt_components.clear();
        self.pending_user_interruption_note = false;
        self.pending_forcible_memo_note = None;
        self.pending_interrupted_memo_note = None;
        self.memo_deleted_this_turn = None;
        self.memo_deleted_trailer_shown = false;
        self.touched_paths.clear();
        self.last_observed_prompt_tokens = 0;
        self.context_compress_required = false;
        self.threshold_compaction_reasoning_required = false;
        self.manual_compact_trailer_pending = false;
        self.pending_compact_request_notice = None;
        self.threshold_compaction_followup_state = ThresholdCompactionFollowupState::Available;
        self.post_compaction_verification = None;
        self.current_round = 0;
        self.current_stats = UsageStats::zero();
        self.repair_attempted = false;
        self.repair_attempts = 0;
        self.last_repair_issue = None;
        self.pending_approval = None;
        self.current_action_turn_id = None;
        self.current_session_id = None;
        self.process_scope_prompted_session = None;
        self.current_action_user_question.clear();
        self.last_notifications.clear();
        self.turn_finished_summary = None;
        self.loaded_work_instruction_fingerprints.clear();
        // The memo is persisted beside the dynamic context as one consistency
        // unit; clearing the context clears the reminder with it.
        self.active_memo = None;
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

    fn text_contains_complete_worker_role_instruction(text: &str, instruction: &str) -> bool {
        text.match_indices(instruction).any(|(start, _)| {
            let before = &text[..start];
            let after = &text[start + instruction.len()..];
            (before.is_empty() || before.ends_with("\n\n"))
                && (after.is_empty() || after.starts_with("\n\n") || after.starts_with("\n## "))
        })
    }

    fn current_visible_context_contains(&self, instruction: &str) -> bool {
        if instruction.is_empty() {
            return true;
        }
        if self.pending_prompt_components.iter().any(|component| {
            Self::text_contains_complete_worker_role_instruction(&component.content, instruction)
        }) {
            return true;
        }
        self.deltas.iter().any(|delta| {
            let hidden = delta.hidden_slice_ids.iter().collect::<HashSet<_>>();
            let mut visible_component = String::new();
            let mut current_component_id: Option<&str> = None;
            for slice in &delta.slices {
                let component_id = if slice.component_id.is_empty() {
                    slice.slice_id.as_str()
                } else {
                    slice.component_id.as_str()
                };
                let component_changed = current_component_id
                    .map(|current| current != component_id)
                    .unwrap_or(false);
                if component_changed || hidden.contains(&slice.slice_id) {
                    if Self::text_contains_complete_worker_role_instruction(
                        &visible_component,
                        instruction,
                    ) {
                        return true;
                    }
                    visible_component.clear();
                }
                current_component_id = Some(component_id);
                if hidden.contains(&slice.slice_id) {
                    current_component_id = None;
                } else {
                    visible_component.push_str(&slice.text);
                }
            }
            Self::text_contains_complete_worker_role_instruction(&visible_component, instruction)
        })
    }

    fn filter_repeated_worker_roles(&self, supporting_context: &str) -> String {
        let mut rendered_in_this_context = HashSet::new();
        supporting_context
            .lines()
            .map(|line| {
                let Some(payload) = line.strip_prefix(WORKER_ROLE_CONTEXT_PREFIX) else {
                    return line.to_string();
                };
                let Ok(payload) = serde_json::from_str::<Value>(payload) else {
                    return line.to_string();
                };
                let Some(name) = payload.get("name").and_then(Value::as_str) else {
                    return line.to_string();
                };
                let Some(description) = payload.get("description").and_then(Value::as_str) else {
                    return line.to_string();
                };
                let spec = self.response_protocol.suite().prompt_boundaries();
                let full = worker_role_full_instruction(name, description, spec);
                if self.current_visible_context_contains(&full)
                    || rendered_in_this_context.contains(&full)
                {
                    worker_role_reference_instruction(name, spec)
                } else {
                    rendered_in_this_context.insert(full.clone());
                    full
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }

    fn filter_supporting_context(&mut self, supporting_context: &str) -> String {
        let context = self.filter_repeated_work_instructions(supporting_context);
        self.filter_repeated_worker_roles(&context)
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

    pub(crate) fn mark_user_interrupted_work(&mut self) {
        self.pending_user_interruption_note = true;
    }

    /// The single runtime-held memo for long-running work, if any.
    pub fn active_memo(&self) -> Option<&str> {
        self.active_memo.as_deref()
    }

    pub(crate) fn set_active_memo(&mut self, text: String) {
        self.active_memo = Some(text);
        // A new active goal supersedes any pending deletion reminder.
        self.memo_deleted_this_turn = None;
        self.memo_deleted_trailer_shown = false;
    }

    /// The turn is finishing while a memo is still active and the guard
    /// budget is exhausted. Close the memo on the runtime's authority and
    /// record the notice for the next turn's prompt.
    pub(crate) fn force_close_memo_on_finish(&mut self, runtime: &mut dyn ActionRuntime) {
        if let Some(memo) = self.active_memo.take() {
            self.pending_forcible_memo_note = Some(memo.clone());
            let session_id = self.current_session_id().to_string();
            runtime.on_core_topic_events(&[crate::host::memo_topic_event_with_op(
                session_id,
                None,
                "force_deleted",
            )]);
        }
    }

    /// A user stop/interrupt abandons the turn: close the memo on runtime
    /// authority and record the notice for the next turn.
    pub(crate) fn force_close_memo_on_interrupt(&mut self) -> Option<String> {
        let memo = self.active_memo.take()?;
        self.pending_interrupted_memo_note = Some(memo.clone());
        Some(memo)
    }

    pub(crate) fn clear_active_memo(&mut self) {
        if let Some(memo) = self.active_memo.take() {
            self.memo_deleted_this_turn = Some(memo);
        }
    }

    pub(crate) fn record_turn_finished(&mut self, summary: String) {
        if self.turn_finished_summary.is_none() {
            self.turn_finished_summary = Some(summary);
        }
    }

    pub(crate) fn take_turn_finished_summary(&mut self) -> Option<String> {
        self.turn_finished_summary.take()
    }

    pub fn begin_direct_resume_turn(&mut self, supporting_context: Option<&str>) -> CoreStep {
        self.begin_turn_with_input_kind("", supporting_context, true)
    }

    pub fn begin_turn(&mut self, user_input: &str, supporting_context: Option<&str>) -> CoreStep {
        self.begin_turn_with_input_kind(user_input, supporting_context, false)
    }

    fn begin_turn_with_input_kind(
        &mut self,
        user_input: &str,
        supporting_context: Option<&str>,
        direct_resume: bool,
    ) -> CoreStep {
        self.current_round = 0;
        self.round_budget = self.configured_round_budget;
        self.current_stats = UsageStats::zero();
        self.repair_attempted = false;
        self.repair_attempts = 0;
        self.last_repair_issue = None;
        self.pending_approval = None;
        self.last_notifications.clear();
        self.turn_finished_summary = None;
        self.memo_finish_guard_tokens = MEMO_FINISH_GUARD_TOKEN_CAP;
        self.memo_deleted_this_turn = None;
        self.memo_deleted_trailer_shown = false;
        let action_turn_id = unique_id("action_turn");
        self.current_action_turn_id = Some(action_turn_id.clone());
        self.current_action_user_question = user_input.trim().to_string();
        self.action_audit.begin_turn(
            &action_turn_id,
            now_ms(),
            &self.current_action_user_question,
        );
        let text = user_input.trim().to_string();
        if self.pending_user_interruption_note && (direct_resume || !text.is_empty()) {
            self.pending_user_interruption_note = false;
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "user_interrupted_work",
                "NOTE: User interrupted the above work. Continue it based on the user's new input's intent. If not sure, ask the user.",
                "runtime",
            );
        }
        if let Some(memo) = self.pending_interrupted_memo_note.take() {
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "memo_interrupted_deleted",
                format!(
                    "User interrupted the previous work and the runtime forcibly deleted its active memo: {memo:?} Recreate the memo if necessary based on the user's new input."
                ),
                "runtime",
            );
        }
        if let Some(memo) = self.pending_forcible_memo_note.take() {
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "memo_forcibly_deleted",
                format!(
                    "Last time you forcibly invoked task_finished without deleting the active memo: \"{memo}\" Now this memo has been deleted forcibly by runtime. Recreate it if necessary."
                ),
                "runtime",
            );
        }
        let should_memory_precheck = !text.is_empty()
            && supporting_context
                .map(should_run_memory_precheck)
                .unwrap_or(false);
        let filtered_supporting_context = supporting_context
            .map(|ctx| self.filter_supporting_context(ctx))
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
        self.require_context_compress_if_needed(incoming_prompt_tokens);
        for system_text in system_texts {
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "runtime_note",
                system_text,
                "runtime",
            );
        }
        // Supporting context describes state already present at turn admission
        // (notably restart/history notices), so it precedes the user input.
        if direct_resume || !text.is_empty() {
            self.submit_prompt_component(
                PromptComponentRole::user(),
                if direct_resume {
                    "user_resume_directly"
                } else {
                    "user_question"
                },
                text,
                "user_input",
            );
        }
        if should_memory_precheck {
            let result = self.runtime_memory_precheck(user_input, 5);
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "result_of_llm_action",
                result,
                "runtime_memory_precheck",
            );
        }
        self.submit_cwd_note_if_pending();
        CoreStep::NeedModel {
            prompt: self.build_next_prompt(),
            rounds_remaining: self.round_budget,
        }
    }
    pub fn append_user_supplement(&mut self, text: &str) -> Option<CoreStep> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        self.submit_prompt_component(
            PromptComponentRole::user(),
            "user_supplement",
            text,
            "user_input",
        );
        Some(CoreStep::NeedModel {
            prompt: self.build_next_prompt(),
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
        let mut added = false;
        for supplement in supplements {
            let supplement = supplement.trim();
            if supplement.is_empty() {
                continue;
            }
            let _ = append_audit_event(
                audit_file,
                &user_supplement_audit_event(session, turn_id, supplement),
            );
            self.submit_prompt_component(
                PromptComponentRole::user(),
                "user_supplement",
                supplement,
                "user_input",
            );
            added = true;
        }
        added.then(|| CoreStep::NeedModel {
            prompt: self.build_next_prompt(),
            rounds_remaining: self.remaining_rounds(),
        })
    }

    pub fn append_user_supplements_with_context_and_audit(
        &mut self,
        supplements: impl IntoIterator<Item = UserSupplement>,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
    ) -> Option<CoreStep> {
        let mut added = false;
        for supplement in supplements {
            let text = supplement.text.trim();
            if text.is_empty() {
                continue;
            }
            let _ = append_audit_event(
                audit_file,
                &user_supplement_audit_event(session, turn_id, text),
            );
            if let Some(context) = supplement
                .additional_context
                .as_deref()
                .map(|context| self.filter_supporting_context(context))
                .filter(|context| !context.trim().is_empty())
            {
                self.submit_prompt_component(
                    PromptComponentRole::system(),
                    "user_supplement_context",
                    context,
                    "host_context",
                );
            }
            self.submit_prompt_component(
                PromptComponentRole::user(),
                "user_supplement",
                text,
                "user_input",
            );
            added = true;
        }
        added.then(|| CoreStep::NeedModel {
            prompt: self.build_next_prompt(),
            rounds_remaining: self.remaining_rounds(),
        })
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
        let raw_model_output = response.content.clone();
        let protocol_suite = self.response_protocol.suite();
        let mut native_calls = response.tool_calls.clone();
        let mut parsed = if self.resolved_tool_call_mode == ToolCallMode::Native {
            self.parse_native_response(&response)
        } else {
            protocol_suite.parse(&response.content, &self.capabilities)
        };
        self.normalize_intrinsic_actions(&mut parsed);
        let preview_accepted = parsed.repair_issue.is_none()
            && !response.truncated
            && (!self.context_compress_required || parsed.context_compresses.len() == 1);
        runtime.on_model_response_validated(preview_accepted, !parsed.continue_work);

        if self.context_compress_required
            && (parsed.context_compresses.len() != 1 || parsed.repair_issue.is_some())
        {
            self.current_round = self.current_round.saturating_sub(1);
            let mut prompt = self.render_prompt();
            let (body, response_trailer) = prompt_render::split_formatted_response_trailer(&prompt);
            prompt = body.trim_end().to_string();
            prompt.push_str("\n\n");
            if self.manual_compact_trailer_pending {
                prompt.push_str(prompt_render::MANUAL_CONTEXT_COMPRESS_TRAILER);
            } else {
                prompt.push_str(prompt_render::CONTEXT_COMPRESS_REQUIRED_TRAILER);
            }
            if let Some(response_trailer) = response_trailer {
                prompt.push_str("\n\n");
                prompt.push_str(&response_trailer);
            }
            return CoreStep::NeedModel {
                prompt,
                rounds_remaining: self.remaining_rounds(),
            };
        }
        if response.truncated && self.repair_attempts < MAX_PROTOCOL_REPAIR_ATTEMPTS {
            let instruction = protocol_suite
                .repair_instruction_for_response("truncated_model_output", &response.content);
            return self.request_protocol_repair(
                "truncated_model_output",
                &instruction,
                &response.content,
                runtime,
            );
        }
        let mut slices = Vec::new();
        if parsed.repair_issue.is_none() {
            let tool_count = parsed
                .action_groups
                .iter()
                .map(|group| group.actions.len())
                .sum();
            runtime.on_model_response_parsed(
                tool_count,
                !parsed.thought.trim().is_empty(),
                tool_count > 0 || !parsed.context_compresses.is_empty() || !native_calls.is_empty(),
            );
            if parsed.recovered_issue.as_deref() == Some("runtime_root_repair_help") {
                runtime.on_core_topic_events(&[host::runtime_root_repair_help_topic_event(
                    self.current_session_id(),
                )]);
            }
        }
        if let Some(issue) = parsed.repair_issue.clone() {
            if self.repair_attempts < MAX_PROTOCOL_REPAIR_ATTEMPTS {
                let instruction =
                    protocol_suite.repair_instruction_for_response(&issue, &response.content);
                return self.request_protocol_repair(
                    &issue,
                    &instruction,
                    &response.content,
                    runtime,
                );
            }
            if issue == "invalid_json"
                && protocol_suite.can_show_plain_text_after_repair_failure(&response.content)
            {
                let final_text = response.content.trim().to_string();
                slices.extend(self.assistant_replay_slices(
                    &raw_model_output,
                    None,
                    Some(&final_text),
                ));
                self.defer_next_turn_slices(slices);
                return CoreStep::Final(TurnFinal {
                    final_answer: final_text,
                    toolgen_retrospect: String::new(),
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
                    toolgen_retrospect: String::new(),
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
            slices.extend(self.assistant_replay_slices(
                &raw_model_output,
                Some(&parsed),
                Some(&final_text),
            ));
            self.defer_next_turn_slices(slices);
            return CoreStep::Final(TurnFinal {
                final_answer: final_text,
                toolgen_retrospect: parsed.toolgen_retrospect.clone(),
                stats: self.current_stats.clone(),
                profile_label: self.profile.label(),
                repair_issue: Some(issue),
                stop_summary: None,
            });
        }
        if parsed.context_compresses.is_empty() {
            if let Some(note) =
                self.verify_post_compaction_provider_usage(response.usage.prompt_tokens)
            {
                slices.push(("runtime_note".to_string(), note));
            }
        } else {
            // A new explicit compaction supersedes verification of the previous
            // one. Do not let the old measurement reclassify or constrain the
            // compaction currently being processed.
            self.post_compaction_verification = None;
        }
        self.last_notifications = notification::notifications_from_envelope(&parsed);
        if !self.last_notifications.is_empty() {
            let events = host::notification_topic_events(
                &self.current_session_id(),
                &self.last_notifications,
            );
            runtime.on_core_topic_events(&events);
        }
        let deferred_compact_replay = parsed.continue_work
            && self.resolved_tool_call_mode != ToolCallMode::Native
            && !parsed.context_compresses.is_empty();
        if parsed.continue_work
            && self.resolved_tool_call_mode != ToolCallMode::Native
            && !deferred_compact_replay
        {
            slices.extend(self.assistant_replay_slices(&raw_model_output, Some(&parsed), None));
        }
        let compact_result_slice_start = slices.len();
        let mut compacted_successfully = false;
        let mut successful_compact_summaries = Vec::new();
        let threshold_compaction_requested = self.threshold_compaction_reasoning_required;
        let mut threshold_followup_required = false;
        for compact in &parsed.context_compresses {
            // Keep semantics are fail-closed: every explicitly retained or
            // offloaded id must still be live. Unlike the old discard list, a
            // missing keep id can silently destroy context, so it is never
            // treated as idempotent success.
            let missing = self.missing_prompt_refs(&compact.delta_ids, &compact.slice_ids);
            let existing_delta_ids = self
                .deltas
                .iter()
                .map(|delta| delta.delta_id.clone())
                .collect::<Vec<_>>();
            let keep_delta_ids = compact
                .keep_delta_ids
                .iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect::<HashSet<_>>();
            let discarded_delta_ids = existing_delta_ids
                .iter()
                .filter(|id| !keep_delta_ids.contains(id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let live_offload_ids = compact.offload_delta_ids.clone();
            if missing.is_empty() {
                let estimated_before = self.dynamic_context_token_estimate();
                let offload_record = if live_offload_ids.is_empty() {
                    None
                } else {
                    match self.collect_prompt_context_for_scratch(&live_offload_ids, &[]) {
                        Ok(offload) => {
                            match self.scratch.write_record(
                                "context_offload",
                                "context compress offload",
                                &offload.content,
                                &offload.delta_ids,
                                &offload.slice_ids,
                            ) {
                                Ok(record) => Some(record),
                                Err(err) => {
                                    let outcome = ActionOutcome::failed(format!(
                                        "scratch_offload_failed: {err}"
                                    ))
                                    .with_runtime_metadata("error_type", "ScratchOffloadFailed");
                                    let result = self.format_context_compress_outcome(
                                        compact, &outcome, runtime,
                                    );
                                    slices.push(("result_of_llm_action".to_string(), result));
                                    continue;
                                }
                            }
                        }
                        Err(err) => {
                            let outcome =
                                ActionOutcome::failed(format!("scratch_offload_failed: {err}"))
                                    .with_runtime_metadata("error_type", "ScratchOffloadFailed");
                            let result =
                                self.format_context_compress_outcome(compact, &outcome, runtime);
                            slices.push(("result_of_llm_action".to_string(), result));
                            continue;
                        }
                    }
                };
                // The detailed shrink report and selected ids are internal accounting.
                // Re-injecting them would immediately spend the context that compaction
                // just recovered. A minimal runtime confirmation is enough for the
                // model; only an offload scratch id remains actionable afterward.
                let _shrink_report =
                    self.apply_prompt_shrink(&discarded_delta_ids, &compact.slice_ids);
                if let Some(record) = offload_record.as_ref() {
                    slices.push((
                        "runtime_note".to_string(),
                        format!(
                            "Context offload saved. Retrieve it with memmgr scratch read using scratch_id: {}",
                            record.id
                        ),
                    ));
                }
                let estimated_after = self.dynamic_context_token_estimate();
                let summary_tokens = estimate_prompt_tokens(&compact.summary);
                let estimated_before_tokens = estimated_before.total_tokens();
                let estimated_after_tokens = estimated_after
                    .total_tokens()
                    .saturating_add(summary_tokens);
                let compact_report = host::CoreContextCompressTopic {
                    estimated_before_tokens,
                    estimated_after_tokens,
                    estimated_text_before_tokens: estimated_before.text_tokens,
                    estimated_text_after_tokens: estimated_after
                        .text_tokens
                        .saturating_add(summary_tokens),
                    estimated_native_before_tokens: estimated_before.native_tokens,
                    estimated_native_after_tokens: estimated_after.native_tokens,
                    discarded_delta_ids: discarded_delta_ids.clone(),
                    offloaded_delta_ids: compact.offload_delta_ids.clone(),
                    scratch_id: offload_record.as_ref().map(|record| record.id.clone()),
                };
                runtime.on_core_topic_events(&[host::context_compress_topic_event(
                    self.current_session_id(),
                    &compact_report,
                )]);
                let verification = if threshold_compaction_requested {
                    Some(
                        if self.threshold_compaction_followup_state
                            == ThresholdCompactionFollowupState::FollowupPending
                        {
                            PostCompactionVerification::Followup
                        } else {
                            PostCompactionVerification::Initial
                        },
                    )
                } else {
                    None
                };
                let (force_followup, quality_note) = self.threshold_compaction_quality_note(
                    threshold_compaction_requested,
                    estimated_before_tokens,
                    estimated_after_tokens,
                );
                if threshold_compaction_requested && !force_followup && quality_note.is_none() {
                    self.post_compaction_verification = verification;
                } else if threshold_compaction_requested {
                    // A locally poor result already takes the existing bounded
                    // follow-up/warning path; there is no normal post-compact
                    // response to verify before that maintenance request.
                    self.post_compaction_verification = None;
                }
                threshold_followup_required |= force_followup;
                if let Some(note) = quality_note {
                    slices.push(("runtime_note".to_string(), note));
                }
                successful_compact_summaries.push(compact.summary.trim().to_string());
                compacted_successfully = true;
            } else {
                let outcome = ActionOutcome::failed(format!(
                    "invalid_prompt_refs\nmissing_ids: {}\ncurrent_live_delta_refs:\n{}",
                    missing.join(", "),
                    self.live_delta_refs_hint()
                ))
                .with_runtime_metadata("error_type", "InvalidPromptRefs")
                .with_runtime_metadata("missing_ids", json!(missing));
                let result = self.format_context_compress_outcome(compact, &outcome, runtime);
                slices.push(("result_of_llm_action".to_string(), result));
            }
        }
        if compacted_successfully {
            // The request succeeded; superseded maintenance instructions and
            // past failure echoes would only pollute future compactions.
            self.hide_prompt_slices_matching("force_shrink_required");
            self.hide_prompt_slices_matching("error: invalid_prompt_refs");
            self.hide_prompt_slices_matching("error: scratch_offload_failed");
        }
        if compacted_successfully {
            // A successful compaction is accepted as-is: shrink depth cannot
            // be quantified reliably by the model, so depth guidance lives in
            // the compact trailers (discard stale deltas/tool noise, extract
            // a valuable short summary) rather than a numeric gate.
            self.context_compress_required = threshold_followup_required;
            self.threshold_compaction_reasoning_required = threshold_followup_required;
            self.manual_compact_trailer_pending = false;
            self.process_scope_prompted_session = None;
            if let Some(note) = self.take_process_aggregate_scopes_if_needed() {
                slices.push(("runtime_note".to_string(), note));
            }
        }
        if compacted_successfully {
            // A successful compact gets a dedicated assistant checkpoint in both
            // native and protocol tool-call modes. Keep non-native user-visible
            // progress separate so only replacement context receives the summary
            // heading.
            let mut assistant_slices = Vec::new();
            if deferred_compact_replay && !parsed.thought.trim().is_empty() {
                assistant_slices.push((
                    "llm_free_talk".to_string(),
                    parsed.thought.trim().to_string(),
                ));
            }
            assistant_slices.extend(
                successful_compact_summaries
                    .into_iter()
                    .map(|summary| ("context_compression_summary".to_string(), summary)),
            );
            slices.splice(
                compact_result_slice_start..compact_result_slice_start,
                assistant_slices,
            );
        } else if deferred_compact_replay {
            // Validation or offload failure must not erase the assistant request
            // that produced the runtime error evidence.
            let assistant_slices =
                self.assistant_replay_slices(&raw_model_output, Some(&parsed), None);
            slices.splice(
                compact_result_slice_start..compact_result_slice_start,
                assistant_slices,
            );
        }
        if compacted_successfully {
            // Only the latest runtime confirmation is operationally useful.
            // Keep every assistant-authored compaction summary, but retire the
            // prior CWD/memo confirmation before appending its replacement.
            self.hide_prompt_slices_by_type("context_compressed");
            // The runtime-held memo survives compaction; restate it so the
            // next submission still carries the long-task reminder.
            let memo_line = self
                .active_memo
                .as_ref()
                .map(|memo| format!("\nmemo active: {memo}"))
                .unwrap_or_default();
            slices.push((
                "context_compressed".to_string(),
                format!(
                    "context compressed successfully.\nCWD: {}{memo_line}",
                    self.current_prompt_cwd.display()
                ),
            ));
        }
        if !parsed.context_compresses.is_empty() && !compacted_successfully {
            // context_compress is a barrier: later actions were authored against the
            // pre-compaction response but must not run unless the state rewrite succeeds.
            self.submit_running_job_updates_for_session(&self.current_session_id(), runtime);
            self.append_delta_with_action_output_budget(slices);
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
        if compacted_successfully && !native_calls.is_empty() {
            // The compact call rewrites its own context and is represented by the
            // independently persisted summary. Keep only later native calls for
            // provider replay and tool-result correlation.
            native_calls.retain(|call| call.name != "context_compress");
        }
        if !parsed.continue_work {
            for candidate in &parsed.memory_candidates {
                if self.memory.write(candidate).is_ok() {
                    self.current_stats.tool_calls += 1;
                    self.current_stats.mem_writes += 1;
                }
            }
            let final_text = parsed.final_text();
            // The inline-protocol final answer (status:ALL_FINISHED) must pass
            // the same memo guard as the explicit task_finished tool.
            if let Some(memo) = self.active_memo.clone() {
                if self.memo_finish_guard_tokens > 0 && self.remaining_rounds() > 0 {
                    self.memo_finish_guard_tokens -= 1;
                    runtime.on_core_topic_events(&[crate::host::memo_stops_finish_topic_event(
                        self.current_session_id().to_string(),
                        &memo,
                    )]);
                    slices.extend(self.assistant_replay_slices(
                        &raw_model_output,
                        Some(&parsed),
                        Some(&final_text),
                    ));
                    slices.push((
                        "memo_finish_guard".to_string(),
                        memo_finish_guard_reminder(&memo),
                    ));
                    self.append_delta_with_action_output_budget(slices);
                    return CoreStep::NeedModel {
                        prompt: self.render_prompt(),
                        rounds_remaining: self.remaining_rounds(),
                    };
                }
            }
            // Guard budget exhausted (or rounds ran out) with an active memo:
            // close the memo on runtime authority before finishing.
            if self.active_memo.is_some() {
                self.force_close_memo_on_finish(runtime);
            }
            slices.extend(self.assistant_replay_slices(
                &raw_model_output,
                Some(&parsed),
                Some(&final_text),
            ));
            // Keep native exchanges structured across turn boundaries. Their
            // delta_id places them on the same append-only context timeline, so
            // provider projection stays byte-stable and cacheable.
            self.defer_next_turn_slices(slices);
            return CoreStep::Final(TurnFinal {
                final_answer: final_text,
                toolgen_retrospect: parsed.toolgen_retrospect.clone(),
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
            let parsed_action_groups = parsed.action_groups.clone();
            let result_lines = match self.execute_action_groups(parsed_action_groups, runtime) {
                Ok(result_lines) => result_lines,
                Err((result_lines, pending)) => {
                    if !native_calls.is_empty() {
                        let delta_id = self.append_native_interaction_delta(slices);
                        self.pending_native_exchange = Some((
                            delta_id,
                            response.content.clone(),
                            native_calls,
                            result_lines,
                        ));
                    } else {
                        slices.extend(
                            result_lines
                                .into_iter()
                                .map(|result| ("result_of_llm_action".to_string(), result)),
                        );
                        self.append_delta_with_action_output_budget(slices);
                    }
                    let request = pending.request.clone();
                    self.pending_approval = Some(pending);
                    return CoreStep::NeedsUserApproval { request };
                }
            };
            if native_calls.is_empty() {
                slices.extend(
                    result_lines
                        .iter()
                        .cloned()
                        .map(|result| ("result_of_llm_action".to_string(), result)),
                );
            }
            if let Some(stop_summary) = self.take_turn_finished_summary() {
                if let Some(memo) = self.active_memo.clone() {
                    if self.memo_finish_guard_tokens > 0 && self.remaining_rounds() > 0 {
                        // A still-active memo contradicts "all work done".
                        // Interception consumes one guard token; genuine work
                        // rounds recharge it, so only consecutive finish
                        // attempts can exhaust the budget and end the turn.
                        self.memo_finish_guard_tokens -= 1;
                        runtime.on_core_topic_events(&[
                            crate::host::memo_stops_finish_topic_event(
                                self.current_session_id().to_string(),
                                &memo,
                            ),
                        ]);
                        slices.push((
                            "memo_finish_guard".to_string(),
                            memo_finish_guard_reminder(&memo),
                        ));
                        if !native_calls.is_empty() {
                            let exchange_results = native_calls
                                .iter()
                                .zip(result_lines.iter())
                                .map(|(call, result)| NativeToolResult {
                                    call_id: call.id.clone(),
                                    name: call.name.clone(),
                                    content: result.clone(),
                                    is_error: Self::action_result_is_error(result),
                                })
                                .collect();
                            let delta_id = self.append_native_interaction_delta(slices);
                            self.register_native_exchange(NativeExchange {
                                delta_id,
                                assistant_text: response.content.clone(),
                                results: exchange_results,
                                calls: native_calls,
                            });
                        } else {
                            self.append_delta_with_action_output_budget(slices);
                        }
                        return CoreStep::NeedModel {
                            prompt: self.render_prompt(),
                            rounds_remaining: self.remaining_rounds(),
                        };
                    }
                }
                // The guard budget is exhausted (or rounds ran out) while a
                // memo is still active: close the memo on runtime authority so
                // the finished turn leaves no stale long-task state.
                if self.active_memo.is_some() {
                    self.force_close_memo_on_finish(runtime);
                }
                // task_finished was executed among the actions above. Its native
                // tool exchange is still recorded below so the provider message
                // sequence stays valid; the turn ends here regardless.
                slices.extend(self.assistant_replay_slices(
                    &raw_model_output,
                    Some(&parsed),
                    Some(&stop_summary),
                ));
                if !native_calls.is_empty() {
                    let exchange_results = native_calls
                        .iter()
                        .zip(result_lines.iter())
                        .map(|(call, result)| NativeToolResult {
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            content: result.clone(),
                            is_error: Self::action_result_is_error(result),
                        })
                        .collect();
                    let delta_id = self.append_native_interaction_delta(slices);
                    self.register_native_exchange(NativeExchange {
                        delta_id,
                        assistant_text: response.content.clone(),
                        results: exchange_results,
                        calls: native_calls,
                    });
                } else {
                    self.defer_next_turn_slices(slices);
                }
                let stats = self.current_stats.clone();
                return CoreStep::Final(TurnFinal {
                    final_answer: stop_summary.clone(),
                    toolgen_retrospect: parsed.toolgen_retrospect,
                    stats: stats.clone(),
                    profile_label: self.profile.label(),
                    repair_issue: None,
                    stop_summary: Some(TurnStopSummary::turn_finished(stop_summary, stats)),
                });
            }
            let native_exchange = if native_calls.is_empty() {
                None
            } else {
                Some(NativeExchange {
                    delta_id: String::new(),
                    assistant_text: response.content.clone(),
                    results: native_calls
                        .iter()
                        .zip(result_lines.iter())
                        .map(|(call, result)| NativeToolResult {
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            content: result.clone(),
                            is_error: Self::action_result_is_error(result),
                        })
                        .collect(),
                    calls: native_calls,
                })
            };
            self.recharge_memo_guard_tokens();
            self.submit_running_job_updates_for_session(&self.current_session_id(), runtime);
            if let Some(mut exchange) = native_exchange {
                exchange.delta_id = self.append_native_interaction_delta(slices);
                self.register_native_exchange(exchange);
            } else {
                self.append_delta_with_action_output_budget(slices);
            }
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
        if !parsed.context_compresses.is_empty() {
            // context_compress is an intrinsic state rewrite. In native mode, do not
            // retain its tool exchange: its owning delta may be removed by the same
            // operation. The independently persisted summary and runtime confirmation
            // below are the canonical continuation context.
            self.submit_running_job_updates_for_session(&self.current_session_id(), runtime);
            self.append_delta_with_action_output_budget(slices);
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
        if self.resolved_tool_call_mode == ToolCallMode::Native {
            // Native mode: a plain-text response without tool calls no longer
            // finishes the turn. Keep the text as visible thought and ask the
            // model to either continue working or call task_finished.
            slices.extend(self.assistant_replay_slices(
                &raw_model_output,
                Some(&parsed),
                Some(&response.content),
            ));
            slices.push((
                "runtime_note".to_string(),
                "A plain-text response without tool calls does not finish the turn. Continue working with tool calls, or call the task_finished tool with the complete final answer when all work is done.".to_string(),
            ));
            self.append_delta_with_action_output_budget(slices);
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
        slices.extend(self.assistant_replay_slices(
            &raw_model_output,
            Some(&parsed),
            Some(&final_text),
        ));
        self.defer_next_turn_slices(slices);
        CoreStep::Final(TurnFinal {
            final_answer: final_text,
            toolgen_retrospect: parsed.toolgen_retrospect,
            stats: self.current_stats.clone(),
            profile_label: self.profile.label(),
            repair_issue: None,
            stop_summary: None,
        })
    }

    pub fn record_unapplied_model_response_usage(&mut self, usage: &UsageStats) {
        self.current_round += 1;
        self.current_stats.add(usage);
        self.last_observed_prompt_tokens =
            self.last_observed_prompt_tokens.max(usage.prompt_tokens);
    }

    fn parse_native_response(&self, response: &LlmResponse) -> ParsedEnvelope {
        let actions = response
            .tool_calls
            .iter()
            .map(|call| ParsedAction {
                action: call.name.clone(),
                name: None,
                call_id: call.id.clone(),
                raw_input: call.arguments.clone(),
            })
            .collect::<Vec<_>>();
        let action_groups = if actions.is_empty() {
            Vec::new()
        } else {
            vec![ParsedActionGroup {
                order: if self.native_parallel_tool_calls && actions.len() > 1 {
                    ActionGroupOrder::Parallel
                } else {
                    ActionGroupOrder::Sequential
                },
                actions,
            }]
        };
        ParsedEnvelope {
            // A no-tool-call response no longer finishes the turn: only the
            // explicit task_finished tool does. Plain text is retained as
            // thought so the loop continues.
            final_answer: String::new(),
            toolgen_retrospect: String::new(),
            continue_work: true,
            thought: response.content.clone(),
            thought_keep_in_context: !response.content.trim().is_empty(),
            next_actions: Vec::new(),
            action_groups,
            context_compresses: Vec::new(),
            memory_candidates: Vec::new(),
            accepted_response: None,
            runtime_note: None,
            recovered_issue: None,
            repair_issue: None,
        }
    }

    fn current_native_delta_id(&self) -> String {
        self.deltas
            .last()
            .map(|delta| delta.delta_id.clone())
            .unwrap_or_else(|| "pd_0".to_string())
    }

    fn append_native_interaction_delta(&mut self, slices: Vec<(String, String)>) -> String {
        // One native model interaction is one transport batch. When the batch has
        // no textual components, retain an empty PromptDelta so its visible delta
        // boundary can own the structured assistant/tool exchange and remain
        // independently addressable by context compression.
        let delta_count_before = self.deltas.len();
        self.append_delta_with_action_output_budget(slices);
        if self.deltas.len() == delta_count_before {
            let time_ms = now_ms();
            let delta_sequence = self.next_delta_sequence;
            self.next_delta_sequence = self.next_delta_sequence.saturating_add(1);
            self.deltas.push(PromptDelta {
                delta_id: format!("pd_{delta_sequence}"),
                time_ms,
                slices: Vec::new(),
                hidden_slice_ids: Vec::new(),
            });
        }
        self.current_native_delta_id()
    }

    fn materialize_native_exchanges(&mut self) {
        let exchanges = std::mem::take(&mut self.native_exchanges);
        for exchange in exchanges {
            let mut assistant = exchange.assistant_text.trim().to_string();
            if !exchange.calls.is_empty() {
                let calls = exchange
                    .calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                        })
                    })
                    .collect::<Vec<_>>();
                if !assistant.is_empty() {
                    assistant.push('\n');
                }
                assistant.push_str("Tool calls:\n");
                assistant.push_str(&serde_json::to_string(&calls).unwrap_or_default());
            }
            self.submit_prompt_component(
                PromptComponentRole::assistant(self.assistant_speaker_name.clone()),
                "llm_response",
                assistant,
                "native_interaction",
            );
            for result in exchange.results {
                self.submit_prompt_component(
                    PromptComponentRole::system(),
                    "result_of_llm_action",
                    result.content,
                    "native_interaction",
                );
            }
        }
    }

    fn complete_pending_native_exchange(&mut self, new_results: Vec<String>) -> bool {
        let Some((delta_id, assistant_text, calls, mut results)) =
            self.pending_native_exchange.take()
        else {
            return false;
        };
        results.extend(new_results);
        let tool_results = calls
            .iter()
            .enumerate()
            .map(|(index, call)| {
                let content = results.get(index).cloned().unwrap_or_else(|| {
                    format!(
                        "Action result: {}\nerror: not_executed_after_approval_boundary",
                        call.name
                    )
                });
                NativeToolResult {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    is_error: Self::action_result_is_error(&content),
                    content,
                }
            })
            .collect();
        self.register_native_exchange(NativeExchange {
            delta_id,
            assistant_text,
            calls,
            results: tool_results,
        });
        true
    }

    fn normalize_intrinsic_actions(&self, parsed: &mut ParsedEnvelope) {
        if parsed.repair_issue.is_some() {
            return;
        }
        let compact_positions = parsed
            .action_groups
            .iter()
            .enumerate()
            .flat_map(|(group_index, group)| {
                group
                    .actions
                    .iter()
                    .enumerate()
                    .filter(|(_, action)| action.action == "context_compress")
                    .map(move |(action_index, _)| (group_index, action_index))
            })
            .collect::<Vec<_>>();
        if compact_positions.is_empty() {
            return;
        }
        if compact_positions.len() != 1 || !parsed.context_compresses.is_empty() {
            parsed.repair_issue = Some("context_compress_only_once".to_string());
            return;
        }
        let (group_index, action_index) = compact_positions[0];
        if group_index != 0 || action_index != 0 {
            parsed.repair_issue = Some("context_compress_must_be_first".to_string());
            return;
        }
        let action = parsed.action_groups[0].actions.remove(0);
        if parsed.action_groups[0].actions.is_empty() {
            parsed.action_groups.remove(0);
        }
        if let Err(issue) = self
            .capabilities
            .validate_action_input(&action.action, &action.raw_input)
        {
            parsed.repair_issue = Some(format!("context_compress.{issue}"));
            return;
        }
        match context_compress::from_action(&action) {
            Ok(compact) => {
                parsed.context_compresses.push(compact);
                parsed.continue_work = true;
            }
            Err(issue) => parsed.repair_issue = Some(issue),
        }
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
        let response_content = response.content.clone();
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
            &response_content,
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
        let response_content = response.content.clone();
        let step = self.apply_model_response_with_action_runtime(response, runtime);
        self.record_model_repair_audit_if_needed(
            audit_file,
            session,
            turn_id,
            repair_calls_before,
            &response_model,
            &response_usage,
            response_truncated,
            &response_content,
        );
        step
    }

    #[allow(clippy::too_many_arguments)]
    fn record_model_repair_audit_if_needed(
        &self,
        audit_file: &Path,
        session: &str,
        turn_id: &str,
        repair_calls_before: u32,
        response_model: &str,
        response_usage: &UsageStats,
        response_truncated: bool,
        raw_response: &str,
    ) {
        let repair_calls_after = self.current_stats().repair_calls;
        if repair_calls_after > repair_calls_before {
            let issue = self.last_repair_issue();
            let _ = append_audit_event(
                audit_file,
                &model_repair_request_audit_event(
                    session,
                    turn_id,
                    issue,
                    response_model,
                    response_usage,
                    response_truncated,
                    repair_calls_after,
                    repair_calls_after.saturating_sub(repair_calls_before),
                ),
            );
            let instruction = issue
                .map(|issue| {
                    self.response_protocol
                        .suite()
                        .repair_instruction_for_response(issue, raw_response)
                })
                .unwrap_or_else(|| {
                    "Please resend the response using the required protocol format.".to_string()
                });
            let system_message = format!(
                "{}'s previous response is not protocol compliant.\nerror: {}\n\n{}",
                self.assistant_speaker_name,
                issue.unwrap_or("unknown_repair_issue"),
                instruction
            );
            let _ = append_repair_output_event(
                audit_file,
                &model_repair_output_event(
                    session,
                    turn_id,
                    issue,
                    &self.assistant_speaker_name,
                    raw_response,
                    &system_message,
                    response_model,
                    response_usage,
                    response_truncated,
                    repair_calls_after,
                    repair_calls_after.saturating_sub(repair_calls_before),
                    self.response_protocol.suite().prompt_boundaries(),
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
            self.append_delta_with_action_output_budget(vec![(
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
        if let Some(continuation) = pending.continuation.clone() {
            return self.resolve_parallel_group_approval_with_runtime(
                pending,
                approved,
                continuation,
                runtime,
            );
        }
        let action_cpu_start = thread_cpu_time();
        let approved_action = ParsedAction {
            action: pending.request.action.clone(),
            name: pending.action_name.clone(),
            call_id: pending.action_call_id.clone(),
            raw_input: pending.approved_action.audit_input(
                &pending.request.approval_id,
                &pending.request.risk,
                &pending.request.reason,
            ),
        };
        if approved
            && matches!(
                pending.approved_action,
                PendingApprovedAction::RunBash { .. }
            )
        {
            self.emit_action_execution_start_topic(&approved_action, runtime);
        }
        let outcome = if approved {
            match &pending.approved_action {
                PendingApprovedAction::RunBash {
                    command,
                    background,
                    timeout_ms,
                    interval_ms,
                    once_timeout_ms,
                    session_id,
                    turn_id,
                    tool_call_id,
                    cwd,
                    tail_out,
                    edited_files,
                } => shell_exec::execute_approved_bash_with_tail(
                    command,
                    cwd,
                    *background,
                    *timeout_ms,
                    *interval_ms,
                    *once_timeout_ms,
                    session_id,
                    turn_id,
                    tool_call_id,
                    interval_ms.is_none(),
                    *tail_out,
                    edited_files,
                    &pending.request,
                    &self.shell_jobs,
                    runtime,
                ),
                PendingApprovedAction::ToolgenPublish { repo, draft_path } => {
                    toolgen::execute_approved_publish_outcome(repo, draft_path, &pending.request)
                }
            }
        } else {
            self.denied_approval_outcome(&pending)
        };
        self.record_pending_approval_audit(&pending, approved, &outcome.text);
        self.emit_action_finish_topic(
            &approved_action,
            &outcome,
            match &pending.approved_action {
                PendingApprovedAction::RunBash { .. } => None,
                PendingApprovedAction::ToolgenPublish { .. } => {
                    elapsed_thread_cpu(action_cpu_start)
                }
            },
            runtime,
        );
        let prompt_result = self.format_pending_action_result(&pending, &outcome, runtime);
        if !self.complete_pending_native_exchange(vec![prompt_result.clone()]) {
            self.append_delta_with_action_output_budget(vec![(
                "result_of_llm_action".to_string(),
                prompt_result,
            )]);
        }
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

    fn denied_approval_outcome(&self, pending: &PendingApproval) -> ActionOutcome {
        ActionOutcome::failed("approval denied by user")
            .with_runtime_metadata("approval_status", "denied_by_user")
            .with_runtime_metadata("approval_id", pending.request.approval_id.clone())
            .with_runtime_metadata("approval_reason", pending.request.reason.clone())
    }

    #[allow(clippy::result_large_err)]
    fn finish_parallel_group_after_approvals(
        &mut self,
        actions: Vec<ParsedAction>,
        approved: Vec<(usize, PendingApproval)>,
        denied_results: Vec<(usize, String)>,
        completed_results: Vec<(usize, String)>,
        runtime: &mut dyn ActionRuntime,
    ) -> Result<Vec<String>, (Vec<String>, PendingApproval)> {
        let mut results = vec![None; actions.len()];
        let cancel_requested = Arc::new(AtomicBool::new(false));
        for (idx, result) in denied_results.into_iter().chain(completed_results) {
            if let Some(slot) = results.get_mut(idx) {
                *slot = Some(result);
            }
        }

        let mut approved_bash_handles = Vec::new();
        for (idx, pending) in approved {
            let action = ParsedAction {
                action: pending.request.action.clone(),
                name: pending.action_name.clone(),
                call_id: pending.action_call_id.clone(),
                raw_input: pending.approved_action.audit_input(
                    &pending.request.approval_id,
                    &pending.request.risk,
                    &pending.request.reason,
                ),
            };
            self.emit_action_execution_start_topic(&action, runtime);
            approved_bash_handles.push(self.spawn_approved_parallel_bash_action(
                idx,
                pending,
                Arc::clone(&cancel_requested),
            ));
        }

        let mut action_handles = Vec::new();
        for (idx, action) in actions.iter().cloned().enumerate() {
            if results.get(idx).is_some_and(Option::is_some)
                || shell_exec::is_local_shell_action(&action.action)
            {
                continue;
            }
            if self.can_spawn_parallel_readfile_action(&action) {
                self.emit_action_execution_start_topic(&action, runtime);
                action_handles.push(self.spawn_parallel_readfile_action(idx, action));
                continue;
            }
            match self.execute_action(action.clone(), runtime) {
                ActionExecution::Completed(outcome) => {
                    if let Some(slot) = results.get_mut(idx) {
                        *slot = Some(
                            self.format_action_outcome_with_runtime(&action, &outcome, runtime),
                        );
                    }
                }
                ActionExecution::NeedsApproval(pending) => {
                    self.collect_parallel_action_handles(
                        action_handles,
                        &mut results,
                        runtime,
                        &cancel_requested,
                    );
                    self.collect_approved_parallel_bash_handles(
                        approved_bash_handles,
                        &mut results,
                        runtime,
                        &cancel_requested,
                    );
                    return Err((Self::ordered_parallel_results(results), pending));
                }
            }
        }

        self.collect_parallel_action_handles(
            action_handles,
            &mut results,
            runtime,
            &cancel_requested,
        );
        self.collect_approved_parallel_bash_handles(
            approved_bash_handles,
            &mut results,
            runtime,
            &cancel_requested,
        );
        Ok(Self::ordered_parallel_results(results))
    }

    fn resolve_parallel_group_approval_with_runtime(
        &mut self,
        mut pending: PendingApproval,
        approved_by_user: bool,
        continuation: PendingApprovalContinuation,
        runtime: &mut dyn ActionRuntime,
    ) -> CoreStep {
        let PendingApprovalContinuation::ParallelGroup {
            actions,
            current_index,
            mut approved,
            mut denied_results,
            mut completed_results,
        } = continuation;

        pending.continuation = None;
        if approved_by_user {
            approved.push((current_index, pending));
            // If the host signaled "always allow", update bash approval mode
            // before processing subsequent actions in this parallel group.
            if runtime.take_bash_always_allow() {
                self.bash_approval_mode = BashApprovalMode::Approve;
            }
        } else {
            let outcome = self.denied_approval_outcome(&pending);
            self.record_pending_approval_audit(&pending, false, &outcome.text);
            let prompt_result = self.format_pending_action_result(&pending, &outcome, runtime);
            denied_results.push((current_index, prompt_result));
        }

        for next_index in (current_index + 1)..actions.len() {
            let action = actions[next_index].clone();
            if !shell_exec::is_local_shell_action(&action.action) {
                continue;
            }
            match self.execute_action(action.clone(), runtime) {
                ActionExecution::Completed(outcome) => {
                    completed_results.push((
                        next_index,
                        self.format_action_outcome_with_runtime(&action, &outcome, runtime),
                    ));
                }
                ActionExecution::NeedsApproval(next_pending) => {
                    let pending = Self::pending_approval_with_parallel_continuation(
                        next_pending,
                        actions,
                        next_index,
                        approved,
                        denied_results,
                        completed_results,
                    );
                    let request = pending.request.clone();
                    self.pending_approval = Some(pending);
                    return CoreStep::NeedsUserApproval { request };
                }
            }
        }

        let result_lines = match self.finish_parallel_group_after_approvals(
            actions,
            approved,
            denied_results,
            completed_results,
            runtime,
        ) {
            Ok(results) => results,
            Err((partial, pending)) => {
                self.pending_approval = Some(pending.clone());
                if let Some((_, _, _, results)) = self.pending_native_exchange.as_mut() {
                    results.extend(partial);
                } else {
                    self.append_delta_with_action_output_budget(
                        partial
                            .into_iter()
                            .map(|result| ("result_of_llm_action".to_string(), result))
                            .collect(),
                    );
                }
                return CoreStep::NeedsUserApproval {
                    request: pending.request,
                };
            }
        };

        if !self.complete_pending_native_exchange(result_lines.clone()) {
            self.append_delta_with_action_output_budget(
                result_lines
                    .into_iter()
                    .map(|result| ("result_of_llm_action".to_string(), result))
                    .collect(),
            );
        }
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

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_output_expansion_with_audit(
        &self,
        config: &mut ModelServiceConfig,
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
        self.render_prompt_from_deltas(&self.deltas)
    }

    pub fn submit_prompt_component(
        &mut self,
        role: PromptComponentRole,
        kind: impl Into<String>,
        content: impl Into<String>,
        source: impl Into<String>,
    ) -> Option<String> {
        let logical_time_ms = now_ms();
        self.submit_prompt_component_at(role, kind, content, source, logical_time_ms)
    }

    /// A model response that keeps working (no finish attempt) proves the
    /// turn made real progress; recharge one memo-finish-guard token so an
    /// earlier interception cannot be "spent" by stale history.
    fn recharge_memo_guard_tokens(&mut self) {
        self.memo_finish_guard_tokens = self
            .memo_finish_guard_tokens
            .saturating_add(1)
            .min(MEMO_FINISH_GUARD_TOKEN_CAP);
    }

    /// After a memo is deleted mid-turn, the next request carries a one-shot
    /// re-verify trailer; no per-round memo restate is attached otherwise.
    fn take_memo_deleted_trailer(&mut self) -> Option<String> {
        let deleted = self.memo_deleted_this_turn.clone()?;
        if self.memo_deleted_trailer_shown {
            return None;
        }
        self.memo_deleted_trailer_shown = true;
        Some(format!(
            "\n\nYou just deleted the memo: \"{deleted}\" Re-verify: is the memo's final goal (not an intermediate milestone) genuinely achieved, and is this final answer a complete delivery for the user? If there is still remained work, you should update the memo and continue, and don't issue task_finished tool unless user asks you to."
        ))
    }

    const PROCESS_AGGREGATE_SCOPES_MARKER: &'static str = "PROCESS_AGGREGATE_SCOPES:";

    fn current_process_aggregate_scope_snapshot(
        &self,
        session_id: &str,
    ) -> Option<os::ProcessAggregateScopeSnapshot> {
        #[cfg(test)]
        if let Some(snapshot) = self.process_scope_snapshot_override.clone() {
            return Some(snapshot);
        }
        os::process_aggregate_scope_snapshot(session_id)
            .ok()
            .flatten()
    }

    fn take_process_aggregate_scopes_if_needed(&mut self) -> Option<String> {
        let session_id = self.current_session_id.clone()?;
        if self.process_scope_prompted_session.as_deref() == Some(session_id.as_str()) {
            return None;
        }
        // A restored Context can contain a path from a previous Runtime. Hide
        // it before publishing the current Runtime identity. If cgroup
        // delegation is unavailable, do not leave stale scope claims visible.
        self.hide_prompt_slices_matching(Self::PROCESS_AGGREGATE_SCOPES_MARKER);
        let snapshot = self.current_process_aggregate_scope_snapshot(&session_id)?;
        self.process_scope_prompted_session = Some(session_id);
        Some(format!(
            "{}\n- Runtime process scope: {}\n- Current Session process scope: {}\nThese are aggregate observation directories; exact process ownership and cancellation use their per-Job child scopes. Inspect standard cgroup files there when resource or process diagnosis is needed.",
            Self::PROCESS_AGGREGATE_SCOPES_MARKER,
            snapshot.runtime_observation_note,
            snapshot.session_observation_note,
        ))
    }

    fn submit_process_aggregate_scopes_if_needed(&mut self) {
        let Some(note) = self.take_process_aggregate_scopes_if_needed() else {
            return;
        };
        self.submit_prompt_component(
            PromptComponentRole::system(),
            "runtime_note",
            note,
            "runtime_process_scope",
        );
    }

    pub fn build_next_prompt(&mut self) -> String {
        self.submit_process_aggregate_scopes_if_needed();
        if self.runtime_config_changed_notice_pending {
            self.runtime_config_changed_notice_pending = false;
            self.submit_prompt_component(
                PromptComponentRole::system(),
                "runtime_config_changed",
                RUNTIME_CONFIG_CHANGED_NOTICE,
                "runtime_config",
            );
        }
        self.guard_pending_action_output_budget();
        self.flush_pending_prompt_components();
        let rendered = self.render_prompt();
        let mut prompt = rendered.clone();
        if let Some(trailer) = self.take_memo_deleted_trailer() {
            let (body, response_trailer) =
                prompt_render::split_formatted_response_trailer(&rendered);
            prompt = body.trim_end().to_string();
            prompt.push_str(&trailer);
            if let Some(response_trailer) = response_trailer {
                prompt.push_str("\n\n");
                prompt.push_str(&response_trailer);
            }
        }
        if self.context_compress_required {
            let (body, response_trailer) = prompt_render::split_formatted_response_trailer(&prompt);
            // The manual wording persists across retries (like the
            // threshold wording) until the compaction succeeds: the context
            // may not actually be over the limit, so the retry must not fall
            // back to "Context is too long".
            let compact_trailer = if self.manual_compact_trailer_pending {
                prompt_render::MANUAL_CONTEXT_COMPRESS_TRAILER
            } else {
                prompt_render::CONTEXT_COMPRESS_REQUIRED_TRAILER
            };
            prompt = body.trim_end().to_string();
            prompt.push_str("\n\n");
            prompt.push_str(compact_trailer);
            if let Some(response_trailer) = response_trailer {
                let _ = response_trailer;
            }
        }
        prompt
    }

    fn guard_pending_action_output_budget(&mut self) -> bool {
        let action_output_bytes = self
            .pending_prompt_components
            .iter()
            .filter(|component| component.prompt_type() == "result_of_llm_action")
            .map(|component| component.content.len())
            .fold(0usize, usize::saturating_add);
        if action_output_bytes == 0 {
            return false;
        }
        let pending_tokens = self
            .pending_prompt_components
            .iter()
            .map(|component| {
                if component.prompt_type() == "result_of_llm_action" {
                    estimate_action_output_tokens(&component.content)
                } else {
                    estimate_prompt_tokens(&component.content)
                }
            })
            .fold(0u32, u32::saturating_add);
        let base_tokens = self.current_prompt_token_baseline();
        let projected_tokens = base_tokens
            .saturating_add(pending_tokens)
            .saturating_add(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS);
        let safety_limit = self
            .max_llm_input_tokens
            .saturating_mul(ACTION_OUTPUT_CONTEXT_SAFETY_PERCENT)
            / 100;
        if projected_tokens <= safety_limit {
            return false;
        }

        self.pending_prompt_components
            .retain(|component| component.prompt_type() != "result_of_llm_action");
        let retained_pending_tokens = self
            .pending_prompt_components
            .iter()
            .map(|component| estimate_prompt_tokens(&component.content))
            .fold(0u32, u32::saturating_add);
        let remaining_tokens = self.max_llm_input_tokens.saturating_sub(
            base_tokens
                .saturating_add(retained_pending_tokens)
                .saturating_add(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS),
        );
        self.submit_prompt_component(
            PromptComponentRole::system(),
            "runtime_note",
            action_output_too_large_note(action_output_bytes, remaining_tokens),
            "runtime_context_budget",
        );
        true
    }

    fn render_prompt_slices(&self) -> Vec<PromptSlice> {
        prompt_render::render_prompt_slices(&self.deltas)
    }
    fn remaining_rounds(&self) -> u32 {
        if self.round_budget == UNLIMITED_ROUND_BUDGET {
            UNLIMITED_ROUND_BUDGET
        } else {
            self.round_budget.saturating_sub(self.current_round)
        }
    }

    fn request_protocol_repair(
        &mut self,
        issue: &str,
        instruction: &str,
        raw_response: &str,
        runtime: &mut dyn ActionRuntime,
    ) -> CoreStep {
        self.repair_attempted = true;
        self.repair_attempts = self.repair_attempts.saturating_add(1);
        self.last_repair_issue = Some(issue.to_string());
        self.current_stats.repair_calls = self.current_stats.repair_calls.saturating_add(1);
        let repair_reason = self
            .response_protocol
            .suite()
            .repair_reason(issue)
            .to_string();
        runtime.on_core_topic_events(&[host::model_repair_topic_event(
            self.current_session_id(),
            issue,
            repair_reason,
            self.repair_attempts,
            MAX_PROTOCOL_REPAIR_ATTEMPTS,
        )]);
        let focused_response = self
            .response_protocol
            .suite()
            .focused_repair_text(issue, raw_response);
        let repair_note = format!(
            "{}'s previous response is not protocol compliant.\nerror: {}\n\n{}",
            self.assistant_speaker_name, issue, instruction
        );
        let prompt = self.render_prompt_with_temporary_delta(vec![
            ("llm_response".to_string(), focused_response),
            ("response_repair".to_string(), repair_note),
        ]);
        CoreStep::NeedModel {
            prompt,
            rounds_remaining: self.remaining_rounds(),
        }
    }

    fn render_prompt_with_temporary_delta(&self, slice_texts: Vec<(String, String)>) -> String {
        let time_ms = now_ms();
        let delta_id = format!("temp_repair_{}_{}", time_ms, self.repair_attempts);
        let chunks = slice_texts
            .into_iter()
            .flat_map(|(prompt_type, text)| {
                split_prompt_component_text(&prompt_type, &text, PROMPT_SLICE_TEXT_LIMIT)
                    .into_iter()
                    .map(move |chunk| (prompt_type.clone(), chunk))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let slice_count = chunks.len();
        let slices = chunks
            .into_iter()
            .enumerate()
            .map(|(idx, (prompt_type, text))| {
                let slice_index = idx + 1;
                PromptSlice {
                    delta_id: delta_id.clone(),
                    slice_id: format!(
                        "ps_{}_s{:03}",
                        delta_id.trim_start_matches("pd_"),
                        slice_index
                    ),
                    component_id: format!("temp_component_{slice_index}"),
                    prompt_type,
                    time_ms,
                    text,
                    slice_index,
                    slice_count,
                }
            })
            .collect::<Vec<_>>();
        let mut deltas = self.deltas.clone();
        deltas.push(PromptDelta {
            delta_id,
            time_ms,
            slices,
            hidden_slice_ids: Vec::new(),
        });
        self.render_prompt_from_deltas(&deltas)
    }

    fn append_in_turn_shrink_review_if_needed(&mut self) {
        self.require_context_compress_if_needed(0);
    }

    fn inline_tool_call_labels(&self, parsed: &ParsedEnvelope) -> Vec<(String, String)> {
        if self.resolved_tool_call_mode == ToolCallMode::Native {
            return Vec::new();
        }
        parsed
            .action_groups
            .iter()
            .flat_map(|group| group.actions.iter())
            .map(|action| (action.call_id.clone(), action.action.clone()))
            .collect()
    }

    fn append_inline_tool_call_labels(&self, replay: &mut String, parsed: &ParsedEnvelope) {
        let calls = self.inline_tool_call_labels(parsed);
        if calls.is_empty() {
            return;
        }
        replay.push_str(
            "
Runtime tool_call ids:",
        );
        for (call_id, action) in calls {
            replay.push_str(&format!(
                "
- {call_id}: {action}"
            ));
        }
    }

    fn assistant_replay_slices(
        &self,
        raw_response: &str,
        parsed: Option<&ParsedEnvelope>,
        final_text: Option<&str>,
    ) -> Vec<(String, String)> {
        match self.assistant_replay_mode {
            AssistantReplayMode::RawOutput => {
                let accepted_response =
                    parsed.and_then(|parsed| parsed.accepted_response.as_deref());
                let mut replay = accepted_response.unwrap_or(raw_response).trim().to_string();
                if let Some(parsed) = parsed {
                    self.append_inline_tool_call_labels(&mut replay, parsed);
                }
                if replay.is_empty() {
                    Vec::new()
                } else {
                    let prompt_type = if accepted_response.is_some()
                        && self.response_protocol == ResponseProtocolKind::Xml
                    {
                        "llm_response_raw_xml"
                    } else {
                        "llm_response"
                    };
                    vec![(prompt_type.to_string(), replay)]
                }
            }
            AssistantReplayMode::ExtractedFields => {
                if let Some(accepted_response) =
                    parsed.and_then(|parsed| parsed.accepted_response.as_deref())
                {
                    let mut replay = accepted_response.trim().to_string();
                    if let Some(parsed) = parsed {
                        self.append_inline_tool_call_labels(&mut replay, parsed);
                    }
                    if !replay.is_empty() {
                        let prompt_type = if self.response_protocol == ResponseProtocolKind::Xml {
                            "llm_response_raw_xml"
                        } else {
                            "llm_response"
                        };
                        return vec![(prompt_type.to_string(), replay)];
                    }
                }

                let mut slices = Vec::new();
                if let Some(parsed) = parsed {
                    if !parsed.thought.is_empty() {
                        slices.push(("llm_free_talk".to_string(), parsed.thought.to_string()));
                    }
                    for compact in &parsed.context_compresses {
                        if !compact.summary.trim().is_empty() {
                            slices.push((
                                "llm_response".to_string(),
                                compact.summary.trim().to_string(),
                            ));
                        }
                    }
                }
                if let Some(final_text) = final_text {
                    if !final_text.trim().is_empty() {
                        slices.push((
                            "llm_response".to_string(),
                            llm_final_answer_slice_text(final_text),
                        ));
                    }
                }
                slices
            }
        }
    }

    fn submit_prompt_component_at(
        &mut self,
        role: PromptComponentRole,
        kind: impl Into<String>,
        content: impl Into<String>,
        source: impl Into<String>,
        logical_time_ms: i64,
    ) -> Option<String> {
        let kind = kind.into();
        let mut content = content.into();
        if role.prompt_type_hint(&kind) == "result_of_llm_action"
            && !prompt_render::is_structured_action_result_envelope(&content)
        {
            // Defensive ingress for legacy/internal producers that do not originate
            // from a typed action. Structured action envelopes have already applied
            // their per-call model budget and must remain valid JSON end to end.
            content = tool_result_gate::fit(
                &content,
                self.model_tool_result_bytes,
                tool_result_gate::Retention::Head,
            );
        }
        // Explicit resume is a header-only user behavior, not synthetic text.
        if content.trim().is_empty()
            && !(role == PromptComponentRole::User && kind == "user_resume_directly")
        {
            return None;
        }
        self.prompt_component_sequence = self.prompt_component_sequence.saturating_add(1);
        let sequence = self.prompt_component_sequence;
        let batch_id = format!("pcb_{}", logical_time_ms);
        let id = format!("pc_{}_{}", logical_time_ms, sequence);
        self.pending_prompt_components.push(PromptComponent {
            id: id.clone(),
            role,
            kind,
            content,
            source: source.into(),
            created_at_ms: logical_time_ms,
            sequence,
            batch_id,
            cache_policy_hint: None,
        });
        Some(id)
    }

    fn submit_prompt_components_from_slice_texts(
        &mut self,
        slice_texts: Vec<(String, String)>,
        source: &str,
        logical_time_ms: i64,
    ) {
        for (prompt_type, text) in slice_texts {
            let role = role_for_prompt_type(&prompt_type, &self.assistant_speaker_name);
            self.submit_prompt_component_at(role, prompt_type, text, source, logical_time_ms);
        }
    }

    fn append_delta(&mut self, slice_texts: Vec<(String, String)>) {
        let logical_time_ms = now_ms();
        if self.cwd_note_pending {
            self.cwd_note_pending = false;
            self.submit_cwd_note_at(logical_time_ms);
        }
        self.submit_prompt_components_from_slice_texts(slice_texts, "runtime", logical_time_ms);
        self.flush_pending_prompt_components();
    }

    fn append_delta_with_action_output_budget(
        &mut self,
        mut slice_texts: Vec<(String, String)>,
    ) -> bool {
        let action_output_bytes = slice_texts
            .iter()
            .filter(|(prompt_type, _)| prompt_type == "result_of_llm_action")
            .map(|(_, text)| text.len())
            .fold(0usize, usize::saturating_add)
            .saturating_add(
                self.pending_prompt_components
                    .iter()
                    .filter(|component| component.prompt_type() == "result_of_llm_action")
                    .map(|component| component.content.len())
                    .fold(0usize, usize::saturating_add),
            );
        if action_output_bytes == 0 {
            self.append_delta(slice_texts);
            return false;
        }

        let pending_tokens = self
            .pending_prompt_components
            .iter()
            .map(|component| {
                if component.prompt_type() == "result_of_llm_action" {
                    estimate_action_output_tokens(&component.content)
                } else {
                    estimate_prompt_tokens(&component.content)
                }
            })
            .fold(0u32, u32::saturating_add);
        let candidate_tokens = slice_texts
            .iter()
            .map(|(prompt_type, text)| {
                if prompt_type == "result_of_llm_action" {
                    estimate_action_output_tokens(text)
                } else {
                    estimate_prompt_tokens(text)
                }
            })
            .fold(0u32, u32::saturating_add);
        let base_tokens = self.current_prompt_token_baseline();
        let current_tokens = base_tokens.saturating_add(pending_tokens);
        let projected_tokens = current_tokens
            .saturating_add(candidate_tokens)
            .saturating_add(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS);
        let safety_limit = self
            .max_llm_input_tokens
            .saturating_mul(ACTION_OUTPUT_CONTEXT_SAFETY_PERCENT)
            / 100;
        if projected_tokens <= safety_limit {
            self.append_delta(slice_texts);
            return false;
        }

        let retained_pending_tokens = self
            .pending_prompt_components
            .iter()
            .filter(|component| component.prompt_type() != "result_of_llm_action")
            .map(|component| estimate_prompt_tokens(&component.content))
            .fold(0u32, u32::saturating_add);
        let remaining_tokens = self.max_llm_input_tokens.saturating_sub(
            base_tokens
                .saturating_add(retained_pending_tokens)
                .saturating_add(PROMPT_DELTA_RENDER_OVERHEAD_TOKENS),
        );
        self.pending_prompt_components
            .retain(|component| component.prompt_type() != "result_of_llm_action");
        slice_texts.clear();
        slice_texts.push((
            "runtime_note".to_string(),
            action_output_too_large_note(action_output_bytes, remaining_tokens),
        ));
        self.append_delta(slice_texts);
        true
    }

    fn current_prompt_token_baseline(&self) -> u32 {
        if self.last_observed_prompt_tokens > 0 {
            self.last_observed_prompt_tokens
        } else {
            estimate_prompt_tokens(&self.render_prompt())
        }
    }

    pub fn recover_from_model_input_too_large(
        &mut self,
        error: &str,
    ) -> Option<ModelInputOverflowRecovery> {
        let delta_index = self.deltas.len().checked_sub(1)?;
        if !prompt_render::render_delta_slices(&self.deltas[delta_index])
            .iter()
            .any(|slice| slice.prompt_type == "result_of_llm_action")
        {
            return None;
        }
        let removed_output_bytes = self.deltas[delta_index]
            .slices
            .iter()
            .filter(|slice| {
                slice.prompt_type == "result_of_llm_action"
                    && !self.deltas[delta_index]
                        .hidden_slice_ids
                        .contains(&slice.slice_id)
            })
            .map(|slice| slice.text.len())
            .sum::<usize>();
        if removed_output_bytes == 0 {
            return None;
        }

        let removed_delta_id = self.deltas[delta_index].delta_id.clone();
        self.deltas.remove(delta_index);
        self.last_observed_prompt_tokens = 0;
        let current_tokens = estimate_prompt_tokens(&self.render_prompt());
        let remaining_tokens = self.max_llm_input_tokens.saturating_sub(current_tokens);
        self.append_delta(vec![(
            "runtime_note".to_string(),
            format!(
                "{}\nThe previous model request was rejected because its input was too large: {}",
                action_output_too_large_note(removed_output_bytes, remaining_tokens),
                compact_text(error, 500)
            ),
        )]);
        Some(ModelInputOverflowRecovery {
            step: CoreStep::NeedModel {
                prompt: self.render_prompt(),
                rounds_remaining: self.remaining_rounds(),
            },
            removed_delta_id,
            removed_action_output_bytes: removed_output_bytes,
        })
    }

    fn flush_pending_prompt_components(&mut self) {
        if self.pending_prompt_components.is_empty() {
            return;
        }
        let mut components = std::mem::take(&mut self.pending_prompt_components);
        components.sort_by_key(|component| (component.created_at_ms, component.sequence));
        let timestamp = components
            .iter()
            .map(|component| component.created_at_ms)
            .min()
            .unwrap_or_else(now_ms);
        let delta_sequence = self.next_delta_sequence;
        self.next_delta_sequence = self.next_delta_sequence.saturating_add(1);
        let delta_id = format!("pd_{delta_sequence}");
        let chunks = components
            .into_iter()
            .flat_map(|component| {
                let prompt_type = component.prompt_type();
                let component_id = component.id;
                let slice_time_ms = component.created_at_ms;
                split_prompt_component_text(
                    &prompt_type,
                    &component.content,
                    PROMPT_SLICE_TEXT_LIMIT,
                )
                .into_iter()
                .map(move |chunk| {
                    (
                        component_id.clone(),
                        prompt_type.clone(),
                        slice_time_ms,
                        chunk,
                    )
                })
                .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let slice_count = chunks.len();
        let slices = chunks
            .into_iter()
            .enumerate()
            .map(|(idx, (component_id, prompt_type, time_ms, text))| {
                let slice_index = idx + 1;
                PromptSlice {
                    delta_id: delta_id.clone(),
                    slice_id: format!(
                        "ps_{}_s{:03}",
                        delta_id.trim_start_matches("pd_"),
                        slice_index
                    ),
                    component_id,
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
        let logical_time_ms = now_ms();
        self.submit_prompt_components_from_slice_texts(
            slice_texts,
            "previous_model_response",
            logical_time_ms,
        );
    }

    fn require_context_compress_if_needed(&mut self, incoming_prompt_tokens: u32) {
        let estimated_prompt_tokens = self.estimate_rendered_prompt_tokens(incoming_prompt_tokens);
        let force_threshold = self
            .max_llm_input_tokens
            .saturating_mul(u32::from(self.context_compress_threshold_percent))
            / 100;
        let threshold_crossed = estimated_prompt_tokens >= force_threshold;
        if !threshold_crossed && !self.context_compress_required {
            self.threshold_compaction_followup_state = ThresholdCompactionFollowupState::Available;
            return;
        }
        if self.render_prompt_slices().is_empty() {
            return;
        }
        if threshold_crossed {
            // A manual request keeps its user-facing wording, but crossing the
            // automatic threshold still upgrades the model request to critical
            // reasoning and subjects the result to the bounded quality
            // follow-up. The exhausted latch blocks only a new automatic cycle;
            // it must never block an explicit user request.
            if !self.manual_compact_trailer_pending
                && !self.context_compress_required
                && self.threshold_compaction_followup_state
                    == ThresholdCompactionFollowupState::Exhausted
            {
                return;
            }
            if !self.manual_compact_trailer_pending && !self.context_compress_required {
                self.pending_compact_request_notice =
                    Some((estimated_prompt_tokens, force_threshold, None));
            }
            self.threshold_compaction_reasoning_required = true;
        }
        self.context_compress_required = true;
    }

    /// Drains the pending forced-compaction request notice (observed/estimated
    /// prompt tokens, configured force threshold, optional quality target).
    /// Called by the turn loop each iteration.
    pub fn take_pending_compact_request_notice(&mut self) -> Option<(u32, u32, Option<u32>)> {
        self.pending_compact_request_notice.take()
    }

    /// User-initiated compaction request: the next model request must lead
    /// with context_compress, announced with the manual-request wording. The
    /// forced-shrink machinery (response suppression, tool-call gating) is
    /// reused so the compaction actually happens.
    pub fn request_manual_context_compress(&mut self) {
        self.manual_compact_trailer_pending = true;
        self.context_compress_required = true;
        // The "compacting..." UI notice for a manual request is published
        // immediately by the Host when the user clicks, so Core must not
        // schedule a second requested notice here; only the forced-shrink
        // threshold path still emits its own notice.
    }

    fn verify_post_compaction_provider_usage(&mut self, prompt_tokens: u32) -> Option<String> {
        let verification = self.post_compaction_verification.take()?;
        if prompt_tokens == 0 {
            // Some compatible services omit usage. Zero is "not observed", not
            // evidence that compression reached the provider-measured target.
            self.post_compaction_verification = Some(verification);
            return None;
        }
        let target_tokens = self.max_llm_input_tokens.saturating_mul(25) / 100;
        let force_threshold_tokens = self
            .max_llm_input_tokens
            .saturating_mul(u32::from(self.context_compress_threshold_percent))
            / 100;
        let remains_above_target =
            u64::from(prompt_tokens) * 100 > u64::from(self.max_llm_input_tokens) * 25;
        if !remains_above_target {
            self.threshold_compaction_followup_state = ThresholdCompactionFollowupState::Available;
            return None;
        }

        match verification {
            PostCompactionVerification::Initial => {
                self.threshold_compaction_followup_state =
                    ThresholdCompactionFollowupState::FollowupPending;
                self.context_compress_required = true;
                self.threshold_compaction_reasoning_required = true;
                self.pending_compact_request_notice =
                    Some((prompt_tokens, force_threshold_tokens, Some(target_tokens)));
                None
            }
            PostCompactionVerification::Followup => {
                self.threshold_compaction_followup_state =
                    ThresholdCompactionFollowupState::Exhausted;
                let occupancy_percent = (u64::from(prompt_tokens) * 100)
                    .div_ceil(u64::from(self.max_llm_input_tokens))
                    as u32;
                Some(format!(
                    "**WARN**: provider-reported prompt usage remains at {occupancy_percent}% of the model window after one forced compression follow-up; automatic compression will not loop. Retain only necessary state and discard bulky side information during later work."
                ))
            }
        }
    }

    fn threshold_compaction_quality_note(
        &mut self,
        threshold_triggered: bool,
        estimated_before_tokens: u32,
        estimated_after_tokens: u32,
    ) -> (bool, Option<String>) {
        if !threshold_triggered {
            return (false, None);
        }
        let window_tokens = self.max_llm_input_tokens;
        let remains_above_target =
            u64::from(estimated_after_tokens) * 100 > u64::from(window_tokens) * 25;
        if !remains_above_target {
            self.threshold_compaction_followup_state = ThresholdCompactionFollowupState::Available;
            return (false, None);
        }

        if self.threshold_compaction_followup_state == ThresholdCompactionFollowupState::Available {
            self.threshold_compaction_followup_state =
                ThresholdCompactionFollowupState::FollowupPending;
            return (true, None);
        }
        self.threshold_compaction_followup_state = ThresholdCompactionFollowupState::Exhausted;

        let occupancy_percent = |tokens: u32| {
            let numerator = u64::from(tokens) * 100;
            numerator.div_ceil(u64::from(window_tokens)) as u32
        };
        (
            false,
            Some(format!(
                "**WARN**: context compression ratio is not very good, {}% -> {}%; one forced follow-up was already attempted, so automatic compression will not loop. Retain only necessary state and discard bulky side information during later work.",
                occupancy_percent(estimated_before_tokens),
                occupancy_percent(estimated_after_tokens)
            )),
        )
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
        rows.sort_by_key(|row| std::cmp::Reverse(row.time_ms));
        rows.truncate(limit.clamp(1, 50));
        rows
    }

    fn readfile_first_touch_notes(&mut self, outcome: &ActionOutcome) -> String {
        let Some(evidence) = outcome.readfile_result.as_ref() else {
            return String::new();
        };
        if outcome.status != ActionStatus::Completed || evidence.error_type.is_some() {
            return String::new();
        }
        self.first_touch_note_for_paths(vec![PathBuf::from(&evidence.path)])
    }

    fn local_shell_first_touch_notes(&mut self, action: &ParsedAction) -> String {
        let edited_files = action.input_list("edit");
        if edited_files.is_empty() {
            return String::new();
        }
        // The model declares `edit` paths; they anchor first-touch reminders
        // even when the command itself later fails validation or execution.
        let cwd = self.current_prompt_cwd.clone();
        let paths = edited_files
            .iter()
            .map(|file| {
                let path = Path::new(file);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    cwd.join(path)
                }
            })
            .collect::<Vec<_>>();
        self.first_touch_note_for_paths(paths)
    }

    fn first_touch_note_for_paths(&mut self, paths: Vec<PathBuf>) -> String {
        let mut notes = Vec::new();
        for path in paths {
            if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
                if self.touched_paths.insert(dir.to_path_buf()) {
                    let dir_text = dir.to_string_lossy();
                    let dir_text = if dir_text.ends_with('/') {
                        dir_text.into_owned()
                    } else {
                        format!("{dir_text}/")
                    };
                    notes.push(format!(
                        "Reminder: This seems the first time to touch dir {dir_text}, need properly understand the module boundary of this dir, so that your work can be consistent with the architecture, avoiding local blindness."
                    ));
                }
            }
            if self.touched_paths.insert(path.clone()) {
                notes.push(format!(
                    "Reminder: This seems the first time to touch file {}, need to understand the module function of this file, so that your work can be globally consistent, avoiding local blindness.",
                    path.display()
                ));
            }
        }
        notes.join("\n")
    }

    fn action_runtime_notes(
        &mut self,
        action: &ParsedAction,
        outcome: &ActionOutcome,
    ) -> Vec<String> {
        let notes = if action.action == "readfile" {
            self.readfile_first_touch_notes(outcome)
        } else if shell_exec::is_local_shell_action(&action.action) {
            self.local_shell_first_touch_notes(action)
        } else {
            String::new()
        };
        notes
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn truncation_report(fragment: &tool_result_gate::RetainedFragment) -> Option<Value> {
        fragment.truncated.then(|| {
            json!({
                "truncated": true,
                "retained": fragment.retained,
                "original_bytes": fragment.original_bytes,
                "retained_bytes": fragment.retained_bytes,
            })
        })
    }

    fn capture_truncation_report(truncation: &StreamCaptureTruncation) -> Value {
        json!({
            "truncated": true,
            "retained": truncation.retained,
            "original_bytes": truncation.original_bytes,
            "retained_bytes": truncation.retained_bytes,
        })
    }

    fn insert_truncation_stage(
        truncation: &mut serde_json::Map<String, Value>,
        field: &str,
        stage: &str,
        report: Value,
    ) {
        let stages = truncation
            .entry(field.to_string())
            .or_insert_with(|| json!({}));
        if let Some(stages) = stages.as_object_mut() {
            stages.insert(stage.to_string(), report);
        }
    }

    fn insert_sparse_metadata(
        metadata: &mut serde_json::Map<String, Value>,
        key: impl Into<String>,
        value: Value,
    ) {
        let meaningful = match &value {
            Value::Null => false,
            Value::Bool(value) => *value,
            Value::String(value) => !value.is_empty(),
            Value::Array(value) => !value.is_empty(),
            Value::Object(value) => !value.is_empty(),
            Value::Number(_) => true,
        };
        if meaningful {
            metadata.insert(key.into(), value);
        }
    }

    fn structured_tool_output(
        &self,
        action: &ParsedAction,
        outcome: &ActionOutcome,
        output_budget: usize,
    ) -> (Value, serde_json::Map<String, Value>) {
        let tail_out = action
            .raw_input
            .get("tail_out")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| shell_exec::is_local_shell_action(&action.action));
        let retention = tool_result_gate::Retention::from_tail_out(tail_out);
        let output_budget = output_budget.min(self.model_tool_result_bytes);
        if shell_exec::is_local_shell_action(&action.action) {
            if let Some(result) = outcome.bash_result.as_ref() {
                let stdout_budget = if result.stderr.is_empty() {
                    output_budget
                } else {
                    output_budget / 2
                };
                let stderr_budget = if result.stdout.is_empty() {
                    output_budget
                } else {
                    output_budget.saturating_sub(stdout_budget)
                };
                let stdout = tool_result_gate::retain_fragment(
                    result.stdout.trim_end(),
                    stdout_budget,
                    retention,
                );
                let stderr = tool_result_gate::retain_fragment(
                    result.stderr.trim_end(),
                    stderr_budget,
                    retention,
                );
                let mut truncation = serde_json::Map::new();
                if let Some(report) = Self::truncation_report(&stdout) {
                    Self::insert_truncation_stage(
                        &mut truncation,
                        "stdout",
                        "model_result_budget",
                        report,
                    );
                }
                if let Some(report) = Self::truncation_report(&stderr) {
                    Self::insert_truncation_stage(
                        &mut truncation,
                        "stderr",
                        "model_result_budget",
                        report,
                    );
                }
                let mut tool_output = serde_json::Map::new();
                if !stdout.text.is_empty() {
                    tool_output.insert("stdout".to_string(), json!(stdout.text));
                }
                if !stderr.text.is_empty() {
                    tool_output.insert("stderr".to_string(), json!(stderr.text));
                }
                return (Value::Object(tool_output), truncation);
            }
        }
        let content = if action.action == "readfile" {
            outcome
                .readfile_result
                .as_ref()
                .map(|result| result.content.as_str())
        } else if action.action == "memmgr" {
            outcome
                .memmgr_result
                .as_ref()
                .map(|result| result.content.as_str())
        } else if action.action == "self_tool" {
            outcome
                .self_tool_result
                .as_ref()
                .map(|result| result.content.as_str())
        } else {
            None
        }
        .unwrap_or(outcome.text.as_str());
        let content =
            tool_result_gate::retain_fragment(content.trim_end(), output_budget, retention);
        let mut truncation = serde_json::Map::new();
        if let Some(report) = Self::truncation_report(&content) {
            Self::insert_truncation_stage(
                &mut truncation,
                "content",
                "model_result_budget",
                report,
            );
        }
        let mut tool_output = serde_json::Map::new();
        if !content.text.is_empty() {
            tool_output.insert("content".to_string(), json!(content.text));
        }
        (Value::Object(tool_output), truncation)
    }

    fn action_runtime_metadata(
        &self,
        outcome: &ActionOutcome,
        runtime_notes: Vec<String>,
    ) -> Value {
        let mut metadata = serde_json::Map::new();
        metadata.insert("status".to_string(), json!(outcome.status.as_str()));
        if let Some(elapsed_ms) = outcome.elapsed_ms {
            metadata.insert("elapsed_ms".to_string(), json!(elapsed_ms));
        }
        if !runtime_notes.is_empty() {
            let notes = runtime_notes
                .into_iter()
                .map(|note| {
                    tool_result_gate::retain_fragment(
                        &note,
                        2 * 1024,
                        tool_result_gate::Retention::Head,
                    )
                    .text
                })
                .collect::<Vec<_>>();
            metadata.insert("notes".to_string(), json!(notes));
        }
        for (key, value) in &outcome.runtime_metadata {
            if !metadata.contains_key(key) {
                Self::insert_sparse_metadata(&mut metadata, key.clone(), value.clone());
            }
        }
        let mut truncation = serde_json::Map::new();
        if let Some(result) = outcome.bash_result.as_ref() {
            if let Some(value) = result.exit_code {
                metadata.insert("exit_code".to_string(), json!(value));
            }
            if let Some(value) = result.signal {
                metadata.insert("signal".to_string(), json!(value));
            }
            if let Some(value) = result.pid {
                metadata.insert("pid".to_string(), json!(value));
            }
            if result.timed_out {
                metadata.insert("timed_out".to_string(), json!(true));
            }
            if let Some(value) = &result.pid_kind {
                Self::insert_sparse_metadata(&mut metadata, "pid_kind", json!(value));
            }
            if let Some(value) = &result.error_type {
                Self::insert_sparse_metadata(&mut metadata, "error_type", json!(value));
            }
            if let Some(value) = &result.stdout_truncation {
                Self::insert_truncation_stage(
                    &mut truncation,
                    "stdout",
                    "execution_capture",
                    Self::capture_truncation_report(value),
                );
            }
            if let Some(value) = &result.stderr_truncation {
                Self::insert_truncation_stage(
                    &mut truncation,
                    "stderr",
                    "execution_capture",
                    Self::capture_truncation_report(value),
                );
            }
        } else if let Some(result) = outcome.readfile_result.as_ref() {
            Self::insert_sparse_metadata(&mut metadata, "path", json!(result.path));
            if let Some(value) = &result.matcher {
                Self::insert_sparse_metadata(&mut metadata, "matcher", json!(value));
            }
            if let Some(value) = result.start_line {
                metadata.insert("start_line".to_string(), json!(value));
            }
            if let Some(value) = result.end_line {
                metadata.insert("end_line".to_string(), json!(value));
            }
            if let Some(value) = result.total_lines {
                metadata.insert("total_lines".to_string(), json!(value));
            }
            if let Some(value) = &result.encoding {
                Self::insert_sparse_metadata(&mut metadata, "encoding", json!(value));
            }
            if let Some(value) = result.file_bytes {
                metadata.insert("file_bytes".to_string(), json!(value));
            }
            if let Some(value) = result.content_bytes {
                metadata.insert("content_bytes".to_string(), json!(value));
            }
            if result.limited == Some(true) {
                Self::insert_truncation_stage(
                    &mut truncation,
                    "content",
                    "tool_selection",
                    json!({
                        "truncated": true,
                        "retained": if result.tail_out == Some(true) { "tail" } else { "head" },
                        "retained_bytes": result.content_bytes,
                    }),
                );
            }
            if result.tail_out == Some(true) {
                metadata.insert("tail_out".to_string(), json!(true));
            }
            if let Some(value) = &result.error_type {
                Self::insert_sparse_metadata(&mut metadata, "error_type", json!(value));
            }
        } else if let Some(result) = outcome.memmgr_result.as_ref() {
            Self::insert_sparse_metadata(&mut metadata, "memory_type", json!(result.memory_type));
            Self::insert_sparse_metadata(&mut metadata, "operation", json!(result.op));
            if let Some(value) = &result.error_type {
                Self::insert_sparse_metadata(&mut metadata, "error_type", json!(value));
            }
        } else if let Some(result) = outcome.self_tool_result.as_ref() {
            Self::insert_sparse_metadata(&mut metadata, "self_type", json!(result.self_type));
            if let Some(value) = &result.cwd {
                Self::insert_sparse_metadata(&mut metadata, "cwd", json!(value));
            }
            if let Some(value) = &result.error_type {
                Self::insert_sparse_metadata(&mut metadata, "error_type", json!(value));
            }
        }
        if !truncation.is_empty() {
            metadata.insert("truncation".to_string(), Value::Object(truncation));
        }
        Value::Object(metadata)
    }

    fn merge_truncation(
        runtime_metadata: &mut serde_json::Map<String, Value>,
        additional: serde_json::Map<String, Value>,
    ) {
        if additional.is_empty() {
            return;
        }
        let truncation = runtime_metadata
            .entry("truncation".to_string())
            .or_insert_with(|| json!({}));
        let Some(fields) = truncation.as_object_mut() else {
            return;
        };
        for (field, stages) in additional {
            let current = fields.entry(field).or_insert_with(|| json!({}));
            if let (Some(current), Some(stages)) = (current.as_object_mut(), stages.as_object()) {
                for (stage, report) in stages {
                    current.insert(stage.clone(), report.clone());
                }
            }
        }
    }

    fn render_action_result_envelope(
        &self,
        action: &ParsedAction,
        outcome: &ActionOutcome,
        runtime_metadata: &Value,
        output_budget: usize,
    ) -> String {
        let (tool_output, truncation) = self.structured_tool_output(action, outcome, output_budget);
        let mut runtime_metadata = runtime_metadata.as_object().cloned().unwrap_or_default();
        Self::merge_truncation(&mut runtime_metadata, truncation);
        serde_json::to_string(&json!({
            "action_result": {
                "tool_call_id": action.call_id,
                "runtime_metadata": runtime_metadata,
                "tool_output": tool_output,
            }
        }))
        .unwrap_or_else(|_| {
            "{\"action_result\":{\"runtime_metadata\":{\"status\":\"serialization_failed\"}}}"
                .to_string()
        })
    }

    fn format_action_outcome_with_runtime(
        &mut self,
        action: &ParsedAction,
        outcome: &ActionOutcome,
        runtime: &mut dyn ActionRuntime,
    ) -> String {
        if let Some(max_bytes) = runtime.take_model_tool_result_bytes_update() {
            let _ = self.set_model_tool_result_bytes(max_bytes);
        }
        self.format_action_outcome(action, outcome)
    }

    fn format_action_outcome(&mut self, action: &ParsedAction, outcome: &ActionOutcome) -> String {
        let runtime_notes = self.action_runtime_notes(action, outcome);
        let runtime_metadata = self.action_runtime_metadata(outcome, runtime_notes);
        let max_bytes = self.model_tool_result_bytes;
        let mut low = 0usize;
        let mut high = max_bytes;
        let mut best = self.render_action_result_envelope(action, outcome, &runtime_metadata, 0);
        while low <= high {
            let candidate_budget = low + (high - low) / 2;
            let candidate = self.render_action_result_envelope(
                action,
                outcome,
                &runtime_metadata,
                candidate_budget,
            );
            if candidate.len() <= max_bytes {
                best = candidate;
                low = candidate_budget.saturating_add(1);
            } else if candidate_budget == 0 {
                break;
            } else {
                high = candidate_budget - 1;
            }
        }
        best
    }

    fn format_context_compress_outcome(
        &mut self,
        compact: &ParsedContextCompress,
        outcome: &ActionOutcome,
        runtime: &mut dyn ActionRuntime,
    ) -> String {
        let action = ParsedAction {
            action: "context_compress".to_string(),
            name: None,
            call_id: compact.call_id.clone(),
            raw_input: json!({}),
        };
        self.format_action_outcome_with_runtime(&action, outcome, runtime)
    }

    fn action_result_is_error(content: &str) -> bool {
        let Ok(envelope) = serde_json::from_str::<Value>(content) else {
            return true;
        };
        !matches!(
            envelope["action_result"]["runtime_metadata"]["status"].as_str(),
            Some("completed" | "background_finished")
        )
    }

    fn format_pending_action_result(
        &mut self,
        pending: &PendingApproval,
        outcome: &ActionOutcome,
        runtime: &mut dyn ActionRuntime,
    ) -> String {
        let action = ParsedAction {
            action: pending.request.action.clone(),
            name: pending.action_name.clone(),
            call_id: pending.action_call_id.clone(),
            raw_input: json!({ "tail_out": pending.approved_action.tail_out() }),
        };
        self.format_action_outcome_with_runtime(&action, outcome, runtime)
    }

    #[allow(clippy::result_large_err)]
    fn execute_action_groups(
        &mut self,
        groups: Vec<ParsedActionGroup>,
        runtime: &mut dyn ActionRuntime,
    ) -> Result<Vec<String>, (Vec<String>, PendingApproval)> {
        let mut result_lines = Vec::new();
        for group in groups {
            if group.order == ActionGroupOrder::Parallel && group.actions.len() > 1 {
                match self.execute_parallel_action_group(group.actions, runtime) {
                    Ok(group_results) => result_lines.extend(group_results),
                    Err((group_results, pending)) => {
                        result_lines.extend(group_results);
                        return Err((result_lines, pending));
                    }
                }
                continue;
            }
            for action in group.actions {
                match self.execute_action(action.clone(), runtime) {
                    ActionExecution::Completed(outcome) => {
                        result_lines.push(
                            self.format_action_outcome_with_runtime(&action, &outcome, runtime),
                        );
                    }
                    ActionExecution::NeedsApproval(pending) => {
                        return Err((result_lines, pending));
                    }
                }
            }
        }
        Ok(result_lines)
    }

    fn can_spawn_parallel_bash_action(&self, action: &ParsedAction) -> bool {
        self.bash_approval_mode == BashApprovalMode::Approve
            && shell_exec::is_local_shell_action(&action.action)
    }

    fn can_spawn_parallel_readfile_action(&self, action: &ParsedAction) -> bool {
        if action.action != "readfile"
            || self
                .capabilities
                .validate_action_input(&action.action, &action.raw_input)
                .is_err()
        {
            return false;
        }
        matches!(
            executor::resolve_action(&self.capabilities, &action.action),
            Ok(executor::ExecutorTarget::Builtin { binding_name })
                if binding_name == "readfile"
        )
    }

    fn spawn_parallel_readfile_action(
        &mut self,
        idx: usize,
        action: ParsedAction,
    ) -> ParallelActionHandle {
        let action_for_thread = action.clone();
        let cwd = self.current_prompt_cwd().to_path_buf();
        let model_tool_result_bytes = self.model_tool_result_bytes;
        if action_counts_as_tool_call(&action.action) {
            self.current_stats.tool_calls += 1;
        }
        thread::spawn(move || {
            let wall_start = Instant::now();
            let cpu_start = thread_cpu_time();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                readfile::execute_with_timeout_outcome_and_limit(
                    &cwd,
                    &action_for_thread.raw_input,
                    readfile::DEFAULT_TIMEOUT,
                    model_tool_result_bytes,
                )
            }))
            .unwrap_or_else(|_| {
                let path = action_for_thread
                    .raw_input
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
                    .to_string();
                let message =
                    "The tool failed internally. Timem isolated the failure and remains available.";
                ActionOutcome::failed(format!(
                    "Action result: readfile\nerror: builtin_action_panicked\nmessage: {message}"
                ))
                .with_readfile_result(ReadfileResultEvidence {
                    path,
                    matcher: None,
                    start_line: None,
                    end_line: None,
                    total_lines: None,
                    encoding: None,
                    file_bytes: None,
                    content_bytes: None,
                    limited: None,
                    tail_out: None,
                    content: message.to_string(),
                    error_type: Some("InternalError".to_string()),
                })
            });
            let outcome = outcome.with_elapsed_ms(wall_start.elapsed().as_millis() as u64);
            (idx, action, outcome, elapsed_thread_cpu(cpu_start))
        })
    }

    fn spawn_approved_parallel_bash_action(
        &mut self,
        idx: usize,
        pending: PendingApproval,
        cancel_requested: Arc<AtomicBool>,
    ) -> ApprovedParallelBashHandle {
        let action = ParsedAction {
            action: pending.request.action.clone(),
            name: pending.action_name.clone(),
            call_id: pending.action_call_id.clone(),
            raw_input: pending.approved_action.audit_input(
                &pending.request.approval_id,
                &pending.request.risk,
                &pending.request.reason,
            ),
        };
        let pending_for_thread = pending.clone();
        let shell_jobs = self.shell_jobs.clone();
        if action_counts_as_tool_call(&action.action) {
            self.current_stats.tool_calls += 1;
        }
        thread::spawn(move || {
            let wall_start = Instant::now();
            let result = match &pending_for_thread.approved_action {
                PendingApprovedAction::RunBash {
                    command,
                    background,
                    timeout_ms,
                    interval_ms,
                    once_timeout_ms,
                    session_id,
                    turn_id,
                    tool_call_id,
                    cwd,
                    tail_out,
                    edited_files,
                } => {
                    let mut should_cancel = || cancel_requested.load(Ordering::SeqCst);
                    let mut runtime = CancelOnlyActionRuntime::new(&mut should_cancel);
                    shell_exec::execute_approved_bash_with_tail(
                        command,
                        cwd,
                        *background,
                        *timeout_ms,
                        *interval_ms,
                        *once_timeout_ms,
                        session_id,
                        turn_id,
                        tool_call_id,
                        interval_ms.is_none(),
                        *tail_out,
                        edited_files,
                        &pending_for_thread.request,
                        &shell_jobs,
                        &mut runtime,
                    )
                }
                PendingApprovedAction::ToolgenPublish { repo, draft_path } => {
                    toolgen::execute_approved_publish_outcome(
                        repo,
                        draft_path,
                        &pending_for_thread.request,
                    )
                }
            };
            let mut result = result;
            if result.elapsed_ms.is_none() {
                result.elapsed_ms = Some(wall_start.elapsed().as_millis() as u64);
            }
            (idx, action, pending_for_thread, result, None)
        })
    }

    fn spawn_parallel_bash_action(
        &mut self,
        idx: usize,
        action: ParsedAction,
        cancel_requested: Arc<AtomicBool>,
    ) -> ParallelActionHandle {
        let action_for_audit = action.clone();
        let shell_jobs = self.shell_jobs.clone();
        let session_id = self.current_session_id();
        let turn_id = self.current_action_turn_id();
        let cwd = self.current_prompt_cwd().to_path_buf();
        if action_counts_as_tool_call(&action.action) {
            self.current_stats.tool_calls += 1;
        }
        thread::spawn(move || {
            let wall_start = Instant::now();
            let loop_command = action.input_str("loop_cmd");
            let is_regular_command = loop_command.is_empty();
            let cmd_command = action.input_str("cmd");
            let command = if is_regular_command {
                cmd_command.clone()
            } else {
                loop_command.clone()
            };
            let result = if !loop_command.is_empty() && !cmd_command.is_empty() {
                ActionExecution::Completed(ActionOutcome::failed(
                    format!(
                        "Action result: {}\nThe command was not executed.\nReason: The action provided both cmd and loop_cmd. Use cmd for a normal/background command, or loop_cmd with interval_ms for polling.",
                        action.action
                    ),
                ))
            } else {
                let mut should_cancel = || cancel_requested.load(Ordering::SeqCst);
                let mut runtime = CancelOnlyActionRuntime::new(&mut should_cancel);
                shell_exec::execute_run_bash_with_tail(
                    &command,
                    &cwd,
                    action.background(),
                    if is_regular_command {
                        action.timeout_ms_i64(5000)
                    } else {
                        action.input_i64("loop_timeout_ms").unwrap_or(600_000)
                    },
                    action.input_u64("interval_ms"),
                    action.input_u64("once_timeout_ms").unwrap_or(5000),
                    action.input_list("edit"),
                    BashApprovalMode::Approve,
                    &shell_jobs,
                    &session_id,
                    &turn_id,
                    action.call_id.as_str(),
                    is_regular_command,
                    action
                        .raw_input
                        .get("tail_out")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    &mut runtime,
                )
            };
            let outcome = match result {
                ActionExecution::Completed(mut outcome) => {
                    if outcome.elapsed_ms.is_none() {
                        outcome.elapsed_ms = Some(wall_start.elapsed().as_millis() as u64);
                    }
                    outcome
                }
                ActionExecution::NeedsApproval(_) => ActionOutcome::failed(format!(
                    "Action result: {}\ncommand: {}\nerror: unexpected_parallel_approval_request",
                    action.action, command,
                )),
            };
            (idx, action_for_audit, outcome, None)
        })
    }

    fn collect_parallel_action_handles(
        &mut self,
        mut handles: Vec<ParallelActionHandle>,
        results: &mut [Option<String>],
        runtime: &mut dyn ActionRuntime,
        cancel_requested: &Arc<AtomicBool>,
    ) {
        while !handles.is_empty() {
            if runtime.should_cancel() {
                cancel_requested.store(true, Ordering::SeqCst);
            }
            let Some(position) = handles.iter().position(thread::JoinHandle::is_finished) else {
                thread::sleep(Duration::from_millis(20));
                continue;
            };
            let handle = handles.swap_remove(position);
            match handle.join() {
                Ok((idx, action, outcome, cpu_time)) => {
                    self.record_action_audit(&action, outcome.status.as_str(), Some(&outcome.text));
                    self.emit_action_finish_topic(&action, &outcome, cpu_time, runtime);
                    if let Some(slot) = results.get_mut(idx) {
                        *slot = Some(
                            self.format_action_outcome_with_runtime(&action, &outcome, runtime),
                        );
                    }
                }
                Err(_) => {
                    let result =
                        "Action result: parallel\nerror: parallel_action_panicked".to_string();
                    if let Some(slot) = results.iter_mut().find(|slot| slot.is_none()) {
                        *slot = Some(result);
                    }
                }
            }
        }
    }

    fn collect_approved_parallel_bash_handles(
        &mut self,
        mut handles: Vec<ApprovedParallelBashHandle>,
        results: &mut [Option<String>],
        runtime: &mut dyn ActionRuntime,
        cancel_requested: &Arc<AtomicBool>,
    ) {
        while !handles.is_empty() {
            if runtime.should_cancel() {
                cancel_requested.store(true, Ordering::SeqCst);
            }
            let Some(position) = handles.iter().position(thread::JoinHandle::is_finished) else {
                thread::sleep(Duration::from_millis(20));
                continue;
            };
            let handle = handles.swap_remove(position);
            match handle.join() {
                Ok((idx, action, pending, outcome, cpu_time)) => {
                    self.record_pending_approval_audit(&pending, true, &outcome.text);
                    self.emit_action_finish_topic(&action, &outcome, cpu_time, runtime);
                    if let Some(slot) = results.get_mut(idx) {
                        *slot = Some(
                            self.format_action_outcome_with_runtime(&action, &outcome, runtime),
                        );
                    }
                }
                Err(_) => {
                    let result = format!(
                        "Action result: {}\nerror: parallel_action_panicked",
                        crate::os::local_shell_tool_name()
                    );
                    if let Some(slot) = results.iter_mut().find(|slot| slot.is_none()) {
                        *slot = Some(result);
                    }
                }
            }
        }
    }

    fn ordered_parallel_results(results: Vec<Option<String>>) -> Vec<String> {
        results.into_iter().flatten().collect()
    }

    fn pending_approval_with_parallel_continuation(
        mut pending: PendingApproval,
        actions: Vec<ParsedAction>,
        current_index: usize,
        approved: Vec<(usize, PendingApproval)>,
        denied_results: Vec<(usize, String)>,
        completed_results: Vec<(usize, String)>,
    ) -> PendingApproval {
        pending.continuation = Some(PendingApprovalContinuation::ParallelGroup {
            actions,
            current_index,
            approved,
            denied_results,
            completed_results,
        });
        pending
    }

    #[allow(clippy::result_large_err)]
    fn execute_parallel_action_group(
        &mut self,
        actions: Vec<ParsedAction>,
        runtime: &mut dyn ActionRuntime,
    ) -> Result<Vec<String>, (Vec<String>, PendingApproval)> {
        let action_count = actions.len();
        let mut results = vec![None; action_count];
        let mut handles = Vec::new();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        for (idx, action) in actions.iter().cloned().enumerate() {
            if self.can_spawn_parallel_bash_action(&action) {
                self.emit_action_execution_start_topic(&action, runtime);
                handles.push(self.spawn_parallel_bash_action(
                    idx,
                    action,
                    Arc::clone(&cancel_requested),
                ));
                continue;
            }
            if self.can_spawn_parallel_readfile_action(&action) {
                self.emit_action_execution_start_topic(&action, runtime);
                handles.push(self.spawn_parallel_readfile_action(idx, action));
                continue;
            }
            match self.execute_action(action.clone(), runtime) {
                ActionExecution::Completed(outcome) => {
                    results[idx] =
                        Some(self.format_action_outcome_with_runtime(&action, &outcome, runtime));
                }
                ActionExecution::NeedsApproval(pending) => {
                    self.collect_parallel_action_handles(
                        handles,
                        &mut results,
                        runtime,
                        &cancel_requested,
                    );
                    let pending = Self::pending_approval_with_parallel_continuation(
                        pending,
                        actions,
                        idx,
                        Vec::new(),
                        Vec::new(),
                        results
                            .into_iter()
                            .enumerate()
                            .filter_map(|(idx, result)| result.map(|result| (idx, result)))
                            .collect(),
                    );
                    return Err((Vec::new(), pending));
                }
            }
        }
        self.collect_parallel_action_handles(handles, &mut results, runtime, &cancel_requested);
        Ok(Self::ordered_parallel_results(results))
    }

    fn execute_action(
        &mut self,
        action: ParsedAction,
        runtime: &mut dyn ActionRuntime,
    ) -> ActionExecution {
        let wall_start = Instant::now();
        let mut execution = self.execute_action_inner(action, runtime);
        // Completed tool run observation point for disk pressure sampling.
        if matches!(execution, ActionExecution::Completed(_)) {
            self.observe_disk_pressure(&[]);
        }
        let elapsed_ms = wall_start.elapsed().as_millis() as u64;
        match &mut execution {
            ActionExecution::Completed(outcome) => {
                if outcome.elapsed_ms.is_none() {
                    outcome.elapsed_ms = Some(elapsed_ms);
                }
            }
            ActionExecution::NeedsApproval(_) => {}
        }
        execution
    }

    fn execute_action_inner(
        &mut self,
        action: ParsedAction,
        runtime: &mut dyn ActionRuntime,
    ) -> ActionExecution {
        let action_cpu_start = thread_cpu_time();
        let action_for_audit = action.clone();
        let executor_target = match executor::resolve_action(&self.capabilities, &action.action) {
            Ok(target) => target,
            Err(err) => {
                let outcome = ActionOutcome::failed(format!(
                    "Action result: {}\nerror: {}",
                    action.action, err
                ));
                self.record_action_audit(
                    &action_for_audit,
                    outcome.status.as_str(),
                    Some(&outcome.text),
                );
                self.emit_action_finish_topic(
                    &action_for_audit,
                    &outcome,
                    elapsed_thread_cpu(action_cpu_start),
                    runtime,
                );
                return ActionExecution::Completed(outcome);
            }
        };

        if let Err(issue) = self
            .capabilities
            .validate_action_input(&action.action, &action.raw_input)
        {
            let outcome = ActionOutcome::failed(format!(
                "Action result: {}\nerror: invalid_input\nmessage: {}",
                action.action, issue
            ));
            self.record_action_audit(&action_for_audit, "invalid_input", Some(&outcome.text));
            self.emit_action_finish_topic(
                &action_for_audit,
                &outcome,
                elapsed_thread_cpu(action_cpu_start),
                runtime,
            );
            return ActionExecution::Completed(outcome);
        }

        if let executor::ExecutorTarget::Command { path, .. } = &executor_target {
            self.emit_action_execution_start_topic(&action, runtime);
            let outcome = self.execute_command_capability(&action, path);
            self.record_action_audit(
                &action_for_audit,
                outcome.status.as_str(),
                Some(&outcome.text),
            );
            self.emit_action_finish_topic(&action_for_audit, &outcome, None, runtime);
            return ActionExecution::Completed(outcome);
        }

        if let executor::ExecutorTarget::Mcp {
            server_id,
            tool_name,
        } = &executor_target
        {
            if action_counts_as_tool_call(&action.action) {
                self.current_stats.tool_calls += 1;
            }
            self.emit_action_execution_start_topic(&action, runtime);
            let outcome = match self.mcp_servers.get(server_id) {
                Some(config) => {
                    match self
                        .mcp_runtime
                        .call_tool_outcome(config, tool_name, &action.raw_input)
                    {
                        Ok(outcome) => outcome,
                        Err(error) => ActionOutcome::failed(format!(
                            "Action result: {}\nstatus: failed\nerror: {}",
                            action.action, error
                        )),
                    }
                }
                None => ActionOutcome::failed(format!(
                    "Action result: {}\nstatus: failed\nerror: mcp_server_not_enabled",
                    action.action
                )),
            };
            self.record_action_audit(
                &action_for_audit,
                outcome.status.as_str(),
                Some(&outcome.text),
            );
            self.emit_action_finish_topic(&action_for_audit, &outcome, None, runtime);
            return ActionExecution::Completed(outcome);
        }

        let dispatch_name = match &executor_target {
            executor::ExecutorTarget::Builtin { binding_name } => binding_name.as_str(),
            executor::ExecutorTarget::Command { .. } => {
                unreachable!("command target returned early")
            }
            executor::ExecutorTarget::Mcp { .. } => {
                unreachable!("MCP target returned early")
            }
        };

        if action_counts_as_tool_call(&action.action) {
            self.current_stats.tool_calls += 1;
        }
        if !shell_exec::is_local_shell_action(&action.action)
            || self.bash_approval_mode == BashApprovalMode::Approve
        {
            self.emit_action_execution_start_topic(&action, runtime);
        }
        let execution = match tool_registry::execute_builtin_tool(
            self,
            dispatch_name,
            &action,
            runtime,
        ) {
            Ok(Some(execution)) => execution,
            Ok(None) => ActionExecution::Completed(ActionOutcome::failed(format!(
                "Action result: {}\nunsupported native action",
                dispatch_name
            ))),
            Err(_) => {
                let outcome = ActionOutcome::failed(format!(
                    "Action result: {}\nerror: builtin_action_panicked\nmessage: The tool failed internally. Timem isolated the failure and remains available.",
                    dispatch_name
                ));
                self.record_action_audit(&action_for_audit, "internal_error", Some(&outcome.text));
                self.emit_action_finish_topic(
                    &action_for_audit,
                    &outcome,
                    elapsed_thread_cpu(action_cpu_start),
                    runtime,
                );
                return ActionExecution::Completed(outcome);
            }
        };

        match execution {
            ActionExecution::Completed(outcome) => {
                self.record_action_audit(
                    &action_for_audit,
                    outcome.status.as_str(),
                    Some(&outcome.text),
                );
                let cpu_time = if shell_exec::is_local_shell_action(&action_for_audit.action) {
                    None
                } else {
                    elapsed_thread_cpu(action_cpu_start)
                };
                self.emit_action_finish_topic(&action_for_audit, &outcome, cpu_time, runtime);
                ActionExecution::Completed(outcome)
            }
            ActionExecution::NeedsApproval(mut pending) => {
                pending.action_name = action_for_audit.name.clone();
                pending.action_call_id = action_for_audit.call_id.clone();
                let result = format!(
                    "Action result: {}\ncommand: {}\napproval_id: {}\nstatus: needs_user_approval\nrisk: {}\nreason: {}",
                    action_for_audit.action,
                    pending.approved_action.command(),
                    pending.request.approval_id,
                    pending.request.risk,
                    pending.request.reason
                );
                self.record_action_audit(&action_for_audit, "needs_user_approval", Some(&result));
                ActionExecution::NeedsApproval(pending)
            }
        }
    }

    // 实际执行入口，不是模型提出调用的 start 通知。所有执行路径都要覆盖，
    // 否则普通 readfile 等工具缺少边界，消费者无法观察 A结束 -> B执行 的串行时序。
    // 不可按 proposal 顺序假定执行：同批工具可能并行或等待审批；Shell 审批路径
    // 必须在批准之后发送。普通内置、命令扩展、MCP、并行读文件也不能遗漏。
    // 回归：serial_builtin_actions_emit_execution_boundaries_before_each_finish；
    // 真实产品浏览器 tools 场景验证最终答复之前，第二次 readfile 已让第一项折叠。
    fn emit_action_execution_start_topic(
        &self,
        action: &ParsedAction,
        runtime: &mut dyn ActionRuntime,
    ) {
        let notification = notification::notification_from_action(action);
        let mut event = host::notification_topic_event(&self.current_session_id(), &notification);
        event.topic.attributes["event"] = json!("execution_start");
        event.payload["event"] = json!("execution_start");
        event.payload["active"] = json!(true);
        event.payload["status"] = json!("running");
        runtime.on_core_topic_events(&[event]);
    }

    fn emit_action_finish_topic(
        &self,
        action: &ParsedAction,
        outcome: &ActionOutcome,
        cpu_time: Option<Duration>,
        runtime: &mut dyn ActionRuntime,
    ) {
        let notification = notification::notification_from_action(action);
        let mut event = host::notification_topic_event(&self.current_session_id(), &notification);
        event.topic.attributes["event"] = json!("finish");
        event.topic.attributes["active"] = json!(false);
        event.payload["event"] = json!("finish");
        event.payload["active"] = json!(false);
        event.payload["status"] = json!(outcome.status.as_str());
        match cpu_time {
            Some(duration) => {
                event.payload["cpu_time_ns"] =
                    json!(duration.as_nanos().min(u64::MAX as u128) as u64);
                event.payload["cpu_time_available"] = json!(true);
            }
            None => {
                event.payload["cpu_time_available"] = json!(false);
            }
        }
        if action.action == "self_tool"
            && action.input_lower("type") == "cwd"
            && action.raw_input.get("new_path").is_some()
            && outcome.status == ActionStatus::Completed
        {
            event.payload["context_state"] = json!({
                "cwd": self.current_prompt_cwd().display().to_string(),
            });
        }
        if let Some(pid) = managed_running_bash_pid(outcome) {
            event.payload["pid"] = json!(pid);
        }
        runtime.on_core_topic_events(&[event]);
    }

    fn execute_command_capability(&mut self, action: &ParsedAction, path: &Path) -> ActionOutcome {
        if action_counts_as_tool_call(&action.action) {
            self.current_stats.tool_calls += 1;
        }
        let payload = json!({
            "action": action.action,
            "args": action.raw_input,
        });
        if action.background() {
            return self.tool_jobs.spawn_outcome(
                &self.current_session_id(),
                &action.action,
                path,
                &payload,
            );
        }
        executor::execute_command_action_outcome(
            &action.action,
            path,
            &payload,
            action.shell_timeout_ms(),
        )
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
        let spec = self.response_protocol.suite().prompt_boundaries();
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
                    sections.push(format_prompt_slice_for_scratch(&slice, spec));
                }
                for exchange in self
                    .native_exchanges
                    .iter()
                    .filter(|exchange| exchange.delta_id == delta.delta_id)
                {
                    sections.push(format_native_exchange_for_scratch(exchange));
                }
                sections.push(format!("[END SCRATCH OFFLOAD DELTA {}]", delta.delta_id));
                continue;
            }
            for slice in rendered {
                if slice_id_set.contains(&slice.slice_id) {
                    matched_slice_ids.insert(slice.slice_id.clone());
                    sections.push(format_prompt_slice_for_scratch(&slice, spec));
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

    /// Hide every visible slice with the exact structured prompt type. Used
    /// when lifecycle cleanup must not reinterpret or match user/model text.
    fn hide_prompt_slices_by_type(&mut self, prompt_type: &str) {
        for delta in &mut self.deltas {
            for slice in prompt_render::render_delta_slices(delta) {
                if slice.prompt_type == prompt_type
                    && !delta.hidden_slice_ids.contains(&slice.slice_id)
                {
                    delta.hidden_slice_ids.push(slice.slice_id.clone());
                }
            }
        }
    }

    /// Hide every visible slice whose text contains `needle`. Used to retire
    /// superseded long-context maintenance instructions and stale compaction
    /// failure echoes without touching unrelated history.
    fn hide_prompt_slices_matching(&mut self, needle: &str) {
        for delta in &mut self.deltas {
            for slice in prompt_render::render_delta_slices(delta) {
                if slice.text.contains(needle) && !delta.hidden_slice_ids.contains(&slice.slice_id)
                {
                    delta.hidden_slice_ids.push(slice.slice_id.clone());
                }
            }
        }
    }

    pub(crate) fn apply_prompt_shrink(
        &mut self,
        delta_ids: &[String],
        slice_ids: &[String],
    ) -> String {
        // Wholesale removal invalidates the incremental counter; recount.
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
        let estimated_before_tokens = self.dynamic_context_token_estimate().total_tokens();
        let before_delta_count = self.deltas.len();
        if !delta_id_set.is_empty() {
            self.deltas
                .retain(|delta| !delta_id_set.contains(&delta.delta_id));
            self.native_exchanges
                .retain(|exchange| !delta_id_set.contains(&exchange.delta_id));
        }
        let removed_delta_count = before_delta_count.saturating_sub(self.deltas.len());
        // First-touch path tracking mirrors what the model has actually seen in
        // the live prompt context. A shrink rewrites that context, so the
        // tracking resets with it and later reads trigger reminders again.
        self.touched_paths.clear();

        let mut hidden_slice_count = 0usize;
        let mut matched_slice_ids = HashSet::new();
        if !slice_id_set.is_empty() {
            for delta in &mut self.deltas {
                let slices = prompt_render::render_delta_slices(delta);
                for slice in slices {
                    if slice_id_set.contains(&slice.slice_id) {
                        matched_slice_ids.insert(slice.slice_id.clone());
                        if !delta.hidden_slice_ids.contains(&slice.slice_id) {
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

        let shrunk_tokens_estimate = estimated_before_tokens
            .saturating_sub(self.dynamic_context_token_estimate().total_tokens());
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
            "removed_delta_count: {}\nhidden_slice_count: {}\nshrunk_tokens_estimate: {}\nmissing_ids: {}",
            removed_delta_count, hidden_slice_count, shrunk_tokens_estimate, missing_text
        )
    }

    /// Current authoritative delta ids with their text+native token hints for
    /// invalid-reference repair. Native exchanges share the visible id of their
    /// owning delta, including owners with no visible text slices.
    fn live_delta_refs_hint(&self) -> String {
        self.deltas
            .iter()
            .filter(|delta| {
                self.native_exchanges
                    .iter()
                    .any(|exchange| exchange.delta_id == delta.delta_id)
            })
            .rev()
            .take(12)
            .map(|delta| {
                let text_tokens = prompt_render::render_delta_slices(delta)
                    .iter()
                    .map(|slice| estimate_prompt_tokens(&slice.text))
                    .sum::<u32>();
                let native_tokens = self
                    .native_exchanges
                    .iter()
                    .filter(|exchange| exchange.delta_id == delta.delta_id)
                    .map(estimate_native_exchange_tokens)
                    .fold(0_u32, u32::saturating_add);
                format!(
                    "- delta_id={} (text {} + tool_exchanges {})",
                    delta.delta_id, text_tokens, native_tokens
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
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

fn managed_running_bash_pid(outcome: &ActionOutcome) -> Option<u32> {
    if outcome.status != ActionStatus::BackgroundRunning {
        return None;
    }
    let evidence = outcome.bash_result.as_ref()?;
    if evidence.pid_kind.as_deref() != Some(managed_bash_pid_kind()) {
        return None;
    }
    evidence.pid
}

fn managed_bash_pid_kind() -> &'static str {
    #[cfg(unix)]
    {
        "runtime_child_process_group"
    }
    #[cfg(not(unix))]
    {
        "runtime_child_process"
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
            guard: MemGuard::for_memory_domain(dir, "durable-memory"),
        }
    }
    fn write(&self, content: &str) -> std::io::Result<()> {
        let clean = content.trim();
        if clean.is_empty() {
            return Ok(());
        }
        self.guard
            .with_write(|| self.write_clean_unlocked(clean))
            .map_err(std::io::Error::other)??;
        self.snapshot_with_git("memory write");
        Ok(())
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
        Ok(())
    }

    fn query(&self, query: &str, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        self.query_unlocked(query, limit)
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
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }
    fn recent(&self, limit: usize) -> std::io::Result<Vec<MemoryRecord>> {
        let mut rows = self.read_all_unlocked()?;
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }

    fn count(&self) -> std::io::Result<usize> {
        self.read_all_unlocked().map(|rows| rows.len())
    }
    fn update(
        &self,
        operation: &str,
        id: &str,
        content: &str,
        expected_version: Option<u64>,
    ) -> Result<String, String> {
        let result = self
            .guard
            .with_write(|| self.update_unlocked(operation, id, content, expected_version))
            .map_err(|err| err.to_string())?;
        if result.is_ok() {
            self.snapshot_with_git("memory update");
        }
        result
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
        let mut bytes = Vec::new();
        for row in rows {
            writeln!(
                &mut bytes,
                "{}",
                serde_json::to_string(row).unwrap_or_default()
            )?;
        }
        atomic_write_file(&self.file, &bytes)
    }

    fn snapshot_with_git(&self, message: &str) {
        if !self.file.exists() {
            return;
        }
        if timem_platform::command_status(
            Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .arg("init")
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .map(|status| !status.success())
        .unwrap_or(true)
        {
            return;
        }
        let _ = timem_platform::command_status(
            Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .args(["config", "user.name", "timem-memory"])
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        );
        let _ = timem_platform::command_status(
            Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .args(["config", "user.email", "timem-memory@example.invalid"])
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        );
        if timem_platform::command_status(
            Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .args(["add", "memory.jsonl"])
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .map(|status| !status.success())
        .unwrap_or(true)
        {
            return;
        }
        let _ = timem_platform::command_status(
            Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .args(["commit", "-m", message])
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        );
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
        timem_platform::command_output(
            Command::new("git")
                .arg("-C")
                .arg(&self.dir)
                .args(["rev-list", "--count", "HEAD"]),
        )
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

    fn schema_text(&self) -> String {
        "Action result: memmgr\ntype: durable\nop: schema\ntables:\n- memories(id TEXT, created_at_ms INTEGER, updated_at_ms INTEGER, version INTEGER, content TEXT)\n- scratch_notes(id TEXT, created_at_ms INTEGER, scratch_type TEXT, label TEXT, content TEXT, prompt_delta_ids ARRAY, prompt_slice_ids ARRAY)\nsafe_interface: memmgr\nops:\n- durable: schema|sql|insert|update|upsert|delete\n- raw_chat: search|delete\n- scratch: search|write|read|delete\nrules: memmgr sql ops accept SELECT, WITH ... SELECT, or PRAGMA table_info(memories); SQL writes are forbidden; use memmgr type=durable for durable memory insert/update/delete; use expected_version from sql results when updating/deleting an existing durable memory to avoid multi-CLI conflicts; use memmgr type=raw_chat op=delete for explicit chat transcript deletion; scratch write requires kind=notes with content; scratch read requires id and returns full scratch content. Empty raw_chat search_text lists recent chat records. loaded_chat_records={}".to_string()
    }

    fn sql_read(
        &self,
        sql: &str,
        params: &[String],
        limit: usize,
    ) -> Result<Vec<Vec<(String, String)>>, String> {
        self.sql_read_unlocked(sql, params, limit)
    }

    fn sql_read_unlocked(
        &self,
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
        let mut stmt = conn
            .prepare(sql)
            .map_err(|err| format!("sql_prepare_failed: {err}"))?;
        let column_names = stmt
            .column_names()
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let column_count = column_names.len();
        let mut rows = stmt
            .query(params_from_iter(params.iter().map(String::as_str)))
            .map_err(|err| format!("sql_query_failed: {err}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|err| format!("sql_row_failed: {err}"))?
        {
            let mut cells = Vec::new();
            #[allow(clippy::needless_range_loop)]
            for idx in 0..column_count {
                let value = match row
                    .get_ref(idx)
                    .map_err(|err| format!("sql_value_failed: {err}"))?
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
            guard: MemGuard::for_memory_domain(dir, "scratch-notes"),
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
        Ok(self
            .read_all_unlocked()?
            .into_iter()
            .find(|record| record.id == clean_id))
    }

    fn query(&self, query: &str, limit: usize) -> Result<Vec<ScratchNoteRecord>, String> {
        self.query_unlocked(query, limit)
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
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at_ms));
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
        let mut bytes = Vec::new();
        for row in rows {
            writeln!(
                &mut bytes,
                "{}",
                serde_json::to_string(row).unwrap_or_default()
            )
            .map_err(|_| "scratch_write_failed".to_string())?;
        }
        atomic_write_file(&self.file, &bytes).map_err(|_| "scratch_write_failed".to_string())
    }
}

#[derive(Debug, Clone)]
struct FileChatHistoryStore {
    audit_file: PathBuf,
    legacy_audit_file: PathBuf,
}
impl FileChatHistoryStore {
    fn new(memory_dir: &Path) -> Self {
        let space_dir = space_dir_for_memory_dir(memory_dir);
        let audit_file = space_dir.join("audit").join("api_audit.json");
        let legacy_audit_file = space_dir.join("api_audit.jsonl");
        Self {
            audit_file,
            legacy_audit_file,
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
        session_scope: Option<&str>,
    ) -> std::io::Result<Vec<RawChatHistoryRecord>> {
        let mut rows = self.query_unlocked(query, 50, after_ms, before_ms)?;
        if let Some(session_id) = session_scope {
            rows.retain(|record| record.session == session_id);
        }
        rows.truncate(limit.clamp(1, 50));
        Ok(rows)
    }

    fn query_unlocked(
        &self,
        query: &str,
        limit: usize,
        after_ms: Option<i64>,
        before_ms: Option<i64>,
    ) -> std::io::Result<Vec<RawChatHistoryRecord>> {
        let terms = search_terms(query);
        let mut rows = self.read_all_unlocked()?;
        rows.retain(|record| time_in_window(record.started_at_ms, after_ms, before_ms));
        if !terms.is_empty() {
            rows.retain(|record| chat_record_matches(record, &terms));
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.started_at_ms));
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
        MemGuard::for_audit_file(&self.audit_file).with_write(|| {
            let mut deleted_turn_ids = HashSet::new();
            for audit_file in self.audit_files() {
                if !audit_file.exists()
                    && !rolling_file_store::segmented_directory(&audit_file).exists()
                {
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
                    } else {
                        retained.push(value);
                    }
                }
                write_audit_events_unlocked(&audit_file, &retained)
                    .map_err(|_| "chat_history_write_failed".to_string())?;
            }
            Ok::<_, String>(deleted_turn_ids.len())
        })?
    }

    fn read_all_unlocked(&self) -> std::io::Result<Vec<RawChatHistoryRecord>> {
        let mut rows = Vec::<RawChatHistoryRecord>::new();
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
                        rows.push(RawChatHistoryRecord {
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
                            rows.push(RawChatHistoryRecord {
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
    if rolling_file_store::segmented_directory(path).exists() {
        return Ok(rolling_file_store::read_segmented_records(path)?
            .into_iter()
            .filter_map(|record| serde_json::from_slice::<Value>(&record).ok())
            .collect());
    }
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
    let mut bytes = Vec::new();
    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
        let mut records = Vec::with_capacity(events.len());
        for event in events {
            let mut record = serde_json::to_vec(event).map_err(std::io::Error::other)?;
            record.push(b'\n');
            records.push(record);
        }
        if rolling_file_store::segmented_directory(path).exists() {
            let stable_bytes = records
                .iter()
                .map(|record| record.len() as u64)
                .sum::<u64>();
            let slices = stable_bytes
                .div_ceil(rolling_file_store::AUDIT_ROLLING_SLICE_BYTES)
                .max(1);
            let capacity = rolling_file_store::RollingCapacity::with_slice_bytes(
                slices
                    .saturating_add(1)
                    .saturating_mul(rolling_file_store::AUDIT_ROLLING_SLICE_BYTES),
                rolling_file_store::AUDIT_ROLLING_SLICE_BYTES,
            )
            .map_err(std::io::Error::other)?;
            rolling_file_store::rewrite_segmented_records(
                path,
                &records,
                capacity,
                rolling_file_store::AUDIT_ROLLING_SLICE_BYTES,
            )?;
            return Ok(());
        }
        for record in records {
            bytes.extend_from_slice(&record);
        }
    } else {
        let doc = json!({"version": 1, "events": events});
        let text = serde_json::to_string_pretty(&doc).map_err(std::io::Error::other)?;
        bytes.extend_from_slice(format!("{text}\n").as_bytes());
    }
    atomic_write_file(path, &bytes)
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
        {
            return Ok(());
        }
        return Err("only_declared_tables_are_allowed".to_string());
    }
    let allowed_read = lowered.contains(" from memories")
        || lowered.contains(" join memories")
        || lowered.contains(" from (select");
    if !allowed_read {
        return Err("only_declared_tables_are_allowed".to_string());
    }
    Ok(())
}

fn split_prompt_component_text(prompt_type: &str, text: &str, limit: usize) -> Vec<String> {
    if prompt_type == "result_of_llm_action"
        && prompt_render::is_structured_action_result_envelope(text)
    {
        vec![text.to_string()]
    } else {
        split_text_for_prompt_slices(text, limit)
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DynamicContextTokenEstimate {
    visible_delta_count: usize,
    visible_slice_count: usize,
    text_tokens: u32,
    native_tokens: u32,
}

impl DynamicContextTokenEstimate {
    fn total_tokens(self) -> u32 {
        self.text_tokens.saturating_add(self.native_tokens)
    }
}

fn estimate_native_exchange_tokens(exchange: &NativeExchange) -> u32 {
    let calls = exchange
        .calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "name": call.name,
                "arguments": call.raw_arguments,
                "assistant_continuation": call.assistant_continuation,
            })
        })
        .collect::<Vec<_>>();
    let results = exchange
        .results
        .iter()
        .map(|result| {
            json!({
                "tool_call_id": result.call_id,
                "name": result.name,
                "content": result.content,
                "is_error": result.is_error,
            })
        })
        .collect::<Vec<_>>();
    let normalized = json!({
        "assistant": exchange.assistant_text,
        "tool_calls": calls,
        "tool_results": results,
    });
    estimate_prompt_tokens(&normalized.to_string())
}

fn estimate_prompt_tokens(text: &str) -> u32 {
    text.chars().count().div_ceil(4).min(u32::MAX as usize) as u32
}

fn estimate_action_output_tokens(text: &str) -> u32 {
    let (ascii_chars, non_ascii_chars) = text.chars().fold((0usize, 0usize), |counts, ch| {
        if ch.is_ascii() {
            (counts.0 + 1, counts.1)
        } else {
            (counts.0, counts.1 + 1)
        }
    });
    ascii_chars
        .div_ceil(4)
        .saturating_add(non_ascii_chars)
        .min(u32::MAX as usize) as u32
}

fn action_output_too_large_note(output_bytes: usize, remaining_tokens: u32) -> String {
    let output_kb = output_bytes.div_ceil(1024);
    let remaining_kb = (remaining_tokens as usize).saturating_mul(4).div_ceil(1024);
    format!(
        "Your action's output is too large: {output_kb} KB, while the context window has only {remaining_kb} KB left. You need to optimize your action or compress context."
    )
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

fn chat_record_matches(record: &RawChatHistoryRecord, terms: &[String]) -> bool {
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

/// Max consecutive finish attempts the memo-finish-guard can intercept per
/// turn. Non-finishing work responses recharge the budget up to this cap.
const MEMO_FINISH_GUARD_TOKEN_CAP: u32 = 3;

fn memo_finish_guard_reminder(memo: &str) -> String {
    format!(
        "Just now you gave a final answer indicating all tasks are done. But there is still memo active: {memo}. All task/final goal really achieved? If yes, delete the memo (memo op=delete) before giving the final answer; if no, update memo and continue."
    )
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
        "missing_expected_version id={} current_version={} current_content={} hint=read the current row with memmgr type=durable op=sql first, then retry memmgr type=durable op=update with expected_version=current_version",
        id,
        current_version,
        compact_text(current_content, 240)
    )
}

fn should_run_memory_precheck(supporting_context: &str) -> bool {
    supporting_context.contains("memory_lookup_hint:")
}
fn compact_text(text: &str, max_chars: usize) -> String {
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

fn prompt_type_role_for_scratch(
    prompt_type: &str,
    spec: &crate::response_protocol::PromptBoundarySpec,
) -> &'static str {
    match prompt_type {
        "user_question" | "user_supplement" | "user_resume_directly" => spec.user_role,
        "llm_response"
        | "llm_response_raw_xml"
        | "llm_free_talk"
        | "context_compression_summary" => spec.assistant_role,
        "result_of_llm_action" => spec.runtime_role,
        _ => spec.runtime_role,
    }
}

fn format_prompt_slice_for_scratch(
    slice: &PromptSlice,
    spec: &crate::response_protocol::PromptBoundarySpec,
) -> String {
    format!(
        "[BEGIN SCRATCH OFFLOAD BLOCK]\ndelta_id: {}\ntime_ms: {}\nrole: {}\n{}\n[END SCRATCH OFFLOAD BLOCK]",
        slice.delta_id,
        slice.time_ms,
        prompt_type_role_for_scratch(&slice.prompt_type, spec),
        slice.text
    )
}

fn format_native_exchange_for_scratch(exchange: &NativeExchange) -> String {
    let calls = exchange
        .calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "name": call.name,
                "arguments": call.arguments,
            })
        })
        .collect::<Vec<_>>();
    let results = exchange
        .results
        .iter()
        .map(|result| {
            json!({
                "tool_call_id": result.call_id,
                "name": result.name,
                "content": result.content,
                "is_error": result.is_error,
            })
        })
        .collect::<Vec<_>>();
    format!(
        "[BEGIN SCRATCH OFFLOAD NATIVE EXCHANGE]\ndelta_id: {}\nassistant_text: {}\ncalls: {}\nresults: {}\n[END SCRATCH OFFLOAD NATIVE EXCHANGE]",
        exchange.delta_id,
        exchange.assistant_text,
        serde_json::to_string(&calls).unwrap_or_default(),
        serde_json::to_string(&results).unwrap_or_default(),
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
    #[serde(default)]
    interface_preferences: InterfacePreferences,
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
        core: AgentCore::new_with_interface_preferences(
            config.static_prompt,
            config.profile,
            config.memory_dir,
            config.interface_preferences,
        ),
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
            tool_calls: Vec::new(),
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

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod prompt_component_tests;

use crate::{
    context_policy::StaleContextDecisionRequest,
    notification::{CoreActionKind, CoreMemoryActivity, CoreNotification},
    redaction::redact_value,
    work_instructions::{
        WorkInstructionLoadReport, WorkInstructionLoadRequest, WorkInstructionLoadStatus,
    },
    ApprovalRequest, CoreProfile, UsageStats,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TurnInput<'a> {
    pub input: &'a str,
    pub session: &'a str,
    pub audit_file: &'a Path,
    pub runtime: &'a str,
    pub run_bash_target: &'a str,
    pub additional_context: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub text: String,
    pub stats: UsageStats,
    pub latest_usage: Option<UsageStats>,
    pub elapsed: Duration,
    pub repair_issue: Option<String>,
    pub stop_reason: Option<TurnStopReason>,
    pub stop_summary: Option<TurnStopSummary>,
}

impl TurnOutcome {
    pub fn final_response(
        text: impl Into<String>,
        stats: UsageStats,
        latest_usage: Option<UsageStats>,
        repair_issue: Option<String>,
        elapsed: Duration,
    ) -> Self {
        Self {
            text: text.into(),
            stats,
            latest_usage,
            elapsed,
            repair_issue,
            stop_reason: None,
            stop_summary: None,
        }
    }

    pub fn stopped(text: impl Into<String>, stopped: StoppedTurn, elapsed: Duration) -> Self {
        Self {
            text: text.into(),
            stats: stopped.stats,
            latest_usage: stopped.latest_usage,
            elapsed,
            repair_issue: stopped.repair_issue,
            stop_reason: Some(stopped.stop_reason),
            stop_summary: Some(stopped.stop_summary),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnStopReason {
    CancelledByUser,
    ModelError,
    OutputLimitStoppedByUser,
    RoundLimitReached,
    ProtocolRepairFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnStopDetail {
    None,
    ModelError {
        error: String,
    },
    OutputLimit {
        current_tokens: u32,
    },
    RoundLimit {
        max_rounds: u32,
    },
    ProtocolRepairFailure {
        first_issue: String,
        final_issue: String,
        truncated: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnStopSummary {
    pub stats: UsageStats,
    pub latest_usage: Option<UsageStats>,
    pub repair_issue: Option<String>,
    pub stop_reason: TurnStopReason,
    pub detail: TurnStopDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoppedTurn {
    pub stats: UsageStats,
    pub latest_usage: Option<UsageStats>,
    pub repair_issue: Option<String>,
    pub stop_reason: TurnStopReason,
    pub stop_summary: TurnStopSummary,
}

impl TurnStopSummary {
    pub fn cancelled_by_user() -> Self {
        Self {
            stats: UsageStats::zero(),
            latest_usage: None,
            repair_issue: Some("cancelled_by_user".to_string()),
            stop_reason: TurnStopReason::CancelledByUser,
            detail: TurnStopDetail::None,
        }
    }

    pub fn model_error(error: impl Into<String>) -> Self {
        Self {
            stats: UsageStats::zero(),
            latest_usage: None,
            repair_issue: None,
            stop_reason: TurnStopReason::ModelError,
            detail: TurnStopDetail::ModelError {
                error: error.into(),
            },
        }
    }

    pub fn output_limit_stopped_by_user(current_tokens: u32, usage: UsageStats) -> Self {
        Self {
            stats: usage.clone(),
            latest_usage: Some(usage),
            repair_issue: Some("truncated_output_stopped_by_user".to_string()),
            stop_reason: TurnStopReason::OutputLimitStoppedByUser,
            detail: TurnStopDetail::OutputLimit { current_tokens },
        }
    }

    pub fn round_limit_stopped_by_user(
        max_rounds: u32,
        stats: UsageStats,
        latest_usage: Option<UsageStats>,
    ) -> Self {
        Self {
            stats,
            latest_usage,
            repair_issue: Some("round_limit_reached".to_string()),
            stop_reason: TurnStopReason::RoundLimitReached,
            detail: TurnStopDetail::RoundLimit { max_rounds },
        }
    }

    pub fn protocol_repair_failed(
        first_issue: impl Into<String>,
        final_issue: impl Into<String>,
        truncated: bool,
        stats: UsageStats,
        latest_usage: Option<UsageStats>,
    ) -> Self {
        let first_issue = first_issue.into();
        let final_issue = final_issue.into();
        Self {
            stats,
            latest_usage,
            repair_issue: Some(final_issue.clone()),
            stop_reason: TurnStopReason::ProtocolRepairFailed,
            detail: TurnStopDetail::ProtocolRepairFailure {
                first_issue,
                final_issue,
                truncated,
            },
        }
    }

    pub fn into_stopped_turn(self) -> StoppedTurn {
        StoppedTurn {
            stats: self.stats.clone(),
            latest_usage: self.latest_usage.clone(),
            repair_issue: self.repair_issue.clone(),
            stop_reason: self.stop_reason,
            stop_summary: self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundLimitDecisionRequest {
    pub max_rounds: u32,
    pub recharge_rounds: u32,
    pub keep_task_context: bool,
}

impl RoundLimitDecisionRequest {
    pub fn new(max_rounds: u32) -> Self {
        Self {
            max_rounds,
            recharge_rounds: max_rounds,
            keep_task_context: true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum RoundLimitResolution {
    Continue(crate::CoreStep),
    Stop(TurnStopSummary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputExpansionRequest {
    pub current_tokens: u32,
    pub increment_tokens: u32,
    pub retry_same_turn: bool,
}

impl OutputExpansionRequest {
    pub fn new(current_tokens: u32) -> Self {
        Self {
            current_tokens,
            increment_tokens: 10_000,
            retry_same_turn: true,
        }
    }

    pub fn expanded_tokens(self) -> u32 {
        self.current_tokens.saturating_add(self.increment_tokens)
    }
}

#[derive(Debug, Clone)]
pub enum OutputExpansionResolution {
    RetryWithExpandedLimit { max_llm_output_tokens: u32 },
    Stop(TurnStopSummary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSessionState {
    Running,
    WaitingModel,
    WaitingUser,
    WaitingUserWithTimeout { timeout: Duration },
    Paused,
    Stopped,
    Finished,
    Error,
}

impl CoreSessionState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingModel => "waiting_model",
            Self::WaitingUser => "waiting_user",
            Self::WaitingUserWithTimeout { .. } => "waiting_user_with_timeout",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Finished => "finished",
            Self::Error => "error",
        }
    }

    pub fn timeout_ms(self) -> Option<u128> {
        match self {
            Self::WaitingUserWithTimeout { timeout } => Some(timeout.as_millis()),
            Self::Running
            | Self::WaitingModel
            | Self::WaitingUser
            | Self::Paused
            | Self::Stopped
            | Self::Finished
            | Self::Error => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTopic {
    pub name: String,
    pub attributes: Value,
}

impl CoreTopic {
    pub fn new(name: impl Into<String>, attributes: Value) -> Self {
        Self {
            name: name.into(),
            attributes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTopicEvent {
    pub session_id: String,
    pub topic: CoreTopic,
    pub state: CoreSessionState,
    pub payload: Value,
}

pub const CORE_TOPIC_MODEL_RESPONSE: &str = "core.model.response";
pub const CORE_TOPIC_ACTION: &str = "core.action";
pub const CORE_TOPIC_LIFECYCLE: &str = "core.lifecycle";
pub const CORE_TOPIC_USER_APPROVAL_REQUEST: &str = "core.user.approval.request";
pub const CORE_TOPIC_ROUND_LIMIT_REQUEST: &str = "core.user.round_limit.request";
pub const CORE_TOPIC_OUTPUT_EXPAND_REQUEST: &str = "core.user.output_expand.request";
pub const CORE_TOPIC_STALE_CONTEXT_REQUEST: &str = "core.user.stale_context.request";
pub const CORE_TOPIC_WORK_INSTRUCTION_LOAD: &str = "core.work_instruction_load";
static TOPIC_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreModelResponseTopic {
    pub status: String,
    pub free_talk: String,
    pub report_job_progress: String,
    pub final_answer: String,
    pub continue_work: bool,
    pub global: CoreGlobalWorkerStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreWorkInstructionLoadTopic {
    pub status: String,
    pub directory: String,
    pub file_names: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreGlobalWorkerStatus {
    pub working_worker_count: usize,
}

impl CoreGlobalWorkerStatus {
    pub fn new(working_worker_count: usize) -> Self {
        Self {
            working_worker_count,
        }
    }
}

impl Default for CoreGlobalWorkerStatus {
    fn default() -> Self {
        Self {
            working_worker_count: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreActionTopic {
    pub intent: Option<String>,
    pub action: String,
    pub input: Value,
    pub kind: CoreActionKind,
    pub active: bool,
    pub memory_activity: CoreMemoryActivity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTopicStatusHint {
    pub intent: Option<String>,
    pub action: String,
    pub input: Value,
    pub memory_activity: CoreMemoryActivity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSessionWorkerIdentity {
    pub session_id: String,
    pub display_name: String,
    pub ordinal: u32,
    pub parent_session_id: Option<String>,
}

impl CoreSessionWorkerIdentity {
    pub fn new(
        session_id: impl Into<String>,
        ordinal: u32,
        display_name: Option<String>,
        parent_session_id: Option<String>,
    ) -> Self {
        let ordinal = ordinal.max(1);
        Self {
            session_id: session_id.into(),
            display_name: session_worker_default_display_name(ordinal, display_name),
            ordinal,
            parent_session_id,
        }
    }

    pub fn rename(&mut self, display_name: impl Into<String>) {
        let display_name = display_name.into();
        if !display_name.trim().is_empty() {
            self.display_name = display_name.trim().to_string();
        }
    }
}

pub fn session_worker_default_display_name(ordinal: u32, requested: Option<String>) -> String {
    requested
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("[Ai{}]", ordinal.max(1)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSessionWorkerWorkspace {
    pub current_dir: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub audit_file: PathBuf,
    pub runtime: String,
    pub run_bash_target: String,
    pub env: BTreeMap<String, String>,
    pub workspace_dirs: Vec<PathBuf>,
}

impl CoreSessionWorkerWorkspace {
    pub fn new(
        data_dir: impl Into<PathBuf>,
        audit_file: impl Into<PathBuf>,
        runtime: impl Into<String>,
        run_bash_target: impl Into<String>,
    ) -> Self {
        Self {
            current_dir: None,
            data_dir: data_dir.into(),
            audit_file: audit_file.into(),
            runtime: runtime.into(),
            run_bash_target: run_bash_target.into(),
            env: BTreeMap::new(),
            workspace_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreDynamicContextSummary {
    pub visible_delta_count: usize,
    pub visible_slice_count: usize,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreLifecycleTopic {
    pub event: CoreLifecycleEvent,
    pub version: String,
    pub profile: CoreProfile,
    pub response_protocol: String,
    pub max_llm_input_tokens: u32,
    pub max_rounds: u32,
    pub tool_count: usize,
    pub skill_count: usize,
    pub worker: Option<CoreSessionWorkerIdentity>,
    pub workspace: Option<CoreSessionWorkerWorkspace>,
    pub context: Option<CoreDynamicContextSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreLifecycleEvent {
    Initialized,
}

impl CoreLifecycleEvent {
    pub fn name(self) -> &'static str {
        match self {
            Self::Initialized => "initialized",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "initialized" => Some(Self::Initialized),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreHostDecisionRequestTopic {
    pub session_id: String,
    pub kind: &'static str,
    pub state: CoreSessionState,
    pub safe_default: HostDecisionDefault,
    pub timeout: Option<Duration>,
    pub request: HostDecisionRequest,
}

impl CoreTopicEvent {
    pub fn new(
        session_id: impl Into<String>,
        topic: CoreTopic,
        state: CoreSessionState,
        payload: Value,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            topic,
            state,
            payload,
        }
    }

    pub fn state_payload(&self) -> Value {
        let mut state = json!({
            "name": self.state.name(),
        });
        if let Some(timeout_ms) = self.state.timeout_ms() {
            state["timeout_ms"] = json!(timeout_ms);
        }
        state
    }

    pub fn wire_payload(&self) -> Value {
        json!({
            "session_id": &self.session_id,
            "topic": {
                "name": &self.topic.name,
                "attributes": &self.topic.attributes,
            },
            "state": self.state_payload(),
            "payload": &self.payload,
        })
    }

    pub fn expects_reply(&self) -> bool {
        self.topic.attributes["expects_reply"]
            .as_bool()
            .unwrap_or(false)
    }

    pub fn is_blocking_request(&self) -> bool {
        self.expects_reply()
            && matches!(
                self.state,
                CoreSessionState::WaitingUser | CoreSessionState::WaitingUserWithTimeout { .. }
            )
    }

    pub fn request_id(&self) -> Option<&str> {
        self.payload["request_id"].as_str()
    }

    pub fn as_model_response(&self) -> Option<CoreModelResponseTopic> {
        if self.topic.name != CORE_TOPIC_MODEL_RESPONSE {
            return None;
        }
        Some(CoreModelResponseTopic {
            status: self.payload["status"]
                .as_str()
                .unwrap_or("working")
                .to_string(),
            free_talk: self.payload["free_talk"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            report_job_progress: self.payload["report_job_progress"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            final_answer: self.payload["final_answer"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            continue_work: self.payload["continue_work"].as_bool().unwrap_or(true),
            global: parse_global_worker_status(&self.payload["global"]),
        })
    }

    pub fn with_global_worker_status(mut self, status: CoreGlobalWorkerStatus) -> Self {
        self.payload["global"] = global_worker_status_payload(status);
        self
    }

    pub fn as_action(&self) -> Option<CoreActionTopic> {
        if self.topic.name != CORE_TOPIC_ACTION {
            return None;
        }
        Some(CoreActionTopic {
            intent: self.payload["intent"]
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string),
            action: self.payload["action"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            input: self.payload["input"].clone(),
            kind: action_kind_from_topic_payload(
                &self.payload["kind"],
                self.payload["action"].as_str().unwrap_or_default(),
            ),
            active: self.payload["active"].as_bool().unwrap_or(false),
            memory_activity: memory_activity_from_topic_payload(&self.payload["memory_activity"]),
        })
    }

    pub fn as_lifecycle(&self) -> Option<CoreLifecycleTopic> {
        if self.topic.name != CORE_TOPIC_LIFECYCLE {
            return None;
        }
        let worker = parse_worker_identity(&self.payload["worker"]);
        let workspace = parse_worker_workspace(&self.payload["workspace"]);
        let context = parse_dynamic_context_summary(&self.payload["context"]);
        Some(CoreLifecycleTopic {
            event: CoreLifecycleEvent::from_name(self.payload["event"].as_str()?)?,
            version: self.payload["version"].as_str()?.to_string(),
            profile: CoreProfile {
                name: self.payload["profile"]["name"].as_str()?.to_string(),
                provider: self.payload["profile"]["provider"].as_str()?.to_string(),
                model: self.payload["profile"]["model"].as_str()?.to_string(),
            },
            response_protocol: self.payload["response_protocol"].as_str()?.to_string(),
            max_llm_input_tokens: self.payload["max_llm_input_tokens"].as_u64()? as u32,
            max_rounds: self.payload["max_rounds"].as_u64()? as u32,
            tool_count: self.payload["capabilities"]["tools"].as_u64()? as usize,
            skill_count: self.payload["capabilities"]["skills"].as_u64()? as usize,
            worker,
            workspace,
            context,
        })
    }

    pub fn as_host_decision_request(&self) -> Option<CoreHostDecisionRequestTopic> {
        let kind = self.payload["kind"].as_str()?;
        let request = host_decision_request_from_payload(kind, &self.payload["request"])?;
        Some(CoreHostDecisionRequestTopic {
            session_id: self.session_id.clone(),
            kind: request.kind(),
            state: self.state,
            safe_default: host_decision_default_from_name(
                self.payload["safe_default"].as_str().unwrap_or_default(),
            )
            .unwrap_or_else(|| request.safe_default()),
            timeout: self.payload["timeout_ms"]
                .as_u64()
                .map(Duration::from_millis)
                .or_else(|| request.timeout()),
            request,
        })
    }

    pub fn as_work_instruction_load(&self) -> Option<CoreWorkInstructionLoadTopic> {
        if self.topic.name != CORE_TOPIC_WORK_INSTRUCTION_LOAD {
            return None;
        }
        if self.payload.get("status").is_none() {
            return None;
        }
        Some(CoreWorkInstructionLoadTopic {
            status: self.payload["status"].as_str()?.to_string(),
            directory: self.payload["directory"].as_str()?.to_string(),
            file_names: self.payload["file_names"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            error: self.payload["error"].as_str().map(str::to_string),
        })
    }
}

pub fn core_initialized_topic_event(
    session_id: impl Into<String>,
    profile: &CoreProfile,
    response_protocol: impl Into<String>,
    max_llm_input_tokens: u32,
    max_rounds: u32,
    tool_count: usize,
    skill_count: usize,
) -> CoreTopicEvent {
    core_initialized_topic_event_with_worker(
        session_id,
        profile,
        response_protocol,
        max_llm_input_tokens,
        max_rounds,
        tool_count,
        skill_count,
        None,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn core_initialized_topic_event_with_worker(
    session_id: impl Into<String>,
    profile: &CoreProfile,
    response_protocol: impl Into<String>,
    max_llm_input_tokens: u32,
    max_rounds: u32,
    tool_count: usize,
    skill_count: usize,
    worker: Option<&CoreSessionWorkerIdentity>,
    workspace: Option<&CoreSessionWorkerWorkspace>,
    context: Option<CoreDynamicContextSummary>,
) -> CoreTopicEvent {
    let response_protocol = response_protocol.into();
    CoreTopicEvent::new(
        session_id,
        CoreTopic::new(
            CORE_TOPIC_LIFECYCLE,
            json!({
                "name": CORE_TOPIC_LIFECYCLE,
                "event": CoreLifecycleEvent::Initialized.name(),
            }),
        ),
        CoreSessionState::Running,
        json!({
            "event": CoreLifecycleEvent::Initialized.name(),
            "version": env!("CARGO_PKG_VERSION"),
            "profile": {
                "name": &profile.name,
                "provider": &profile.provider,
                "model": &profile.model,
            },
            "response_protocol": response_protocol,
            "max_llm_input_tokens": max_llm_input_tokens,
            "max_rounds": max_rounds,
            "capabilities": {
                "tools": tool_count,
                "skills": skill_count,
            },
            "worker": worker.map(worker_identity_payload),
            "workspace": workspace.map(worker_workspace_payload),
            "context": context.map(dynamic_context_summary_payload),
        }),
    )
}

pub fn work_instruction_load_topic_event(
    session_id: impl Into<String>,
    report: &WorkInstructionLoadReport,
) -> CoreTopicEvent {
    let status = match report.status {
        WorkInstructionLoadStatus::Loaded => "loaded",
        WorkInstructionLoadStatus::NotFound => "not_found",
        WorkInstructionLoadStatus::Failed => "failed",
    };
    CoreTopicEvent::new(
        session_id,
        CoreTopic::new(
            CORE_TOPIC_WORK_INSTRUCTION_LOAD,
            json!({
                "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
            }),
        ),
        CoreSessionState::Running,
        json!({
            "status": status,
            "directory": report.directory.display().to_string(),
            "file_names": report.file_names.clone(),
            "error": report.error.clone(),
        }),
    )
}

fn worker_identity_payload(identity: &CoreSessionWorkerIdentity) -> Value {
    json!({
        "session_id": &identity.session_id,
        "display_name": &identity.display_name,
        "ordinal": identity.ordinal,
        "parent_session_id": &identity.parent_session_id,
    })
}

fn parse_worker_identity(value: &Value) -> Option<CoreSessionWorkerIdentity> {
    if value.is_null() {
        return None;
    }
    Some(CoreSessionWorkerIdentity {
        session_id: value["session_id"].as_str()?.to_string(),
        display_name: value["display_name"].as_str()?.to_string(),
        ordinal: value["ordinal"].as_u64()? as u32,
        parent_session_id: value["parent_session_id"].as_str().map(str::to_string),
    })
}

fn worker_workspace_payload(workspace: &CoreSessionWorkerWorkspace) -> Value {
    let env = redact_value(&json!(&workspace.env));
    json!({
        "current_dir": workspace.current_dir.as_ref().map(path_string),
        "data_dir": path_string(&workspace.data_dir),
        "audit_file": path_string(&workspace.audit_file),
        "runtime": &workspace.runtime,
        "run_bash_target": &workspace.run_bash_target,
        "env": env,
        "workspace_dirs": workspace.workspace_dirs.iter().map(path_string).collect::<Vec<_>>(),
    })
}

fn parse_worker_workspace(value: &Value) -> Option<CoreSessionWorkerWorkspace> {
    if value.is_null() {
        return None;
    }
    let env = value["env"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let workspace_dirs = value["workspace_dirs"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(PathBuf::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(CoreSessionWorkerWorkspace {
        current_dir: value["current_dir"].as_str().map(PathBuf::from),
        data_dir: PathBuf::from(value["data_dir"].as_str()?),
        audit_file: PathBuf::from(value["audit_file"].as_str()?),
        runtime: value["runtime"].as_str()?.to_string(),
        run_bash_target: value["run_bash_target"].as_str()?.to_string(),
        env,
        workspace_dirs,
    })
}

fn dynamic_context_summary_payload(context: CoreDynamicContextSummary) -> Value {
    json!({
        "visible_delta_count": context.visible_delta_count,
        "visible_slice_count": context.visible_slice_count,
        "estimated_tokens": context.estimated_tokens,
    })
}

fn global_worker_status_payload(status: CoreGlobalWorkerStatus) -> Value {
    json!({
        "working_worker_count": status.working_worker_count,
    })
}

fn parse_global_worker_status(value: &Value) -> CoreGlobalWorkerStatus {
    CoreGlobalWorkerStatus {
        working_worker_count: value["working_worker_count"].as_u64().unwrap_or(0) as usize,
    }
}

fn parse_dynamic_context_summary(value: &Value) -> Option<CoreDynamicContextSummary> {
    if value.is_null() {
        return None;
    }
    Some(CoreDynamicContextSummary {
        visible_delta_count: value["visible_delta_count"].as_u64()? as usize,
        visible_slice_count: value["visible_slice_count"].as_u64()? as usize,
        estimated_tokens: value["estimated_tokens"].as_u64()? as u32,
    })
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().display().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicReply {
    pub session_id: String,
    pub topic_name: String,
    pub request_id: Option<String>,
    pub decision: HostDecision,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicReplyError {
    NotBlockingRequest,
    SessionMismatch,
    TopicMismatch,
    RequestIdMismatch,
}

impl TopicReply {
    pub fn new(
        session_id: impl Into<String>,
        topic_name: impl Into<String>,
        decision: HostDecision,
        payload: Value,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            topic_name: topic_name.into(),
            request_id: None,
            decision,
            payload,
        }
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn for_decision_request(event: &CoreTopicEvent, decision: HostDecision) -> Option<Self> {
        if !event.is_blocking_request() {
            return None;
        }
        let mut reply = Self::new(
            event.session_id.clone(),
            event.topic.name.clone(),
            decision,
            json!({ "decision": decision.name() }),
        );
        if let Some(request_id) = event.request_id() {
            reply = reply.with_request_id(request_id);
        }
        Some(reply)
    }

    pub fn wire_payload(&self) -> Value {
        json!({
            "session_id": self.session_id,
            "topic_name": self.topic_name,
            "request_id": self.request_id,
            "decision": self.decision.name(),
            "payload": self.payload,
        })
    }
}

pub fn resolve_topic_reply(
    request_event: &CoreTopicEvent,
    expected_request_id: Option<&str>,
    reply: &TopicReply,
) -> Result<HostDecision, TopicReplyError> {
    if !request_event.is_blocking_request() {
        return Err(TopicReplyError::NotBlockingRequest);
    }
    if reply.session_id != request_event.session_id {
        return Err(TopicReplyError::SessionMismatch);
    }
    if reply.topic_name != request_event.topic.name {
        return Err(TopicReplyError::TopicMismatch);
    }
    let expected_request_id = expected_request_id.or_else(|| request_event.request_id());
    if expected_request_id != reply.request_id.as_deref() {
        return Err(TopicReplyError::RequestIdMismatch);
    }
    Ok(reply.decision)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostDecisionDefault {
    Accept,
    Decline,
}

impl HostDecisionDefault {
    pub fn as_bool(self) -> bool {
        matches!(self, Self::Accept)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostDecision {
    Accept,
    Decline,
}

impl HostDecision {
    pub fn as_bool(self) -> bool {
        matches!(self, Self::Accept)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Decline => "decline",
        }
    }
}

impl From<HostDecisionDefault> for HostDecision {
    fn from(value: HostDecisionDefault) -> Self {
        match value {
            HostDecisionDefault::Accept => Self::Accept,
            HostDecisionDefault::Decline => Self::Decline,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostDecisionRequest {
    UserApproval(ApprovalRequest),
    RoundLimitContinue(RoundLimitDecisionRequest),
    OutputExpansion(OutputExpansionRequest),
    StaleContextContinue(StaleContextDecisionRequest),
    WorkInstructionLoad(WorkInstructionLoadRequest),
}

impl HostDecisionRequest {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UserApproval(_) => "user_approval",
            Self::RoundLimitContinue(_) => "round_limit_continue",
            Self::OutputExpansion(_) => "output_expansion",
            Self::StaleContextContinue(_) => "stale_context_continue",
            Self::WorkInstructionLoad(_) => "work_instruction_load",
        }
    }

    pub fn safe_default(&self) -> HostDecisionDefault {
        match self {
            Self::UserApproval(_) | Self::RoundLimitContinue(_) | Self::OutputExpansion(_) => {
                HostDecisionDefault::Accept
            }
            Self::StaleContextContinue(_) | Self::WorkInstructionLoad(_) => {
                HostDecisionDefault::Decline
            }
        }
    }

    pub fn timeout(&self) -> Option<Duration> {
        match self {
            Self::WorkInstructionLoad(_) => Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT),
            Self::UserApproval(_)
            | Self::RoundLimitContinue(_)
            | Self::OutputExpansion(_)
            | Self::StaleContextContinue(_) => None,
        }
    }

    pub fn topic_name(&self) -> &'static str {
        match self {
            Self::UserApproval(_) => CORE_TOPIC_USER_APPROVAL_REQUEST,
            Self::RoundLimitContinue(_) => CORE_TOPIC_ROUND_LIMIT_REQUEST,
            Self::OutputExpansion(_) => CORE_TOPIC_OUTPUT_EXPAND_REQUEST,
            Self::StaleContextContinue(_) => CORE_TOPIC_STALE_CONTEXT_REQUEST,
            Self::WorkInstructionLoad(_) => CORE_TOPIC_WORK_INSTRUCTION_LOAD,
        }
    }

    pub fn topic_event(&self, session_id: impl Into<String>) -> CoreTopicEvent {
        self.topic_event_with_request_id(session_id, next_topic_request_id(self.kind()))
    }

    pub fn topic_event_with_request_id(
        &self,
        session_id: impl Into<String>,
        request_id: impl Into<String>,
    ) -> CoreTopicEvent {
        self.topic_event_inner(session_id, Some(request_id.into()))
    }

    fn topic_event_inner(
        &self,
        session_id: impl Into<String>,
        request_id: Option<String>,
    ) -> CoreTopicEvent {
        let state = self
            .timeout()
            .map(|timeout| CoreSessionState::WaitingUserWithTimeout { timeout })
            .unwrap_or(CoreSessionState::WaitingUser);
        CoreTopicEvent::new(
            session_id,
            CoreTopic::new(
                self.topic_name(),
                json!({
                    "name": self.topic_name(),
                    "kind": self.kind(),
                    "expects_reply": true,
                }),
            ),
            state,
            json!({
                "kind": self.kind(),
                "request_id": request_id,
                "safe_default": self.safe_default().name(),
                "timeout_ms": self.timeout().map(|timeout| timeout.as_millis()),
                "request": decision_request_payload(self),
            }),
        )
    }
}

impl HostDecisionDefault {
    pub fn name(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Decline => "decline",
        }
    }
}

pub(crate) fn notification_topic_events(
    session_id: &str,
    notifications: &[CoreNotification],
) -> Vec<CoreTopicEvent> {
    notifications
        .iter()
        .map(|notification| notification_topic_event(session_id, notification))
        .collect()
}

pub(crate) fn notification_topic_event(
    session_id: &str,
    notification: &CoreNotification,
) -> CoreTopicEvent {
    match notification {
        CoreNotification::ModelResponse {
            status,
            free_talk,
            report_job_progress,
            final_answer,
            continue_work,
        } => {
            let direct_turn_worker_count = if *continue_work { 1 } else { 0 };
            CoreTopicEvent::new(
                session_id,
                CoreTopic::new(
                    CORE_TOPIC_MODEL_RESPONSE,
                    json!({
                        "name": CORE_TOPIC_MODEL_RESPONSE,
                    }),
                ),
                CoreSessionState::Running,
                json!({
                    "status": status,
                    "free_talk": free_talk,
                    "report_job_progress": report_job_progress,
                    "final_answer": final_answer,
                    "continue_work": continue_work,
                    "global": global_worker_status_payload(CoreGlobalWorkerStatus::new(direct_turn_worker_count)),
                }),
            )
        }
        CoreNotification::Action {
            intent,
            action,
            input,
            kind,
            active,
            memory_activity,
        } => CoreTopicEvent::new(
            session_id,
            CoreTopic::new(
                CORE_TOPIC_ACTION,
                json!({
                    "name": CORE_TOPIC_ACTION,
                    "action": action,
                    "active": active,
                }),
            ),
            CoreSessionState::Running,
            json!({
                "intent": intent,
                "action": action,
                "input": input,
                "kind": action_kind_topic_payload(kind),
                "active": active,
                "memory_activity": memory_activity,
            }),
        ),
    }
}

pub fn topic_event_status_hint(events: &[CoreTopicEvent]) -> Option<CoreTopicStatusHint> {
    events.iter().find_map(|event| {
        let action_topic = event.as_action()?;
        if action_topic.intent.is_none() && action_topic.action.trim().is_empty() {
            return None;
        }
        Some(CoreTopicStatusHint {
            intent: action_topic.intent,
            action: action_topic.action,
            input: action_topic.input,
            memory_activity: action_topic.memory_activity,
        })
    })
}

fn action_kind_topic_payload(kind: &CoreActionKind) -> Value {
    match kind {
        CoreActionKind::Bash {
            command,
            mode,
            interval_ms,
            timeout_ms,
        } => json!({
            "kind": "bash",
            "command": command,
            "mode": mode,
            "interval_ms": interval_ms,
            "timeout_ms": timeout_ms,
        }),
        CoreActionKind::ShellJob { job_id } => json!({
            "kind": "shell_job",
            "job_id": job_id,
        }),
        CoreActionKind::Memory { surface, operation } => json!({
            "kind": "memory",
            "surface": surface,
            "operation": operation,
        }),
        CoreActionKind::Capability { op, kind, id } => json!({
            "kind": "capability",
            "op": op,
            "capability_kind": kind,
            "id": id,
        }),
        CoreActionKind::SelfTool { self_type, op } => json!({
            "kind": "self_tool",
            "self_type": self_type,
            "op": op,
        }),
        CoreActionKind::ChatHistory { operation } => json!({
            "kind": "chat_history",
            "operation": operation,
        }),
        CoreActionKind::Other { action } => json!({
            "kind": "other",
            "action": action,
        }),
    }
}

fn action_kind_from_topic_payload(value: &Value, fallback_action: &str) -> CoreActionKind {
    match value["kind"].as_str().unwrap_or_default() {
        "bash" => CoreActionKind::Bash {
            command: value["command"].as_str().unwrap_or_default().to_string(),
            mode: value["mode"].as_str().unwrap_or("foreground").to_string(),
            interval_ms: value["interval_ms"].as_u64(),
            timeout_ms: value["timeout_ms"].as_i64(),
        },
        "shell_job" => CoreActionKind::ShellJob {
            job_id: value["job_id"].as_str().unwrap_or_default().to_string(),
        },
        "memory" => CoreActionKind::Memory {
            surface: value["surface"].as_str().unwrap_or_default().to_string(),
            operation: value["operation"].as_str().unwrap_or_default().to_string(),
        },
        "capability" => CoreActionKind::Capability {
            op: value["op"].as_str().unwrap_or_default().to_string(),
            kind: value["capability_kind"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            id: value["id"].as_str().unwrap_or_default().to_string(),
        },
        "self_tool" => CoreActionKind::SelfTool {
            self_type: value["self_type"].as_str().unwrap_or_default().to_string(),
            op: value["op"].as_str().unwrap_or_default().to_string(),
        },
        "chat_history" => CoreActionKind::ChatHistory {
            operation: value["operation"].as_str().unwrap_or_default().to_string(),
        },
        "other" => CoreActionKind::Other {
            action: value["action"]
                .as_str()
                .unwrap_or(fallback_action)
                .to_string(),
        },
        _ => CoreActionKind::Other {
            action: fallback_action.to_string(),
        },
    }
}

fn memory_activity_from_topic_payload(value: &Value) -> CoreMemoryActivity {
    match value.as_str().unwrap_or_default() {
        "read" => CoreMemoryActivity::Read,
        "write" => CoreMemoryActivity::Write,
        _ => CoreMemoryActivity::None,
    }
}

pub trait CoreTopicEventSink {
    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]);
}

impl<F> CoreTopicEventSink for F
where
    F: FnMut(&[CoreTopicEvent]),
{
    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        self(events);
    }
}

fn decision_request_payload(request: &HostDecisionRequest) -> Value {
    match request {
        HostDecisionRequest::UserApproval(approval) => json!({
            "approval_id": approval.approval_id,
            "action": approval.action,
            "command": approval.command,
            "reason": approval.reason,
            "risk": approval.risk,
            "intent": approval.intent,
        }),
        HostDecisionRequest::RoundLimitContinue(round) => json!({
            "max_rounds": round.max_rounds,
            "recharge_rounds": round.recharge_rounds,
            "keep_task_context": round.keep_task_context,
        }),
        HostDecisionRequest::OutputExpansion(expansion) => json!({
            "current_tokens": expansion.current_tokens,
            "increment_tokens": expansion.increment_tokens,
            "expanded_tokens": expansion.expanded_tokens(),
            "retry_same_turn": expansion.retry_same_turn,
        }),
        HostDecisionRequest::StaleContextContinue(stale) => json!({
            "idle_ms": stale.idle.as_millis(),
            "dynamic_context_tokens": stale.dynamic_context_tokens,
            "continue_keeps_dynamic_context": stale.continue_keeps_dynamic_context,
            "decline_clears_dynamic_context": stale.decline_clears_dynamic_context,
        }),
        HostDecisionRequest::WorkInstructionLoad(work) => json!({
            "directory": work.directory.display().to_string(),
            "file_names": work.file_names,
        }),
    }
}

fn host_decision_default_from_name(value: &str) -> Option<HostDecisionDefault> {
    match value {
        "accept" => Some(HostDecisionDefault::Accept),
        "decline" => Some(HostDecisionDefault::Decline),
        _ => None,
    }
}

fn host_decision_request_from_payload(kind: &str, payload: &Value) -> Option<HostDecisionRequest> {
    match kind {
        "user_approval" => Some(HostDecisionRequest::UserApproval(ApprovalRequest {
            approval_id: payload["approval_id"].as_str()?.to_string(),
            action: payload["action"].as_str()?.to_string(),
            command: payload["command"].as_str()?.to_string(),
            reason: payload["reason"].as_str()?.to_string(),
            risk: payload["risk"].as_str()?.to_string(),
            intent: payload["intent"].as_str()?.to_string(),
        })),
        "round_limit_continue" => Some(HostDecisionRequest::RoundLimitContinue(
            RoundLimitDecisionRequest {
                max_rounds: payload["max_rounds"].as_u64()? as u32,
                recharge_rounds: payload["recharge_rounds"].as_u64()? as u32,
                keep_task_context: payload["keep_task_context"].as_bool()?,
            },
        )),
        "output_expansion" => Some(HostDecisionRequest::OutputExpansion(
            OutputExpansionRequest {
                current_tokens: payload["current_tokens"].as_u64()? as u32,
                increment_tokens: payload["increment_tokens"].as_u64()? as u32,
                retry_same_turn: payload["retry_same_turn"].as_bool()?,
            },
        )),
        "stale_context_continue" => Some(HostDecisionRequest::StaleContextContinue(
            StaleContextDecisionRequest {
                idle: Duration::from_millis(payload["idle_ms"].as_u64()?),
                dynamic_context_tokens: payload["dynamic_context_tokens"].as_u64()? as u32,
                continue_keeps_dynamic_context: payload["continue_keeps_dynamic_context"]
                    .as_bool()?,
                decline_clears_dynamic_context: payload["decline_clears_dynamic_context"]
                    .as_bool()?,
            },
        )),
        "work_instruction_load" => Some(HostDecisionRequest::WorkInstructionLoad(
            WorkInstructionLoadRequest {
                directory: payload["directory"].as_str()?.into(),
                file_names: payload["file_names"]
                    .as_array()?
                    .iter()
                    .map(|item| item.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()?,
            },
        )),
        _ => None,
    }
}

pub trait TurnUi {
    fn is_cancel_requested(&mut self) -> bool {
        false
    }

    fn take_cancel_request(&mut self) -> bool {
        self.is_cancel_requested()
    }

    fn drain_user_supplements(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn on_model_request(&mut self, _round: u32, _prompt: &str) {}

    fn on_model_response(&mut self, _round: u32, _usage: &UsageStats, _content: &str) {}

    fn on_model_response_discarded(&mut self, _round: u32, _reason: &str) {}

    fn on_core_topic_events(&mut self, _events: &[CoreTopicEvent]) {}

    fn on_model_error(&mut self, _error: &str) {}

    fn on_model_retry(
        &mut self,
        _attempt: u32,
        _max_attempts: u32,
        _delay: Duration,
        _error: &str,
    ) {
    }

    fn pause_for_user_decision(&mut self) {}

    fn resume_after_user_decision(&mut self) {}

    fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
        request.safe_default().into()
    }

    fn reply_to_core_topic(&mut self, event: &CoreTopicEvent) -> Option<TopicReply> {
        let topic = event.as_host_decision_request()?;
        let decision = self.request_host_decision(topic.request);
        TopicReply::for_decision_request(event, decision)
    }

    fn request_host_decision_topic(
        &mut self,
        session: &str,
        request: HostDecisionRequest,
    ) -> HostDecision {
        let event = request.topic_event(session);
        self.on_core_topic_events(std::slice::from_ref(&event));
        let Some(reply) = self.reply_to_core_topic(&event) else {
            return event
                .as_host_decision_request()
                .map(|topic| topic.safe_default.into())
                .unwrap_or_else(|| request.safe_default().into());
        };
        resolve_topic_reply(&event, None, &reply).unwrap_or_else(|_| {
            event
                .as_host_decision_request()
                .map(|topic| topic.safe_default.into())
                .unwrap_or_else(|| request.safe_default().into())
        })
    }

    fn request_user_approval(&mut self, request: &ApprovalRequest) -> bool {
        self.request_host_decision(HostDecisionRequest::UserApproval(request.clone()))
            .as_bool()
    }

    fn request_round_limit_continue(&mut self, request: RoundLimitDecisionRequest) -> bool {
        self.request_host_decision(HostDecisionRequest::RoundLimitContinue(request))
            .as_bool()
    }

    fn can_request_output_expansion(&mut self) -> bool {
        HostDecisionDefault::Accept.as_bool()
    }

    fn request_expand_output_tokens(&mut self, request: OutputExpansionRequest) -> bool {
        self.request_host_decision(HostDecisionRequest::OutputExpansion(request))
            .as_bool()
    }
}

pub fn normalize_user_supplements(supplements: Vec<String>) -> Vec<String> {
    supplements
        .into_iter()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

pub struct NoopTurnUi;

impl TurnUi for NoopTurnUi {}

fn next_topic_request_id(kind: &str) -> String {
    let seq = TOPIC_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("request_{kind}_{seq}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_turn_ui_defaults_are_noninteractive() {
        let mut ui = NoopTurnUi;
        assert!(!ui.is_cancel_requested());
        assert!(!ui.take_cancel_request());
        assert!(ui.drain_user_supplements().is_empty());
        assert!(ui.request_round_limit_continue(RoundLimitDecisionRequest::new(50)));
        assert!(ui.can_request_output_expansion());
        assert!(ui.request_expand_output_tokens(OutputExpansionRequest::new(10_000)));
    }

    #[test]
    fn turn_ui_default_callbacks_follow_host_decision_policy() {
        let mut ui = NoopTurnUi;
        let approval = ApprovalRequest {
            approval_id: "approval_1".to_string(),
            action: "run_bash".to_string(),
            command: "printf ok".to_string(),
            reason: "requires_approval".to_string(),
            risk: "local_command".to_string(),
            intent: "Check local evidence.".to_string(),
        };
        let round = RoundLimitDecisionRequest::new(20);
        let expansion = OutputExpansionRequest::new(10_000);

        assert_eq!(
            ui.request_user_approval(&approval),
            HostDecisionRequest::UserApproval(approval)
                .safe_default()
                .as_bool()
        );
        assert_eq!(
            ui.request_round_limit_continue(round),
            HostDecisionRequest::RoundLimitContinue(round)
                .safe_default()
                .as_bool()
        );
        assert_eq!(
            ui.request_expand_output_tokens(expansion),
            HostDecisionRequest::OutputExpansion(expansion)
                .safe_default()
                .as_bool()
        );
    }

    #[test]
    fn turn_ui_specific_requests_delegate_to_generic_host_decision() {
        #[derive(Default)]
        struct DeclineAll {
            seen: Vec<&'static str>,
        }

        impl TurnUi for DeclineAll {
            fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
                self.seen.push(request.kind());
                HostDecision::Decline
            }
        }

        let mut ui = DeclineAll::default();
        let approval = ApprovalRequest {
            approval_id: "approval_1".to_string(),
            action: "run_bash".to_string(),
            command: "printf ok".to_string(),
            reason: "requires_approval".to_string(),
            risk: "local_command".to_string(),
            intent: "Check local evidence.".to_string(),
        };

        assert!(!ui.request_user_approval(&approval));
        assert!(!ui.request_round_limit_continue(RoundLimitDecisionRequest::new(20)));
        assert!(!ui.request_expand_output_tokens(OutputExpansionRequest::new(10_000)));
        assert_eq!(
            ui.seen,
            vec!["user_approval", "round_limit_continue", "output_expansion"]
        );
    }

    #[test]
    fn turn_ui_request_topic_emits_blocking_event_and_resolves_reply() {
        #[derive(Default)]
        struct TopicAwareUi {
            seen_topics: Vec<String>,
            seen_blocking: Vec<bool>,
            seen_request_ids: Vec<String>,
        }

        impl TurnUi for TopicAwareUi {
            fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
                for event in events {
                    self.seen_topics.push(event.topic.name.clone());
                    self.seen_blocking.push(event.is_blocking_request());
                    if let Some(request_id) = event.request_id() {
                        self.seen_request_ids.push(request_id.to_string());
                    }
                }
            }

            fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
                assert_eq!(request.kind(), "round_limit_continue");
                HostDecision::Decline
            }
        }

        let mut ui = TopicAwareUi::default();
        let decision = ui.request_host_decision_topic(
            "session_a",
            HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
        );

        assert_eq!(decision, HostDecision::Decline);
        assert_eq!(
            ui.seen_topics,
            vec![CORE_TOPIC_ROUND_LIMIT_REQUEST.to_string()]
        );
        assert_eq!(ui.seen_blocking, vec![true]);
        assert_eq!(ui.seen_request_ids.len(), 1);
        assert!(ui.seen_request_ids[0].starts_with("request_round_limit_continue_"));
    }

    #[test]
    fn turn_ui_request_topic_requires_matching_topic_reply_before_resuming() {
        #[derive(Default)]
        struct BadReplyUi {
            seen_event: Option<CoreTopicEvent>,
        }

        impl TurnUi for BadReplyUi {
            fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
                self.seen_event = events.first().cloned();
            }

            fn reply_to_core_topic(&mut self, event: &CoreTopicEvent) -> Option<TopicReply> {
                TopicReply::for_decision_request(event, HostDecision::Decline)
                    .map(|reply| reply.with_request_id("wrong_request_id"))
            }
        }

        let mut ui = BadReplyUi::default();
        let decision = ui.request_host_decision_topic(
            "session_a",
            HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
        );

        assert_eq!(
            decision,
            HostDecision::Accept,
            "mismatched replies must fall back to the request safe default"
        );
        let event = ui.seen_event.expect("request topic should be published");
        assert!(event.is_blocking_request());
        assert_eq!(event.session_id, "session_a");
        assert_eq!(event.topic.name, CORE_TOPIC_ROUND_LIMIT_REQUEST);
        assert!(event.request_id().is_some());
    }

    #[test]
    fn turn_ui_decision_requests_are_structured_and_ui_neutral() {
        let round = RoundLimitDecisionRequest::new(20);
        assert_eq!(round.max_rounds, 20);
        assert_eq!(round.recharge_rounds, 20);
        assert!(round.keep_task_context);

        let output = OutputExpansionRequest::new(10_000);
        assert_eq!(output.current_tokens, 10_000);
        assert_eq!(output.increment_tokens, 10_000);
        assert_eq!(output.expanded_tokens(), 20_000);
        assert!(output.retry_same_turn);

        let debug = format!("{round:?} {output:?}");
        for forbidden in ["继续", "停止", "增加", "重试", "[", "\x1b"] {
            assert!(
                !debug.contains(forbidden),
                "core decision request leaked shell/ui text {forbidden:?}: {debug}"
            );
        }
    }

    #[test]
    fn host_decision_request_exposes_ui_neutral_policy_metadata() {
        let requests = [
            HostDecisionRequest::UserApproval(ApprovalRequest {
                approval_id: "approval_1".to_string(),
                action: "run_bash".to_string(),
                command: "printf ok".to_string(),
                reason: "requires_approval".to_string(),
                risk: "local_command".to_string(),
                intent: "Check local evidence.".to_string(),
            }),
            HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
            HostDecisionRequest::OutputExpansion(OutputExpansionRequest::new(10_000)),
            HostDecisionRequest::StaleContextContinue(StaleContextDecisionRequest {
                idle: Duration::from_secs(3 * 60 * 60),
                dynamic_context_tokens: 10_001,
                continue_keeps_dynamic_context: true,
                decline_clears_dynamic_context: true,
            }),
            HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
                directory: "/tmp/project".into(),
                file_names: vec!["AGENTS.md".to_string()],
            }),
        ];

        assert_eq!(requests[0].kind(), "user_approval");
        assert_eq!(requests[1].kind(), "round_limit_continue");
        assert_eq!(requests[2].kind(), "output_expansion");
        assert_eq!(requests[3].kind(), "stale_context_continue");
        assert_eq!(requests[4].kind(), "work_instruction_load");
        assert!(requests[..4]
            .iter()
            .all(|request| request.timeout().is_none()));
        assert_eq!(
            requests[4].timeout(),
            Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT)
        );
        assert_eq!(requests[0].safe_default(), HostDecisionDefault::Accept);
        assert_eq!(requests[1].safe_default(), HostDecisionDefault::Accept);
        assert_eq!(requests[2].safe_default(), HostDecisionDefault::Accept);
        assert_eq!(requests[3].safe_default(), HostDecisionDefault::Decline);
        assert_eq!(requests[4].safe_default(), HostDecisionDefault::Decline);

        let debug = format!("{requests:?}");
        for forbidden in ["继续", "停止", "加载", "\x1b"] {
            assert!(
                !debug.contains(forbidden),
                "core host request metadata leaked UI text {forbidden:?}: {debug}"
            );
        }
    }

    #[test]
    fn host_decision_requests_can_be_published_as_topic_events() {
        let request = HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
            directory: "/tmp/project".into(),
            file_names: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
        });

        let event = request.topic_event_with_request_id("session_a", "request_1");
        assert_eq!(event.session_id, "session_a");
        assert_eq!(event.topic.name, "core.work_instruction_load");
        assert_eq!(event.topic.attributes["name"], event.topic.name);
        assert_eq!(event.topic.attributes["expects_reply"], true);
        assert_eq!(event.request_id(), Some("request_1"));
        assert!(event.expects_reply());
        assert!(event.is_blocking_request());
        assert_eq!(event.state.name(), "waiting_user_with_timeout");
        assert_eq!(
            event.state.timeout_ms(),
            Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis())
        );
        assert_eq!(event.state_payload()["name"], "waiting_user_with_timeout");
        assert_eq!(
            event.state_payload()["timeout_ms"].as_u64(),
            Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis() as u64)
        );
        assert_eq!(event.payload["kind"], "work_instruction_load");
        assert_eq!(event.payload["request_id"], "request_1");
        assert_eq!(event.payload["safe_default"], "decline");
        assert_eq!(
            event.payload["timeout_ms"].as_u64(),
            Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis() as u64)
        );
        assert_eq!(event.payload["request"]["directory"], "/tmp/project");
        assert_eq!(event.payload["request"]["file_names"][0], "AGENTS.md");
        assert_eq!(
            event.wire_payload(),
            json!({
                "session_id": "session_a",
                "topic": {
                    "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                    "attributes": {
                        "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                        "kind": "work_instruction_load",
                        "expects_reply": true,
                    },
                },
                "state": {
                    "name": "waiting_user_with_timeout",
                    "timeout_ms": DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis(),
                },
                "payload": {
                    "kind": "work_instruction_load",
                    "request_id": "request_1",
                    "safe_default": "decline",
                    "timeout_ms": DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis(),
                    "request": {
                        "directory": "/tmp/project",
                        "file_names": ["AGENTS.md", "CLAUDE.md"],
                    },
                },
            })
        );
        assert_eq!(
            event.as_host_decision_request(),
            Some(CoreHostDecisionRequestTopic {
                session_id: "session_a".to_string(),
                kind: "work_instruction_load",
                state: CoreSessionState::WaitingUserWithTimeout {
                    timeout: DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT,
                },
                safe_default: HostDecisionDefault::Decline,
                timeout: Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT),
                request,
            })
        );
    }

    #[test]
    fn work_instruction_load_status_can_be_published_as_topic_event() {
        let report = WorkInstructionLoadReport {
            status: WorkInstructionLoadStatus::Loaded,
            directory: "/tmp/project".into(),
            file_names: vec!["AGENTS.md".to_string()],
            context: Some("guide".to_string()),
            error: None,
        };

        let event = work_instruction_load_topic_event("session_a", &report);
        assert_eq!(event.session_id, "session_a");
        assert_eq!(event.topic.name, CORE_TOPIC_WORK_INSTRUCTION_LOAD);
        assert_eq!(event.state, CoreSessionState::Running);
        assert!(!event.expects_reply());
        assert_eq!(
            event.wire_payload(),
            json!({
                "session_id": "session_a",
                "topic": {
                    "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                    "attributes": {
                        "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                    },
                },
                "state": {
                    "name": "running",
                },
                "payload": {
                    "status": "loaded",
                    "directory": "/tmp/project",
                    "file_names": ["AGENTS.md"],
                    "error": null,
                },
            })
        );
        assert_eq!(
            event.as_work_instruction_load(),
            Some(CoreWorkInstructionLoadTopic {
                status: "loaded".to_string(),
                directory: "/tmp/project".to_string(),
                file_names: vec!["AGENTS.md".to_string()],
                error: None,
            })
        );
    }

    #[test]
    fn host_decision_request_topic_accessor_round_trips_all_request_kinds() {
        let requests = [
            HostDecisionRequest::UserApproval(ApprovalRequest {
                approval_id: "approval_1".to_string(),
                action: "run_bash".to_string(),
                command: "printf ok".to_string(),
                reason: "requires_approval".to_string(),
                risk: "local_command".to_string(),
                intent: "Check local evidence.".to_string(),
            }),
            HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
            HostDecisionRequest::OutputExpansion(OutputExpansionRequest::new(10_000)),
            HostDecisionRequest::StaleContextContinue(StaleContextDecisionRequest {
                idle: Duration::from_secs(3 * 60 * 60),
                dynamic_context_tokens: 10_001,
                continue_keeps_dynamic_context: true,
                decline_clears_dynamic_context: true,
            }),
            HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
                directory: "/tmp/project".into(),
                file_names: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
            }),
        ];

        for request in requests {
            let request_id = format!("request_{}", request.kind());
            let event = request.topic_event_with_request_id("session_a", &request_id);
            let topic = event
                .as_host_decision_request()
                .expect("host decision request topic should decode");
            assert!(event.expects_reply());
            assert!(event.is_blocking_request());
            assert_eq!(event.request_id(), Some(request_id.as_str()));
            assert_eq!(topic.session_id, "session_a");
            assert_eq!(topic.kind, request.kind());
            assert_eq!(
                topic.state.name(),
                if request.timeout().is_some() {
                    "waiting_user_with_timeout"
                } else {
                    "waiting_user"
                }
            );
            assert_eq!(topic.safe_default, request.safe_default());
            assert_eq!(topic.timeout, request.timeout());
            assert_eq!(topic.request, request);
        }
    }

    #[test]
    fn topic_reply_correlates_to_blocking_request_topic() {
        let request = HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
            directory: "/tmp/project".into(),
            file_names: vec!["AGENTS.md".to_string()],
        });
        let event = request.topic_event_with_request_id("session_a", "request_1");

        let reply = TopicReply::for_decision_request(&event, HostDecision::Accept)
            .expect("blocking request should accept a topic reply");

        assert_eq!(reply.session_id, "session_a");
        assert_eq!(reply.topic_name, CORE_TOPIC_WORK_INSTRUCTION_LOAD);
        assert_eq!(reply.request_id.as_deref(), Some("request_1"));
        assert_eq!(reply.decision, HostDecision::Accept);
        assert_eq!(
            reply.wire_payload(),
            json!({
                "session_id": "session_a",
                "topic_name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                "request_id": "request_1",
                "decision": "accept",
                "payload": {
                    "decision": "accept",
                },
            })
        );

        let progress = notification_topic_event(
            "session_a",
            &CoreNotification::ModelResponse {
                status: "working".to_string(),
                free_talk: String::new(),
                report_job_progress: "not waiting".to_string(),
                final_answer: String::new(),
                continue_work: true,
            },
        );
        assert!(TopicReply::for_decision_request(&progress, HostDecision::Accept).is_none());
    }

    #[test]
    fn topic_reply_resolution_validates_session_topic_and_request_id() {
        let request = HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20));
        let event = request.topic_event_with_request_id("session_a", "request_1");
        let reply = TopicReply::for_decision_request(&event, HostDecision::Decline)
            .expect("blocking request should produce reply")
            .with_request_id("request_1");

        assert_eq!(
            resolve_topic_reply(&event, Some("request_1"), &reply),
            Ok(HostDecision::Decline)
        );

        let mut wrong_session = reply.clone();
        wrong_session.session_id = "session_b".to_string();
        assert_eq!(
            resolve_topic_reply(&event, Some("request_1"), &wrong_session),
            Err(TopicReplyError::SessionMismatch)
        );

        let mut wrong_topic = reply.clone();
        wrong_topic.topic_name = CORE_TOPIC_ACTION.to_string();
        assert_eq!(
            resolve_topic_reply(&event, Some("request_1"), &wrong_topic),
            Err(TopicReplyError::TopicMismatch)
        );

        let mut wrong_request_id = reply.clone();
        wrong_request_id.request_id = Some("request_2".to_string());
        assert_eq!(
            resolve_topic_reply(&event, Some("request_1"), &wrong_request_id),
            Err(TopicReplyError::RequestIdMismatch)
        );

        let progress = notification_topic_event(
            "session_a",
            &CoreNotification::ModelResponse {
                status: "working".to_string(),
                free_talk: String::new(),
                report_job_progress: "not waiting".to_string(),
                final_answer: String::new(),
                continue_work: true,
            },
        );
        assert_eq!(
            resolve_topic_reply(&progress, None, &reply),
            Err(TopicReplyError::NotBlockingRequest)
        );
    }

    #[test]
    fn core_notifications_can_be_published_as_topic_events() {
        let notifications = vec![
            CoreNotification::ModelResponse {
                status: "working".to_string(),
                free_talk: "planning next step".to_string(),
                report_job_progress: "checking context".to_string(),
                final_answer: String::new(),
                continue_work: true,
            },
            CoreNotification::Action {
                intent: Some("Inspect local files.".to_string()),
                action: "run_bash".to_string(),
                input: serde_json::json!({"cmd": "pwd"}),
                kind: crate::CoreActionKind::Bash {
                    command: "pwd".to_string(),
                    mode: "foreground".to_string(),
                    interval_ms: None,
                    timeout_ms: None,
                },
                active: true,
                memory_activity: crate::CoreMemoryActivity::None,
            },
        ];

        let events = notification_topic_events("session_a", &notifications);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].session_id, "session_a");
        assert_eq!(events[0].topic.name, CORE_TOPIC_MODEL_RESPONSE);
        assert_eq!(events[0].state, CoreSessionState::Running);
        assert!(!events[0].expects_reply());
        assert!(!events[0].is_blocking_request());
        assert_eq!(
            events[0].wire_payload(),
            json!({
                "session_id": "session_a",
                "topic": {
                    "name": CORE_TOPIC_MODEL_RESPONSE,
                    "attributes": {
                        "name": CORE_TOPIC_MODEL_RESPONSE,
                    },
                },
                "state": {
                    "name": "running",
                },
                "payload": {
                    "status": "working",
                    "free_talk": "planning next step",
                    "report_job_progress": "checking context",
                    "final_answer": "",
                    "continue_work": true,
                    "global": {
                        "working_worker_count": 1,
                    },
                },
            })
        );
        assert!(events[0].payload.get("text").is_none());
        assert_eq!(
            events[0].as_model_response(),
            Some(CoreModelResponseTopic {
                status: "working".to_string(),
                free_talk: "planning next step".to_string(),
                report_job_progress: "checking context".to_string(),
                final_answer: String::new(),
                continue_work: true,
                global: CoreGlobalWorkerStatus::new(1),
            })
        );

        assert_eq!(events[1].topic.name, CORE_TOPIC_ACTION);
        assert_eq!(events[1].topic.attributes["action"], "run_bash");
        assert!(!events[1].expects_reply());
        assert!(!events[1].is_blocking_request());
        assert_eq!(
            events[1].wire_payload(),
            json!({
                "session_id": "session_a",
                "topic": {
                    "name": CORE_TOPIC_ACTION,
                    "attributes": {
                        "name": CORE_TOPIC_ACTION,
                        "action": "run_bash",
                        "active": true,
                    },
                },
                "state": {
                    "name": "running",
                },
                "payload": {
                    "intent": "Inspect local files.",
                    "action": "run_bash",
                    "input": {
                        "cmd": "pwd",
                    },
                    "kind": {
                        "kind": "bash",
                        "command": "pwd",
                        "mode": "foreground",
                        "interval_ms": null,
                        "timeout_ms": null,
                    },
                    "active": true,
                    "memory_activity": "none",
                },
            })
        );
        assert_eq!(
            events[1].as_action(),
            Some(CoreActionTopic {
                intent: Some("Inspect local files.".to_string()),
                action: "run_bash".to_string(),
                input: serde_json::json!({"cmd": "pwd"}),
                kind: CoreActionKind::Bash {
                    command: "pwd".to_string(),
                    mode: "foreground".to_string(),
                    interval_ms: None,
                    timeout_ms: None,
                },
                active: true,
                memory_activity: CoreMemoryActivity::None,
            })
        );
        assert_eq!(
            topic_event_status_hint(&events),
            Some(CoreTopicStatusHint {
                intent: Some("Inspect local files.".to_string()),
                action: "run_bash".to_string(),
                input: serde_json::json!({"cmd": "pwd"}),
                memory_activity: CoreMemoryActivity::None,
            })
        );
    }

    #[test]
    fn core_init_lifecycle_topic_is_structured_and_ui_neutral() {
        let profile = CoreProfile {
            name: "test".to_string(),
            provider: "aliyun".to_string(),
            model: "qwen-plus".to_string(),
        };

        let event =
            core_initialized_topic_event("session_a", &profile, "markdown", 100_000, 50, 6, 2);

        assert_eq!(event.session_id, "session_a");
        assert_eq!(event.topic.name, CORE_TOPIC_LIFECYCLE);
        assert_eq!(event.topic.attributes["name"], CORE_TOPIC_LIFECYCLE);
        assert_eq!(event.topic.attributes["event"], "initialized");
        assert_eq!(event.state, CoreSessionState::Running);
        assert!(!event.expects_reply());
        assert!(!event.is_blocking_request());
        assert_eq!(
            event.as_lifecycle(),
            Some(CoreLifecycleTopic {
                event: CoreLifecycleEvent::Initialized,
                version: env!("CARGO_PKG_VERSION").to_string(),
                profile,
                response_protocol: "markdown".to_string(),
                max_llm_input_tokens: 100_000,
                max_rounds: 50,
                tool_count: 6,
                skill_count: 2,
                worker: None,
                workspace: None,
                context: None,
            })
        );
        assert_eq!(
            event.wire_payload(),
            json!({
                "session_id": "session_a",
                "topic": {
                    "name": CORE_TOPIC_LIFECYCLE,
                    "attributes": {
                        "name": CORE_TOPIC_LIFECYCLE,
                        "event": "initialized",
                    },
                },
                "state": {
                    "name": "running",
                },
                "payload": {
                    "event": "initialized",
                    "version": env!("CARGO_PKG_VERSION"),
                    "profile": {
                        "name": "test",
                        "provider": "aliyun",
                        "model": "qwen-plus",
                    },
                    "response_protocol": "markdown",
                    "max_llm_input_tokens": 100000,
                    "max_rounds": 50,
                    "capabilities": {
                        "tools": 6,
                        "skills": 2,
                    },
                    "worker": null,
                    "workspace": null,
                    "context": null,
                },
            })
        );
        let debug = format!("{event:?}");
        for forbidden in ["启动成功", "ⓘ", "\x1b"] {
            assert!(
                !debug.contains(forbidden),
                "core lifecycle topic leaked shell rendering {forbidden:?}: {debug}"
            );
        }
    }

    #[test]
    fn core_lifecycle_topic_round_trips_worker_identity_workspace_and_context() {
        let profile = CoreProfile {
            name: "test".to_string(),
            provider: "local".to_string(),
            model: "fake".to_string(),
        };
        let identity = CoreSessionWorkerIdentity::new(
            "session_child",
            2,
            Some("日志分析".to_string()),
            Some("session_parent".to_string()),
        );
        let mut workspace = CoreSessionWorkerWorkspace::new(
            "/tmp/timem-data",
            "/tmp/timem-data/audit/api_audit.json",
            "timem_native_shell",
            "user_local_machine",
        );
        workspace.current_dir = Some(PathBuf::from("/tmp/project"));
        workspace
            .env
            .insert("TIMEM_GATEWAY_PROVIDER".to_string(), "local".to_string());
        workspace.env.insert(
            "TIMEM_API_KEY".to_string(),
            "sk-lifecycle-secret".to_string(),
        );
        workspace.workspace_dirs.push(PathBuf::from("/tmp/project"));
        let mut expected_workspace = workspace.clone();
        expected_workspace.env.insert(
            "TIMEM_API_KEY".to_string(),
            crate::redaction::REDACTED.to_string(),
        );
        let context = CoreDynamicContextSummary {
            visible_delta_count: 3,
            visible_slice_count: 5,
            estimated_tokens: 2048,
        };

        let event = core_initialized_topic_event_with_worker(
            "session_child",
            &profile,
            "markdown",
            100_000,
            50,
            6,
            0,
            Some(&identity),
            Some(&workspace),
            Some(context),
        );
        let lifecycle = event.as_lifecycle().expect("lifecycle should parse");

        assert_eq!(lifecycle.worker, Some(identity));
        assert_eq!(lifecycle.workspace, Some(expected_workspace));
        assert_eq!(lifecycle.context, Some(context));
        assert_eq!(event.payload["worker"]["display_name"], "日志分析");
        assert_eq!(event.payload["worker"]["ordinal"], 2);
        assert_eq!(event.payload["context"]["visible_delta_count"], 3);
        assert_eq!(
            event.payload["workspace"]["env"]["TIMEM_API_KEY"],
            crate::redaction::REDACTED
        );
        assert_eq!(
            event.payload["workspace"]["env"]["TIMEM_GATEWAY_PROVIDER"],
            "local"
        );
        assert!(
            !event
                .wire_payload()
                .to_string()
                .contains("sk-lifecycle-secret"),
            "lifecycle topic must not leak env secrets"
        );
    }

    #[test]
    fn topic_callbacks_can_copy_owned_snapshots_for_async_hosts() {
        let notifications = vec![CoreNotification::Action {
            intent: Some("Inspect local files.".to_string()),
            action: "run_bash".to_string(),
            input: serde_json::json!({"cmd": "pwd"}),
            kind: CoreActionKind::Bash {
                command: "pwd".to_string(),
                mode: "foreground".to_string(),
                interval_ms: None,
                timeout_ms: None,
            },
            active: true,
            memory_activity: CoreMemoryActivity::None,
        }];

        let mut queued: Vec<CoreTopicEvent> = Vec::new();
        {
            let mut sink = |events: &[CoreTopicEvent]| {
                queued.extend_from_slice(events);
            };
            let events = notification_topic_events("session_a", &notifications);
            sink(&events);
        }
        drop(notifications);

        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].session_id, "session_a");
        assert_eq!(queued[0].as_action().unwrap().input["cmd"], "pwd");
    }

    #[test]
    fn action_topic_kind_wire_payload_is_explicit_and_round_trips() {
        let cases = [
            (
                CoreActionKind::Bash {
                    command: "pwd".to_string(),
                    mode: "foreground".to_string(),
                    interval_ms: None,
                    timeout_ms: None,
                },
                json!({"kind": "bash", "command": "pwd", "mode": "foreground", "interval_ms": null, "timeout_ms": null}),
            ),
            (
                CoreActionKind::Bash {
                    command: "gh run list".to_string(),
                    mode: "poll".to_string(),
                    interval_ms: Some(5000),
                    timeout_ms: None,
                },
                json!({"kind": "bash", "command": "gh run list", "mode": "poll", "interval_ms": 5000, "timeout_ms": null}),
            ),
            (
                CoreActionKind::ShellJob {
                    job_id: "job_1".to_string(),
                },
                json!({"kind": "shell_job", "job_id": "job_1"}),
            ),
            (
                CoreActionKind::Memory {
                    surface: "scratch".to_string(),
                    operation: "read".to_string(),
                },
                json!({"kind": "memory", "surface": "scratch", "operation": "read"}),
            ),
            (
                CoreActionKind::Capability {
                    op: "load".to_string(),
                    kind: "skill".to_string(),
                    id: "release".to_string(),
                },
                json!({"kind": "capability", "op": "load", "capability_kind": "skill", "id": "release"}),
            ),
            (
                CoreActionKind::SelfTool {
                    self_type: "about_me".to_string(),
                    op: "read".to_string(),
                },
                json!({"kind": "self_tool", "self_type": "about_me", "op": "read"}),
            ),
            (
                CoreActionKind::ChatHistory {
                    operation: "query".to_string(),
                },
                json!({"kind": "chat_history", "operation": "query"}),
            ),
            (
                CoreActionKind::Other {
                    action: "future_tool".to_string(),
                },
                json!({"kind": "other", "action": "future_tool"}),
            ),
        ];

        for (kind, payload) in cases {
            assert_eq!(action_kind_topic_payload(&kind), payload);
            assert_eq!(action_kind_from_topic_payload(&payload, "fallback"), kind);
        }

        assert_eq!(
            action_kind_from_topic_payload(&json!({"kind": "unknown"}), "future_tool"),
            CoreActionKind::Other {
                action: "future_tool".to_string()
            }
        );
    }

    #[test]
    fn stopped_turn_summary_is_structured_and_ui_neutral() {
        let usage = UsageStats {
            llm_calls: 1,
            prompt_tokens: 10,
            completion_tokens: 2,
            total_tokens: 12,
            ..UsageStats::zero()
        };
        let stops = [
            TurnStopSummary::cancelled_by_user(),
            TurnStopSummary::model_error("provider_network_error"),
            TurnStopSummary::output_limit_stopped_by_user(10_000, usage.clone()),
            TurnStopSummary::round_limit_stopped_by_user(50, usage.clone(), Some(usage)),
        ];

        assert_eq!(stops[0].stop_reason, TurnStopReason::CancelledByUser);
        assert_eq!(stops[0].repair_issue.as_deref(), Some("cancelled_by_user"));
        assert_eq!(stops[0].stats.llm_calls, 0);
        assert!(stops[0].latest_usage.is_none());
        assert_eq!(
            stops[1].detail,
            TurnStopDetail::ModelError {
                error: "provider_network_error".to_string()
            }
        );
        assert_eq!(
            stops[2].detail,
            TurnStopDetail::OutputLimit {
                current_tokens: 10_000
            }
        );
        assert_eq!(
            stops[3].detail,
            TurnStopDetail::RoundLimit { max_rounds: 50 }
        );
        let stopped = stops[1].clone().into_stopped_turn();
        assert_eq!(stopped.stop_reason, TurnStopReason::ModelError);
        assert_eq!(stopped.repair_issue, None);
        let outcome = TurnOutcome::stopped("host-rendered text", stopped, Duration::from_secs(2));
        assert_eq!(outcome.text, "host-rendered text");
        assert_eq!(outcome.stop_reason, Some(TurnStopReason::ModelError));
        assert_eq!(outcome.elapsed, Duration::from_secs(2));

        let serialized = serde_json::to_value(TurnStopSummary::protocol_repair_failed(
            "invalid_json",
            "status_required",
            true,
            UsageStats::zero(),
            None,
        ))
        .unwrap();
        assert_eq!(serialized["stop_reason"], "protocol_repair_failed");
        assert_eq!(serialized["detail"]["kind"], "protocol_repair_failure");
        assert_eq!(serialized["detail"]["first_issue"], "invalid_json");
        assert_eq!(serialized["detail"]["final_issue"], "status_required");
        assert_eq!(serialized["detail"]["truncated"], true);

        let debug = format!("{stops:?}");
        for forbidden in ["已取消", "模型调用失败", "\x1b"] {
            assert!(
                !debug.contains(forbidden),
                "core stop summary leaked UI text {forbidden:?}: {debug}"
            );
        }
    }

    #[test]
    fn user_supplement_normalization_is_host_independent() {
        assert_eq!(
            normalize_user_supplements(vec![
                "  keep this  ".to_string(),
                "\n\t".to_string(),
                "第二条\n".to_string(),
            ]),
            vec!["keep this".to_string(), "第二条".to_string()]
        );
    }
}

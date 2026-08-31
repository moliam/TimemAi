pub mod message_queue;

use agent_core::{
    core_initialized_topic_event_with_worker, run_session_turn_with_model_client, AgentCore,
    CoreGlobalWorkerStatus, CoreSessionWorkerIdentity, CoreSessionWorkerWorkspace, CoreTopicEvent,
    HostDecision, HostDecisionRequest, HttpModelClient, ModelClient, ModelServiceConfig,
    ResponseProtocolKind, RuntimeProfiler, TopicReply, TurnInput, TurnOutcome, TurnStopDetail,
    TurnStopSummary, TurnUi, UsageStats,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
pub use timem_ui_contract::commands::ToolGenRequest;
pub use timem_ui_contract::projections::{
    CoreSessionWorkerLifecycleState, CoreSessionWorkerStatus,
};

fn unix_timestamp_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Typed Agent compatibility API exposed through the Session boundary.
pub use agent_core as agent_api;

pub fn run_synchronous_turn(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    input: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
) -> TurnOutcome {
    agent_core::run_session_turn(core, config, input, ui, profiler)
}

pub fn run_synchronous_turn_with_model_client(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    input: TurnInput<'_>,
    ui: &mut dyn TurnUi,
    profiler: Option<&mut RuntimeProfiler>,
    model_client: &mut dyn ModelClient,
) -> TurnOutcome {
    run_session_turn_with_model_client(core, config, input, ui, profiler, model_client)
}

const TOOLGEN_CONTEXT_INSTRUCTIONS: &str =
    include_str!("../../../resources/toolgen/toolgen_context.md");
const TOOLGEN_XML_COMPLETION: &str =
    include_str!("../../../resources/protocol/xml/toolgen_retrospect.md");
const TOOLGEN_JSON_COMPLETION: &str =
    include_str!("../../../resources/protocol/json/toolgen_retrospect.md");

#[derive(Debug, Clone)]
pub struct CoreSessionWorkerConfig {
    pub identity: CoreSessionWorkerIdentity,
    pub workspace: CoreSessionWorkerWorkspace,
    pub assistant_speaker_name: Option<String>,
    pub continue_supplements_after_final_answer: bool,
}

impl CoreSessionWorkerConfig {
    pub fn new(identity: CoreSessionWorkerIdentity, workspace: CoreSessionWorkerWorkspace) -> Self {
        Self {
            identity,
            workspace,
            assistant_speaker_name: None,
            continue_supplements_after_final_answer: true,
        }
    }

    pub fn with_assistant_speaker_name(mut self, name: impl Into<String>) -> Self {
        self.assistant_speaker_name = Some(name.into());
        self
    }

    pub fn with_separate_turn_for_supplements_after_final_answer(mut self) -> Self {
        self.continue_supplements_after_final_answer = false;
        self
    }

    pub fn session_id(&self) -> &str {
        &self.identity.session_id
    }

    pub fn context_id(&self) -> &str {
        &self.identity.context_id
    }

    pub fn worker_id(&self) -> &str {
        &self.identity.worker_id
    }
}

#[derive(Debug, Default)]
struct WorkingWorkerCounts {
    total: usize,
    by_session: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
pub struct CoreSessionWorkerRuntime {
    working_workers: Arc<Mutex<WorkingWorkerCounts>>,
}

impl CoreSessionWorkerRuntime {
    pub fn new() -> Self {
        Self {
            working_workers: Arc::new(Mutex::new(WorkingWorkerCounts::default())),
        }
    }

    pub fn working_worker_count(&self) -> usize {
        self.working_workers
            .lock()
            .map(|counts| counts.total)
            .unwrap_or(0)
    }

    fn worker_count_snapshot(&self, session_id: &str) -> (usize, usize) {
        self.working_workers
            .lock()
            .map(|counts| {
                (
                    counts.total,
                    counts.by_session.get(session_id).copied().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0))
    }

    fn begin_worker_turn(&self, session_id: &str) -> WorkingWorkerGuard {
        let active = Arc::new(AtomicBool::new(true));
        if let Ok(mut counts) = self.working_workers.lock() {
            counts.total = counts.total.saturating_add(1);
            *counts.by_session.entry(session_id.to_string()).or_insert(0) += 1;
        }
        WorkingWorkerGuard {
            working_workers: Arc::clone(&self.working_workers),
            session_id: session_id.to_string(),
            active,
        }
    }

    fn finish_worker_turn_if_active(
        &self,
        session_id: &str,
        active: &Arc<AtomicBool>,
    ) -> (usize, usize) {
        if active.swap(false, Ordering::SeqCst) {
            decrement_working_count(&self.working_workers, session_id)
        } else {
            self.worker_count_snapshot(session_id)
        }
    }

    fn model_response_global_status(
        &self,
        session_id: &str,
        continue_work: bool,
        active: Option<&Arc<AtomicBool>>,
    ) -> CoreGlobalWorkerStatus {
        let (global_count, session_count) = if continue_work {
            self.worker_count_snapshot(session_id)
        } else if let Some(active) = active {
            self.finish_worker_turn_if_active(session_id, active)
        } else {
            self.worker_count_snapshot(session_id)
        };
        CoreGlobalWorkerStatus::with_session_working_worker_count(global_count, session_count)
    }

    fn enrich_topic_events(
        &self,
        session_id: &str,
        events: Vec<CoreTopicEvent>,
        active: Option<&Arc<AtomicBool>>,
    ) -> Vec<CoreTopicEvent> {
        events
            .into_iter()
            .map(|event| {
                let Some(model_response) = event.as_model_response() else {
                    return event;
                };
                event.with_global_worker_status(self.model_response_global_status(
                    session_id,
                    model_response.continue_work,
                    active,
                ))
            })
            .collect()
    }
}

impl Default for CoreSessionWorkerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

struct WorkingWorkerGuard {
    working_workers: Arc<Mutex<WorkingWorkerCounts>>,
    session_id: String,
    active: Arc<AtomicBool>,
}

fn decrement_working_count(
    counts: &Arc<Mutex<WorkingWorkerCounts>>,
    session_id: &str,
) -> (usize, usize) {
    let Ok(mut counts) = counts.lock() else {
        return (0, 0);
    };
    counts.total = counts.total.saturating_sub(1);
    let remaining = counts
        .by_session
        .get_mut(session_id)
        .map(|count| {
            *count = count.saturating_sub(1);
            *count
        })
        .unwrap_or(0);
    if remaining == 0 {
        counts.by_session.remove(session_id);
    }
    (counts.total, remaining)
}

impl WorkingWorkerGuard {
    fn active_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.active)
    }
}

impl Drop for WorkingWorkerGuard {
    fn drop(&mut self) {
        if self.active.swap(false, Ordering::SeqCst) {
            decrement_working_count(&self.working_workers, &self.session_id);
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum CoreSessionWorkerEvent {
    /// The worker has dequeued the durable Host intent. This is deliberately
    /// separate from the Host successfully writing the user entry: only this
    /// event permits the Host to terminally acknowledge delivery to Core.
    CommandAccepted {
        command_id: String,
    },
    /// Core has entered turn execution and now owns live working state.
    /// Hosts must not infer this state from history or command enqueueing.
    TurnStarted {
        command_id: Option<String>,
    },
    /// Complete Core-owned lifecycle projection. Hosts may cache and deliver it,
    /// but must not rewrite it from worker/topic arrival order.
    TurnProjection(agent_core::TurnProjection),
    Topics(Vec<CoreTopicEvent>),
    ModelRequest {
        round: u32,
        emitted_at_ms: u128,
        prompt: String,
        interaction_profile: Option<agent_core::InteractionProfile>,
        interaction_request: Option<Box<agent_core::ModelInteractionRequest>>,
        api_payload: Option<Box<serde_json::Value>>,
    },
    ModelRequestCompleted {
        latency: Duration,
    },
    ModelResponseParsed {
        tool_count: usize,
    },
    ModelResponse {
        round: u32,
        usage: UsageStats,
        content: String,
        tool_calls: Vec<agent_core::NativeToolCall>,
        truncated: bool,
        runtime_phase: Option<String>,
    },
    ModelRetry {
        attempt: u32,
        max_attempts: u32,
        delay: Duration,
        error: String,
    },
    ModelError {
        error: String,
    },
    UnconsumedSupplements {
        supplements: Vec<String>,
    },
    TurnFinished {
        outcome: TurnOutcome,
    },
    WorkerStopped,
}

enum CoreSessionWorkerCommand {
    RunTurn {
        input: String,
        additional_context: Option<String>,
        command_id: Option<String>,
        initial_supplements: Vec<QueuedSupplement>,
        cancel_generation: u64,
    },
    RunToolGen {
        request: ToolGenRequest,
        command_id: Option<String>,
        cancel_generation: u64,
    },
    Rename {
        display_name: String,
        assistant_speaker_name: Option<String>,
    },
    UpdateBashApproval {
        mode: agent_core::BashApprovalMode,
    },
    ChangeCwd {
        current_dir: PathBuf,
        result_tx: Sender<Result<PathBuf, String>>,
    },
    RuntimeConfigUpdated,
    MaxRoundsUpdated,
    UpdateApiKey {
        api_key: String,
    },
    UpdateHttpHeaders {
        http_headers: BTreeMap<String, String>,
    },
    UpdateRequestFields {
        request_fields: BTreeMap<String, serde_json::Value>,
    },
    UpdateMcp {
        base_capabilities: agent_core::capability::CapabilityRegistry,
        runtime: agent_core::mcp::McpRuntime,
        servers: Vec<agent_core::mcp::McpServerConfig>,
        tools: Vec<agent_core::mcp::McpTool>,
        instructions: BTreeMap<String, String>,
    },
    Shutdown,
}

#[derive(Default)]
struct SupplementMailbox {
    accepting: bool,
    queue: Vec<QueuedSupplement>,
}

struct QueuedSupplement {
    text: String,
    additional_context: Option<String>,
    command_id: Option<String>,
}

enum PendingRuntimeUpdate {
    Config {
        field: agent_core::RuntimeConfigField,
        value: String,
    },
    OpenAiCompatible {
        key: String,
        value: String,
    },
    MaxRounds(u32),
}

#[derive(Clone)]
pub struct CoreSessionWorkerHandle {
    command_tx: Sender<CoreSessionWorkerCommand>,
    supplement_mailbox: Arc<Mutex<SupplementMailbox>>,
    cancel_requested: Arc<AtomicBool>,
    cancel_generation: Arc<AtomicU64>,
    shutdown_requested: Arc<AtomicBool>,
    reply_tx: Sender<TopicReply>,
    accepted_command_ids: Arc<Mutex<BTreeSet<String>>>,
    pending_runtime_updates: Arc<Mutex<Vec<PendingRuntimeUpdate>>>,
    background_cancel: Arc<dyn Fn() + Send + Sync>,
}

impl CoreSessionWorkerHandle {
    pub fn run_turn(
        &self,
        input: impl Into<String>,
        additional_context: Option<String>,
    ) -> Result<(), String> {
        self.run_turn_with_command_id(input, additional_context, None)
    }

    pub fn run_turn_with_command_id(
        &self,
        input: impl Into<String>,
        additional_context: Option<String>,
        command_id: Option<String>,
    ) -> Result<(), String> {
        self.run_turn_batch_with_command_ids(input, additional_context, command_id, Vec::new())
    }

    pub fn run_turn_batch_with_command_ids(
        &self,
        input: impl Into<String>,
        additional_context: Option<String>,
        command_id: Option<String>,
        supplements: Vec<(String, Option<String>)>,
    ) -> Result<(), String> {
        self.run_turn_batch_with_supplements(
            input,
            additional_context,
            command_id,
            supplements
                .into_iter()
                .map(|(text, command_id)| (agent_core::UserSupplement::from(text), command_id))
                .collect(),
        )
    }

    pub fn run_turn_batch_with_supplements(
        &self,
        input: impl Into<String>,
        additional_context: Option<String>,
        command_id: Option<String>,
        supplements: Vec<(agent_core::UserSupplement, Option<String>)>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        let mut batch_command_ids = command_id.iter().cloned().collect::<Vec<_>>();
        batch_command_ids.extend(
            supplements
                .iter()
                .filter_map(|(_, command_id)| command_id.clone()),
        );
        let unique_ids = batch_command_ids.iter().cloned().collect::<BTreeSet<_>>();
        if unique_ids.len() != batch_command_ids.len() {
            return Err("core_command_batch_duplicate_id".to_string());
        }
        {
            let mut accepted = self
                .accepted_command_ids
                .lock()
                .map_err(|_| "core_command_dedup_poisoned".to_string())?;
            if command_id
                .as_ref()
                .is_some_and(|command_id| accepted.contains(command_id))
            {
                return Ok(());
            }
            if batch_command_ids.iter().any(|id| accepted.contains(id)) {
                return Err("core_command_batch_id_conflict".to_string());
            }
            accepted.extend(batch_command_ids.iter().cloned());
        }
        let cancel_generation = self.cancel_generation.load(Ordering::SeqCst);
        self.open_supplement_mailbox();
        let result = self
            .command_tx
            .send(CoreSessionWorkerCommand::RunTurn {
                input: input.into(),
                additional_context,
                command_id: command_id.clone(),
                initial_supplements: supplements
                    .into_iter()
                    .map(|(supplement, command_id)| QueuedSupplement {
                        text: supplement.text,
                        additional_context: supplement.additional_context,
                        command_id,
                    })
                    .collect(),
                cancel_generation,
            })
            .map_err(|_| "core_session_worker_stopped".to_string());
        if result.is_err() {
            self.close_supplement_mailbox();
            if let Ok(mut accepted) = self.accepted_command_ids.lock() {
                for command_id in &batch_command_ids {
                    accepted.remove(command_id);
                }
            }
        }
        result
    }

    pub fn run_toolgen(&self, request: ToolGenRequest) -> Result<(), String> {
        self.run_toolgen_with_command_id(request, None)
    }

    pub fn run_toolgen_with_command_id(
        &self,
        request: ToolGenRequest,
        command_id: Option<String>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        if let Some(command_id) = command_id.as_ref() {
            let mut accepted = self
                .accepted_command_ids
                .lock()
                .map_err(|_| "core_command_dedup_poisoned".to_string())?;
            if !accepted.insert(command_id.clone()) {
                return Ok(());
            }
        }
        let cancel_generation = self.cancel_generation.load(Ordering::SeqCst);
        self.open_supplement_mailbox();
        let result = self
            .command_tx
            .send(CoreSessionWorkerCommand::RunToolGen {
                request,
                command_id: command_id.clone(),
                cancel_generation,
            })
            .map_err(|_| "core_session_worker_stopped".to_string());
        if result.is_err() {
            self.close_supplement_mailbox();
            if let Some(command_id) = command_id.as_ref() {
                if let Ok(mut accepted) = self.accepted_command_ids.lock() {
                    accepted.remove(command_id);
                }
            }
        }
        result
    }

    pub fn try_add_user_supplement(&self, supplement: impl Into<String>) -> bool {
        self.supplement_mailbox
            .lock()
            .map(|mut mailbox| {
                if !mailbox.accepting {
                    return false;
                }
                mailbox.queue.push(QueuedSupplement {
                    text: supplement.into(),
                    additional_context: None,
                    command_id: None,
                });
                true
            })
            .unwrap_or(false)
    }

    /// Runs the Host's durable append while holding the supplement boundary.
    /// This prevents Core from closing the mailbox between Host persistence and
    /// enqueue, without making Core depend on the Host's storage implementation.
    pub fn try_add_user_supplement_after<F>(
        &self,
        supplement: impl Into<String>,
        before_enqueue: F,
    ) -> Result<bool, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let mut mailbox = self
            .supplement_mailbox
            .lock()
            .map_err(|_| "supplement_mailbox_poisoned".to_string())?;
        if !mailbox.accepting {
            return Ok(false);
        }
        before_enqueue()?;
        mailbox.queue.push(QueuedSupplement {
            text: supplement.into(),
            additional_context: None,
            command_id: None,
        });
        Ok(true)
    }

    pub fn try_add_user_supplement_with_command_id_after<F>(
        &self,
        supplement: impl Into<String>,
        command_id: Option<String>,
        before_enqueue: F,
    ) -> Result<bool, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.try_add_user_supplement_with_context_and_command_id_after(
            supplement,
            None,
            command_id,
            before_enqueue,
        )
    }

    pub fn try_add_user_supplement_with_context_and_command_id_after<F>(
        &self,
        supplement: impl Into<String>,
        additional_context: Option<String>,
        command_id: Option<String>,
        before_enqueue: F,
    ) -> Result<bool, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let mut mailbox = self
            .supplement_mailbox
            .lock()
            .map_err(|_| "supplement_mailbox_poisoned".to_string())?;
        if !mailbox.accepting {
            return Ok(false);
        }
        if let Some(command_id) = command_id.as_ref() {
            let mut accepted = self
                .accepted_command_ids
                .lock()
                .map_err(|_| "core_command_dedup_poisoned".to_string())?;
            if !accepted.insert(command_id.clone()) {
                return Ok(true);
            }
        }
        if let Err(error) = before_enqueue() {
            if let Some(command_id) = command_id.as_ref() {
                if let Ok(mut accepted) = self.accepted_command_ids.lock() {
                    accepted.remove(command_id);
                }
            }
            return Err(error);
        }
        mailbox.queue.push(QueuedSupplement {
            text: supplement.into(),
            additional_context,
            command_id,
        });
        Ok(true)
    }

    pub fn add_user_supplement(&self, supplement: impl Into<String>) {
        let _ = self.try_add_user_supplement(supplement);
    }

    pub fn cancel_current_turn(&self) {
        self.signal_turn_cancel();
        (self.background_cancel)();
    }

    fn signal_turn_cancel(&self) {
        self.cancel_generation.fetch_add(1, Ordering::SeqCst);
        self.cancel_requested.store(true, Ordering::SeqCst);
    }

    pub fn reply_to_request(&self, reply: TopicReply) -> Result<(), String> {
        self.reply_tx
            .send(reply)
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn request_shutdown(&self) -> Result<(), String> {
        self.close_supplement_mailbox();
        self.shutdown_requested.store(true, Ordering::SeqCst);
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.command_tx
            .send(CoreSessionWorkerCommand::Shutdown)
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    fn close_supplement_mailbox(&self) {
        if let Ok(mut mailbox) = self.supplement_mailbox.lock() {
            mailbox.accepting = false;
            mailbox.queue.clear();
        }
    }

    fn open_supplement_mailbox(&self) {
        if let Ok(mut mailbox) = self.supplement_mailbox.lock() {
            if !mailbox.accepting {
                mailbox.accepting = true;
                mailbox.queue.clear();
            }
        }
    }

    pub fn is_accepting_user_supplements(&self) -> bool {
        self.supplement_mailbox
            .lock()
            .map(|mailbox| mailbox.accepting)
            .unwrap_or(false)
    }

    pub fn rename(&self, display_name: impl Into<String>) -> Result<(), String> {
        self.rename_with_assistant_speaker_name(display_name, None)
    }

    pub fn rename_with_assistant_speaker_name(
        &self,
        display_name: impl Into<String>,
        assistant_speaker_name: Option<String>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::Rename {
                display_name: display_name.into(),
                assistant_speaker_name,
            })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn update_bash_approval(&self, mode: agent_core::BashApprovalMode) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::UpdateBashApproval { mode })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    /// Queues a prompt/workspace cwd change before any subsequently queued turn.
    pub fn change_cwd(&self, current_dir: PathBuf) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        let (result_tx, result_rx) = mpsc::channel();
        self.command_tx
            .send(CoreSessionWorkerCommand::ChangeCwd {
                current_dir,
                result_tx,
            })
            .map_err(|_| "core_session_worker_stopped".to_string())?;
        result_rx
            .recv()
            .map_err(|_| "core_session_worker_stopped".to_string())?
            .map(|_| ())
    }

    pub fn update_runtime_config(
        &self,
        field: agent_core::RuntimeConfigField,
        value: String,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.enqueue_runtime_update(
            PendingRuntimeUpdate::Config { field, value },
            CoreSessionWorkerCommand::RuntimeConfigUpdated,
        )
    }

    pub fn update_openai_compatible_config(
        &self,
        key: String,
        value: String,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.enqueue_runtime_update(
            PendingRuntimeUpdate::OpenAiCompatible { key, value },
            CoreSessionWorkerCommand::RuntimeConfigUpdated,
        )
    }

    pub fn update_max_rounds(&self, max_rounds: u32) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.enqueue_runtime_update(
            PendingRuntimeUpdate::MaxRounds(max_rounds),
            CoreSessionWorkerCommand::MaxRoundsUpdated,
        )
    }

    fn enqueue_runtime_update(
        &self,
        update: PendingRuntimeUpdate,
        notification: CoreSessionWorkerCommand,
    ) -> Result<(), String> {
        let mut pending = self
            .pending_runtime_updates
            .lock()
            .map_err(|_| "core_runtime_update_poisoned".to_string())?;
        pending.push(update);
        if self.command_tx.send(notification).is_err() {
            pending.pop();
            return Err("core_session_worker_stopped".to_string());
        }
        Ok(())
    }

    pub fn update_api_key(&self, api_key: String) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::UpdateApiKey { api_key })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn update_http_headers(
        &self,
        http_headers: BTreeMap<String, String>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::UpdateHttpHeaders { http_headers })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn update_request_fields(
        &self,
        request_fields: BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::UpdateRequestFields { request_fields })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn update_mcp(
        &self,
        base_capabilities: agent_core::capability::CapabilityRegistry,
        runtime: agent_core::mcp::McpRuntime,
        servers: Vec<agent_core::mcp::McpServerConfig>,
        tools: Vec<agent_core::mcp::McpTool>,
    ) -> Result<(), String> {
        self.update_mcp_with_instructions(
            base_capabilities,
            runtime,
            servers,
            tools,
            BTreeMap::new(),
        )
    }

    pub fn update_mcp_with_instructions(
        &self,
        base_capabilities: agent_core::capability::CapabilityRegistry,
        runtime: agent_core::mcp::McpRuntime,
        servers: Vec<agent_core::mcp::McpServerConfig>,
        tools: Vec<agent_core::mcp::McpTool>,
        instructions: BTreeMap<String, String>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::UpdateMcp {
                base_capabilities,
                runtime,
                servers,
                tools,
                instructions,
            })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }
}

pub struct CoreSessionWorker {
    handle: CoreSessionWorkerHandle,
    event_rx: Receiver<CoreSessionWorkerEvent>,
    join: Option<JoinHandle<()>>,
}

struct ManagedSessionWorker {
    identity: CoreSessionWorkerIdentity,
    state: CoreSessionWorkerLifecycleState,
    worker: CoreSessionWorker,
}

pub struct CoreSessionWorkerManager {
    runtime: CoreSessionWorkerRuntime,
    next_session_ordinal: u32,
    next_worker_ordinal: u32,
    workers: BTreeMap<String, ManagedSessionWorker>,
}

impl CoreSessionWorkerManager {
    pub fn new() -> Self {
        Self {
            runtime: CoreSessionWorkerRuntime::new(),
            next_session_ordinal: 0,
            next_worker_ordinal: 0,
            workers: BTreeMap::new(),
        }
    }

    pub fn runtime(&self) -> CoreSessionWorkerRuntime {
        self.runtime.clone()
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn working_worker_count(&self) -> usize {
        self.runtime.working_worker_count()
    }

    pub fn statuses(&self) -> Vec<CoreSessionWorkerStatus> {
        self.workers
            .values()
            .map(|worker| CoreSessionWorkerStatus {
                identity: worker.identity.clone(),
                state: worker.state,
            })
            .collect()
    }

    pub fn handle(&self, worker_id: &str) -> Option<CoreSessionWorkerHandle> {
        self.workers
            .get(worker_id)
            .map(|worker| worker.worker.handle())
    }

    /// Broadcast cancellation to every worker in a Session without waiting for
    /// model transports, tools, or worker cleanup to finish.
    pub fn cancel_session_turns(&self, session_id: &str) -> usize {
        let handles = self
            .workers
            .values()
            .filter(|worker| worker.identity.session_id == session_id)
            .map(|worker| worker.worker.handle())
            .collect::<Vec<_>>();
        for handle in &handles {
            handle.signal_turn_cancel();
        }
        if let Some(handle) = handles.first() {
            (handle.background_cancel)();
        }
        handles.len()
    }

    pub fn ensure_default_worker(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
    ) -> Result<String, String> {
        if let Some(worker_id) = self.workers.keys().next() {
            return Ok(worker_id.clone());
        }
        self.spawn_worker(core, config, workspace, None, None)
    }

    pub fn ensure_default_worker_with_model_client<M>(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        if let Some(worker_id) = self.workers.keys().next() {
            return Ok(worker_id.clone());
        }
        self.spawn_worker_with_model_client(core, config, workspace, None, None, model_client)
    }

    pub fn spawn_worker(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
    ) -> Result<String, String> {
        self.spawn_worker_with_model_client(
            core,
            config,
            workspace,
            display_name,
            parent_worker_id,
            HttpModelClient,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_worker_in_session(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
    ) -> Result<String, String> {
        self.spawn_worker_in_session_with_assistant_speaker_name(
            core,
            config,
            workspace,
            session_id,
            context_id,
            display_name,
            parent_worker_id,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_worker_in_session_with_separate_late_supplement_turn(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
        assistant_speaker_name: Option<String>,
    ) -> Result<String, String> {
        self.spawn_worker_in_session_with_model_client_and_options(
            core,
            config,
            workspace,
            session_id,
            context_id,
            display_name,
            parent_worker_id,
            assistant_speaker_name,
            false,
            HttpModelClient,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_worker_in_session_with_assistant_speaker_name(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
        assistant_speaker_name: Option<String>,
    ) -> Result<String, String> {
        self.spawn_worker_in_session_with_model_client_and_assistant_speaker_name(
            core,
            config,
            workspace,
            session_id,
            context_id,
            display_name,
            parent_worker_id,
            assistant_speaker_name,
            HttpModelClient,
        )
    }

    pub fn spawn_worker_with_model_client<M>(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        let session_ordinal = self.next_session_ordinal;
        self.next_session_ordinal = self
            .next_session_ordinal
            .checked_add(1)
            .ok_or_else(|| "session_ordinal_overflow".to_string())?;
        self.spawn_worker_in_session_with_model_client(
            core,
            config,
            workspace,
            format!("session_{session_ordinal}"),
            "context_0",
            display_name,
            parent_worker_id,
            model_client,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_worker_in_session_with_model_client<M>(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        self.spawn_worker_in_session_with_model_client_and_assistant_speaker_name(
            core,
            config,
            workspace,
            session_id,
            context_id,
            display_name,
            parent_worker_id,
            None,
            model_client,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_worker_in_session_with_model_client_and_assistant_speaker_name<M>(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
        assistant_speaker_name: Option<String>,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        self.spawn_worker_in_session_with_model_client_and_options(
            core,
            config,
            workspace,
            session_id,
            context_id,
            display_name,
            parent_worker_id,
            assistant_speaker_name,
            true,
            model_client,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_worker_in_session_with_model_client_and_options<M>(
        &mut self,
        core: AgentCore,
        config: ModelServiceConfig,
        workspace: CoreSessionWorkerWorkspace,
        session_id: impl Into<String>,
        context_id: impl Into<String>,
        display_name: Option<String>,
        parent_worker_id: Option<String>,
        assistant_speaker_name: Option<String>,
        continue_supplements_after_final_answer: bool,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        let session_id = session_id.into();
        let context_id = context_id.into();
        if self.workers.values().any(|worker| {
            worker.identity.session_id == session_id && worker.identity.context_id == context_id
        }) {
            return Err("session_context_worker_exists".to_string());
        }
        if let Some(parent_worker_id) = parent_worker_id.as_deref() {
            let parent = self
                .workers
                .get(parent_worker_id)
                .ok_or_else(|| "parent_worker_not_found".to_string())?;
            if parent.identity.session_id != session_id {
                return Err("parent_worker_session_mismatch".to_string());
            }
        }
        let is_primary_worker = !self
            .workers
            .values()
            .any(|worker| worker.identity.session_id == session_id);
        let ordinal = self.next_worker_ordinal;
        self.next_worker_ordinal = self
            .next_worker_ordinal
            .checked_add(1)
            .ok_or_else(|| "session_worker_ordinal_overflow".to_string())?;
        let worker_id = format!("worker_{ordinal}");
        let identity = CoreSessionWorkerIdentity::new_scoped(
            session_id,
            context_id,
            worker_id.clone(),
            ordinal,
            display_name,
            parent_worker_id,
        );
        let mut core = core;
        core.set_sub_answer_enabled(is_primary_worker);
        let worker = CoreSessionWorker::spawn_with_runtime_model_client(
            core,
            config,
            {
                let mut worker_config = CoreSessionWorkerConfig::new(identity.clone(), workspace);
                if let Some(name) = assistant_speaker_name {
                    worker_config = worker_config.with_assistant_speaker_name(name);
                }
                if !continue_supplements_after_final_answer {
                    worker_config =
                        worker_config.with_separate_turn_for_supplements_after_final_answer();
                }
                worker_config
            },
            self.runtime.clone(),
            model_client,
        );
        self.workers.insert(
            worker_id.clone(),
            ManagedSessionWorker {
                identity,
                state: CoreSessionWorkerLifecycleState::Running,
                worker,
            },
        );
        Ok(worker_id)
    }

    pub fn try_recv_event(&mut self, worker_id: &str) -> Option<CoreSessionWorkerEvent> {
        let managed = self.workers.get_mut(worker_id)?;
        match managed.worker.events().try_recv() {
            Ok(event) => {
                if matches!(event, CoreSessionWorkerEvent::WorkerStopped) {
                    managed.state = CoreSessionWorkerLifecycleState::Stopped;
                }
                Some(event)
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    pub fn request_shutdown(&mut self, worker_id: &str) -> Result<(), String> {
        let managed = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| "session_worker_not_found".to_string())?;
        managed.state = CoreSessionWorkerLifecycleState::Stopping;
        managed.worker.handle().request_shutdown()
    }

    pub fn remove_stopped(&mut self, worker_id: &str) -> Result<(), String> {
        let Some(managed) = self.workers.get(worker_id) else {
            return Err("session_worker_not_found".to_string());
        };
        if managed.state != CoreSessionWorkerLifecycleState::Stopped {
            return Err("session_worker_not_stopped".to_string());
        }
        let managed = self.workers.remove(worker_id).unwrap();
        managed.worker.shutdown()
    }

    pub fn shutdown_worker(&mut self, worker_id: &str) -> Result<(), String> {
        let managed = self
            .workers
            .remove(worker_id)
            .ok_or_else(|| "session_worker_not_found".to_string())?;
        managed.worker.shutdown()
    }

    pub fn shutdown_all(mut self) -> Result<(), String> {
        for managed in self.workers.values_mut() {
            let _ = managed.worker.handle().request_shutdown();
            managed.state = CoreSessionWorkerLifecycleState::Stopping;
        }
        let mut first_error = None;
        for (_worker_id, managed) in self.workers {
            if let Err(err) = managed.worker.shutdown() {
                first_error.get_or_insert(err);
            }
        }
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    pub fn shutdown_all_detached(self) -> Result<(), String> {
        for (_worker_id, managed) in self.workers {
            managed.worker.shutdown_detached();
        }
        Ok(())
    }
}

impl Default for CoreSessionWorkerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreSessionWorker {
    pub fn spawn(
        core: AgentCore,
        config: ModelServiceConfig,
        worker_config: CoreSessionWorkerConfig,
    ) -> Self {
        Self::spawn_with_runtime_model_client(
            core,
            config,
            worker_config,
            CoreSessionWorkerRuntime::new(),
            HttpModelClient,
        )
    }

    pub fn spawn_with_model_client<M>(
        core: AgentCore,
        config: ModelServiceConfig,
        worker_config: CoreSessionWorkerConfig,
        model_client: M,
    ) -> Self
    where
        M: ModelClient + Send + 'static,
    {
        Self::spawn_with_runtime_model_client(
            core,
            config,
            worker_config,
            CoreSessionWorkerRuntime::new(),
            model_client,
        )
    }

    pub fn spawn_with_runtime_model_client<M>(
        mut core: AgentCore,
        mut config: ModelServiceConfig,
        worker_config: CoreSessionWorkerConfig,
        runtime: CoreSessionWorkerRuntime,
        mut model_client: M,
    ) -> Self
    where
        M: ModelClient + Send + 'static,
    {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let (reply_tx, reply_rx) = mpsc::channel();
        let supplement_mailbox = Arc::new(Mutex::new(SupplementMailbox::default()));
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let cancel_generation = Arc::new(AtomicU64::new(0));
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let accepted_command_ids = Arc::new(Mutex::new(BTreeSet::new()));
        let pending_runtime_updates = Arc::new(Mutex::new(Vec::new()));
        let background_cancel =
            core.background_resource_cancel_callback(worker_config.identity.session_id.clone());
        let handle = CoreSessionWorkerHandle {
            command_tx,
            supplement_mailbox: Arc::clone(&supplement_mailbox),
            cancel_requested: Arc::clone(&cancel_requested),
            cancel_generation: Arc::clone(&cancel_generation),
            shutdown_requested: Arc::clone(&shutdown_requested),
            reply_tx,
            accepted_command_ids,
            pending_runtime_updates: Arc::clone(&pending_runtime_updates),
            background_cancel,
        };
        let join = thread::spawn(move || {
            let mut identity = worker_config.identity.clone();
            let mut workspace = worker_config.workspace.clone();
            let mut assistant_speaker_name = worker_config
                .assistant_speaker_name
                .clone()
                .unwrap_or_else(|| identity.display_name.clone());
            let continue_supplements_after_final_answer =
                worker_config.continue_supplements_after_final_answer;
            core.set_response_protocol(config.response_protocol);
            core.set_assistant_speaker_name(&assistant_speaker_name);
            core.set_tool_repo_session_id(&identity.session_id);
            let init_event = core_initialized_topic_event_with_worker(
                &identity.session_id,
                core.profile(),
                core.response_protocol_name(),
                core.max_llm_input_tokens(),
                core.configured_round_budget(),
                core.capability_tool_count(),
                core.capability_skill_count(),
                Some(&identity),
                Some(&workspace),
                Some(core.dynamic_context_summary()),
            )
            .with_worker_scope(&identity.context_id, &identity.worker_id);
            let _ = event_tx.send(CoreSessionWorkerEvent::Topics(vec![init_event]));
            let mut profiler = RuntimeProfiler::default();
            let mut ui = WorkerTurnUi {
                event_tx: event_tx.clone(),
                session_id: identity.session_id.clone(),
                context_id: identity.context_id.clone(),
                worker_id: identity.worker_id.clone(),
                supplement_mailbox,
                cancel_requested: Arc::clone(&cancel_requested),
                reply_rx,
                runtime: runtime.clone(),
                current_turn_active: None,
                phase: None,
                accept_supplements: true,
                continue_supplements_after_final_answer,
                pending_bash_always_allow: false,
                pending_runtime_updates,
                interaction_profile: None,
            };

            let mut has_running_shell_jobs = false;
            loop {
                let command = if has_running_shell_jobs {
                    match command_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => {
                            let context_id = identity.context_id.clone();
                            has_running_shell_jobs = !core
                                .refresh_running_shell_jobs_for_session_with_runtime(
                                    &context_id,
                                    Some(&mut ui),
                                )
                                .is_empty();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match command_rx.recv() {
                        Ok(command) => command,
                        Err(_) => break,
                    }
                };
                match command {
                    CoreSessionWorkerCommand::RunTurn { .. }
                    | CoreSessionWorkerCommand::RunToolGen { .. }
                    | CoreSessionWorkerCommand::Rename { .. }
                    | CoreSessionWorkerCommand::UpdateBashApproval { .. }
                    | CoreSessionWorkerCommand::ChangeCwd { .. }
                    | CoreSessionWorkerCommand::RuntimeConfigUpdated
                    | CoreSessionWorkerCommand::MaxRoundsUpdated
                    | CoreSessionWorkerCommand::UpdateApiKey { .. }
                    | CoreSessionWorkerCommand::UpdateHttpHeaders { .. }
                    | CoreSessionWorkerCommand::UpdateRequestFields { .. }
                    | CoreSessionWorkerCommand::UpdateMcp { .. }
                        if shutdown_requested.load(Ordering::SeqCst) =>
                    {
                        break;
                    }
                    CoreSessionWorkerCommand::RunTurn {
                        mut input,
                        mut additional_context,
                        command_id,
                        initial_supplements,
                        cancel_generation: command_generation,
                    } => {
                        if command_generation < cancel_generation.load(Ordering::SeqCst) {
                            if let Some(command_id) = command_id.as_ref() {
                                let _ = event_tx.send(CoreSessionWorkerEvent::CommandAccepted {
                                    command_id: command_id.clone(),
                                });
                            }
                            for supplement in initial_supplements {
                                if let Some(command_id) = supplement.command_id {
                                    let _ =
                                        event_tx.send(CoreSessionWorkerEvent::CommandAccepted {
                                            command_id,
                                        });
                                }
                            }
                            // The Host has already recorded a pending turn before enqueueing
                            // this command. Complete a zero-work cancellation lifecycle so the
                            // pending turn cannot remain visually working until a UI timeout.
                            let _ = event_tx.send(CoreSessionWorkerEvent::TurnStarted {
                                command_id: command_id.clone(),
                            });
                            let outcome = TurnOutcome::stopped(
                                "",
                                TurnStopSummary::cancelled_by_user().into_stopped_turn(),
                                Duration::ZERO,
                            );
                            let _ = event_tx.send(CoreSessionWorkerEvent::TurnFinished { outcome });
                            continue;
                        }
                        cancel_requested.store(false, Ordering::SeqCst);
                        if !initial_supplements.is_empty() {
                            if let Ok(mut mailbox) = ui.supplement_mailbox.lock() {
                                mailbox.queue.extend(initial_supplements);
                            }
                        }
                        if let Some(command_id) = command_id.as_ref() {
                            let _ = event_tx.send(CoreSessionWorkerEvent::CommandAccepted {
                                command_id: command_id.clone(),
                            });
                        }
                        let context_id = identity.context_id.clone();
                        let outcome = {
                            let working = runtime.begin_worker_turn(&identity.session_id);
                            let _ = event_tx.send(CoreSessionWorkerEvent::TurnStarted {
                                command_id: command_id.clone(),
                            });
                            ui.current_turn_active = Some(working.active_handle());
                            let outcome = loop {
                                let main_outcome = run_session_turn_with_model_client(
                                    &mut core,
                                    &mut config,
                                    TurnInput {
                                        input: &input,
                                        session: &context_id,
                                        audit_file: &workspace.audit_file,
                                        runtime: &workspace.runtime,
                                        run_bash_target: &workspace.run_bash_target,
                                        additional_context: additional_context.as_deref(),
                                    },
                                    &mut ui,
                                    Some(&mut profiler),
                                    &mut model_client,
                                );
                                if main_outcome.stop_summary.is_some()
                                    || !ui.continue_supplements_after_final_answer
                                {
                                    // A structured stop is a hard boundary. Web-style turn UIs
                                    // also treat a visible final answer as a boundary so a late
                                    // supplement cannot create a second answer in the same turn.
                                    let supplements = ui.close_supplements_for_main_context();
                                    if !supplements.is_empty() {
                                        let _ = event_tx.send(
                                            CoreSessionWorkerEvent::UnconsumedSupplements {
                                                supplements: supplements
                                                    .into_iter()
                                                    .map(|supplement| supplement.text)
                                                    .collect(),
                                            },
                                        );
                                    }
                                    break main_outcome;
                                }
                                let supplements = ui.take_or_close_supplements_for_main_context();
                                if supplements.is_empty() {
                                    break main_outcome;
                                }
                                input = supplements
                                    .iter()
                                    .map(|supplement| supplement.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n\n");
                                let supplement_contexts = supplements
                                    .iter()
                                    .filter_map(|supplement| {
                                        supplement.additional_context.as_deref()
                                    })
                                    .collect::<Vec<_>>();
                                additional_context = agent_core::combine_additional_contexts(
                                    std::iter::once(Some("These user messages arrived while the previous response was being finalized. Address them before finalizing again."))
                                        .chain(supplement_contexts.into_iter().map(Some)),
                                );
                            };
                            ui.current_turn_active = None;
                            drop(working);
                            outcome
                        };
                        has_running_shell_jobs = !outcome.running_jobs.is_empty();
                        let _ = event_tx.send(CoreSessionWorkerEvent::TurnFinished { outcome });
                    }
                    CoreSessionWorkerCommand::RunToolGen {
                        request,
                        command_id,
                        cancel_generation: command_generation,
                    } => {
                        if command_generation < cancel_generation.load(Ordering::SeqCst) {
                            if let Some(command_id) = command_id.as_ref() {
                                let _ = event_tx.send(CoreSessionWorkerEvent::CommandAccepted {
                                    command_id: command_id.clone(),
                                });
                            }
                            let _ = event_tx.send(CoreSessionWorkerEvent::TurnStarted {
                                command_id: command_id.clone(),
                            });
                            let outcome = TurnOutcome::stopped(
                                "",
                                TurnStopSummary::cancelled_by_user().into_stopped_turn(),
                                Duration::ZERO,
                            );
                            let _ = event_tx.send(CoreSessionWorkerEvent::TurnFinished { outcome });
                            continue;
                        }
                        cancel_requested.store(false, Ordering::SeqCst);
                        if let Some(command_id) = command_id.as_ref() {
                            let _ = event_tx.send(CoreSessionWorkerEvent::CommandAccepted {
                                command_id: command_id.clone(),
                            });
                        }
                        let working = runtime.begin_worker_turn(&identity.session_id);
                        let _ = event_tx.send(CoreSessionWorkerEvent::TurnStarted {
                            command_id: command_id.clone(),
                        });
                        ui.current_turn_active = Some(working.active_handle());
                        let toolgen_runner = ToolGenRunner {
                            core: &mut core,
                            config: &mut config,
                            workspace: &workspace,
                            identity: &identity,
                            ui: &mut ui,
                            profiler: &mut profiler,
                            model_client: &mut model_client,
                        };
                        let mut outcome = match toolgen_runner.run(&request) {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                ui.emit_toolgen_failure(
                                    &error,
                                    core.tool_repo()
                                        .list()
                                        .map(|items| items.len())
                                        .unwrap_or(0),
                                );
                                let mut outcome = TurnOutcome::final_response(
                                    "",
                                    UsageStats::zero(),
                                    None,
                                    None,
                                    Duration::ZERO,
                                );
                                outcome.toolgen_retrospect =
                                    format!("ToolGen could not publish a verified tool: {error}");
                                outcome
                            }
                        };
                        outcome.toolgen_retrospect = outcome.toolgen_retrospect.trim().to_string();
                        ui.current_turn_active = None;
                        drop(working);
                        has_running_shell_jobs = !outcome.running_jobs.is_empty();
                        let _ = event_tx.send(CoreSessionWorkerEvent::TurnFinished { outcome });
                    }
                    CoreSessionWorkerCommand::Rename {
                        display_name,
                        assistant_speaker_name: updated_assistant_speaker_name,
                    } => {
                        identity.rename(display_name);
                        assistant_speaker_name = updated_assistant_speaker_name
                            .unwrap_or_else(|| identity.display_name.clone());
                        core.set_assistant_speaker_name(&assistant_speaker_name);
                        let event = core_initialized_topic_event_with_worker(
                            &identity.session_id,
                            core.profile(),
                            core.response_protocol_name(),
                            core.max_llm_input_tokens(),
                            core.configured_round_budget(),
                            core.capability_tool_count(),
                            core.capability_skill_count(),
                            Some(&identity),
                            Some(&workspace),
                            Some(core.dynamic_context_summary()),
                        )
                        .with_worker_scope(&identity.context_id, &identity.worker_id);
                        let _ = event_tx.send(CoreSessionWorkerEvent::Topics(vec![event]));
                    }
                    CoreSessionWorkerCommand::ChangeCwd {
                        current_dir,
                        result_tx,
                    } => {
                        let result = core
                            .change_prompt_cwd(current_dir.display().to_string())
                            .inspect(|canonical| {
                                workspace.current_dir = Some(canonical.clone());
                            });
                        if let Err(error) = &result {
                            let _ = event_tx.send(CoreSessionWorkerEvent::ModelError {
                                error: error.clone(),
                            });
                        }
                        let _ = result_tx.send(result);
                    }
                    CoreSessionWorkerCommand::UpdateBashApproval { mode } => {
                        core.set_bash_approval_mode(mode);
                        core.set_self_tool_runtime_param(
                            "TIMEM_BASH_APPROVAL",
                            agent_core::bash_approval_mode_label(mode),
                        );
                        core.notify_runtime_config_changed();
                    }
                    CoreSessionWorkerCommand::RuntimeConfigUpdated
                    | CoreSessionWorkerCommand::MaxRoundsUpdated => {
                        ui.apply_pending_runtime_updates(&mut core, &mut config);
                    }
                    CoreSessionWorkerCommand::UpdateApiKey { api_key } => {
                        config.api_key = api_key;
                        core.notify_runtime_config_changed();
                    }
                    CoreSessionWorkerCommand::UpdateHttpHeaders { http_headers } => {
                        config.http_headers = http_headers;
                        core.notify_runtime_config_changed();
                    }
                    CoreSessionWorkerCommand::UpdateRequestFields { request_fields } => {
                        config.request_fields = request_fields;
                        core.notify_runtime_config_changed();
                    }
                    CoreSessionWorkerCommand::UpdateMcp {
                        base_capabilities,
                        runtime,
                        servers,
                        tools,
                        instructions,
                    } => {
                        if let Err(error) = core.apply_mcp_update_with_instructions(
                            base_capabilities,
                            runtime,
                            servers,
                            tools,
                            instructions,
                        ) {
                            let _ = event_tx.send(CoreSessionWorkerEvent::ModelError { error });
                        }
                    }
                    CoreSessionWorkerCommand::Shutdown => break,
                }
            }
            let _ = event_tx.send(CoreSessionWorkerEvent::WorkerStopped);
        });

        Self {
            handle,
            event_rx,
            join: Some(join),
        }
    }

    pub fn handle(&self) -> CoreSessionWorkerHandle {
        self.handle.clone()
    }

    pub fn events(&self) -> &Receiver<CoreSessionWorkerEvent> {
        &self.event_rx
    }

    pub fn shutdown(mut self) -> Result<(), String> {
        self.handle.cancel_current_turn();
        let _ = self.handle.request_shutdown();
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| "core_session_worker_thread_panicked".to_string())?;
        }
        Ok(())
    }

    pub fn shutdown_detached(mut self) {
        self.handle.cancel_current_turn();
        let _ = self.handle.request_shutdown();
        let _ = self.join.take();
    }
}

impl Drop for CoreSessionWorker {
    fn drop(&mut self) {
        self.handle.cancel_current_turn();
        let _ = self.handle.request_shutdown();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct WorkerTurnUi {
    event_tx: Sender<CoreSessionWorkerEvent>,
    session_id: String,
    context_id: String,
    worker_id: String,
    supplement_mailbox: Arc<Mutex<SupplementMailbox>>,
    cancel_requested: Arc<AtomicBool>,
    reply_rx: Receiver<TopicReply>,
    runtime: CoreSessionWorkerRuntime,
    current_turn_active: Option<Arc<AtomicBool>>,
    phase: Option<String>,
    accept_supplements: bool,
    continue_supplements_after_final_answer: bool,
    pending_bash_always_allow: bool,
    pending_runtime_updates: Arc<Mutex<Vec<PendingRuntimeUpdate>>>,
    interaction_profile: Option<agent_core::InteractionProfile>,
}

fn toolgen_completion_instruction(protocol: ResponseProtocolKind) -> &'static str {
    match protocol {
        ResponseProtocolKind::Xml => TOOLGEN_XML_COMPLETION,
        ResponseProtocolKind::Json => TOOLGEN_JSON_COMPLETION,
    }
}

struct ToolGenRunner<'a, M: ModelClient> {
    core: &'a mut AgentCore,
    config: &'a mut ModelServiceConfig,
    workspace: &'a CoreSessionWorkerWorkspace,
    identity: &'a CoreSessionWorkerIdentity,
    ui: &'a mut WorkerTurnUi,
    profiler: &'a mut RuntimeProfiler,
    model_client: &'a mut M,
}

impl<M: ModelClient> ToolGenRunner<'_, M> {
    fn run(self, request: &ToolGenRequest) -> Result<TurnOutcome, String> {
        let Self {
            core,
            config,
            workspace,
            identity,
            ui,
            profiler,
            model_client,
        } = self;
        let repo = core.tool_repo();
        let before = repo.list()?;
        let draft = repo.create_draft()?;
        if let Err(error) = core.enable_toolgen_capability() {
            let _ = repo.discard_draft(&draft);
            return Err(error);
        }
        let system_instruction = format!(
        "{TOOLGEN_CONTEXT_INSTRUCTIONS}\n\n{}\n\nWrite the new tool files only in this temporary staging directory:\n{}\n\nExisting verified tools for this Session are available here:\n{}\n\nCurrent working directory:\n{}",
        toolgen_completion_instruction(config.response_protocol),
        draft.display(),
        repo.root().display(),
        core.current_prompt_cwd().display(),
    );
        core.submit_prompt_component(
            agent_core::PromptComponentRole::system(),
            "runtime_note",
            system_instruction,
            "toolgen_request",
        );
        ui.begin_toolgen_run(before.len());
        let mut input = request.user_instruction.clone().unwrap_or_default();
        let mut additional_context = None;
        let mut outcome = loop {
            let current = run_session_turn_with_model_client(
                core,
                config,
                TurnInput {
                    input: &input,
                    session: &identity.context_id,
                    audit_file: &workspace.audit_file,
                    runtime: &workspace.runtime,
                    run_bash_target: &workspace.run_bash_target,
                    additional_context: additional_context.as_deref(),
                },
                ui,
                Some(profiler),
                model_client,
            );
            if current.stop_summary.is_some() || !ui.continue_supplements_after_final_answer {
                let supplements = ui.close_supplements_for_main_context();
                if !supplements.is_empty() {
                    let _ = ui
                        .event_tx
                        .send(CoreSessionWorkerEvent::UnconsumedSupplements {
                            supplements: supplements
                                .into_iter()
                                .map(|supplement| supplement.text)
                                .collect(),
                        });
                }
                break current;
            }
            let supplements = ui.take_or_close_supplements_for_main_context();
            if supplements.is_empty() {
                break current;
            }
            input = supplements
                .iter()
                .map(|supplement| supplement.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            additional_context = agent_core::combine_additional_contexts(
                supplements
                    .iter()
                    .filter_map(|supplement| supplement.additional_context.as_deref())
                    .map(Some),
            );
        };
        core.disable_toolgen_capability();
        let after = match repo.list() {
            Ok(after) => after,
            Err(error) => {
                let _ = repo.discard_draft(&draft);
                ui.finish_toolgen_run(
                    before.len(),
                    None,
                    &outcome.toolgen_retrospect,
                    Some(&error),
                );
                return Err(error);
            }
        };
        let published = after.iter().find(|tool| {
            !before
                .iter()
                .any(|old| old.tool_id == tool.tool_id && old.updated_at_ms == tool.updated_at_ms)
        });
        if published.is_none() {
            let _ = repo.discard_draft(&draft);
            if outcome.toolgen_retrospect.trim().is_empty() {
                outcome.toolgen_retrospect = toolgen_failure_detail(&outcome)
                    .map(|detail| format!("ToolGen did not publish a verified tool: {detail}"))
                    .unwrap_or_else(|| "ToolGen did not publish a verified tool.".to_string());
            }
        }
        let completion_error = if published.is_none() {
            Some(
                toolgen_failure_detail(&outcome)
                    .unwrap_or_else(|| "toolgen_no_verified_tool".to_string()),
            )
        } else {
            None
        };
        ui.finish_toolgen_run(
            after.len(),
            published,
            &outcome.toolgen_retrospect,
            completion_error.as_deref(),
        );
        Ok(outcome)
    }
}

fn toolgen_failure_detail(outcome: &TurnOutcome) -> Option<String> {
    let summary = outcome.stop_summary.as_ref()?;
    Some(match &summary.detail {
        TurnStopDetail::ModelError { error } => error.clone(),
        TurnStopDetail::RoundLimit { max_rounds } => {
            format!("toolgen_round_limit_reached:max_rounds={max_rounds}")
        }
        TurnStopDetail::OutputLimit { current_tokens } => {
            format!("toolgen_output_limit_reached:current_tokens={current_tokens}")
        }
        TurnStopDetail::ProtocolRepairFailure {
            first_issue,
            final_issue,
            truncated,
        } => format!(
            "toolgen_protocol_repair_failed:first_issue={first_issue},final_issue={final_issue},truncated={truncated}"
        ),
        TurnStopDetail::None => format!("toolgen_run_stopped:{:?}", summary.stop_reason),
    })
}

fn apply_worker_runtime_update(
    core: &mut AgentCore,
    config: &mut ModelServiceConfig,
    update: PendingRuntimeUpdate,
) {
    use agent_core::RuntimeConfigField;

    match update {
        PendingRuntimeUpdate::Config { field, value } => match field {
            RuntimeConfigField::Model => {
                config.model = value;
                core.set_self_tool_runtime_param(field.label(), config.model.clone());
            }
            RuntimeConfigField::ApiProtocol => {
                if let Ok(protocol) = agent_core::parse_api_protocol(&value) {
                    config.api_protocol = protocol;
                    core.set_self_tool_runtime_param(field.label(), config.api_protocol.label());
                }
            }
            RuntimeConfigField::ResponseProtocol => {
                config.response_protocol = agent_core::ResponseProtocolKind::from_name(&value);
                core.set_response_protocol(config.response_protocol);
                core.set_self_tool_runtime_param(field.label(), config.response_protocol.name());
            }
            RuntimeConfigField::BaseUrl => {
                config.base_url = value;
                core.set_self_tool_runtime_param(field.label(), config.base_url.clone());
            }
            RuntimeConfigField::MaxInput => {
                if let Some(tokens) = agent_core::parse_token_count(&value) {
                    let tokens = tokens.max(3_000);
                    config.max_llm_input_tokens = tokens;
                    core.set_max_llm_input_tokens(tokens);
                    core.set_self_tool_runtime_param(field.label(), tokens.to_string());
                }
            }
            RuntimeConfigField::MaxOutput => {
                if let Some(tokens) = agent_core::parse_token_count(&value) {
                    config.max_llm_output_tokens = tokens.max(512);
                    core.set_self_tool_runtime_param(
                        field.label(),
                        config.max_llm_output_tokens.to_string(),
                    );
                }
            }
            RuntimeConfigField::BashApproval => {
                let mode = match value.trim().to_lowercase().as_str() {
                    "approve" => Some(agent_core::BashApprovalMode::Approve),
                    "ask" => Some(agent_core::BashApprovalMode::Ask),
                    _ => None,
                };
                if let Some(mode) = mode {
                    core.set_bash_approval_mode(mode);
                    core.set_self_tool_runtime_param(
                        field.label(),
                        agent_core::bash_approval_mode_label(mode),
                    );
                }
            }
            RuntimeConfigField::WorkInstructions => {
                core.set_self_tool_runtime_param(field.label(), value);
            }
        },
        PendingRuntimeUpdate::OpenAiCompatible { key, value } => {
            if agent_core::apply_openai_compatible_env_value(
                &mut config.openai_compatible,
                &key,
                &value,
            )
            .unwrap_or(false)
            {
                core.set_self_tool_runtime_param(&key, value);
            }
        }
        PendingRuntimeUpdate::MaxRounds(max_rounds) => core.set_max_rounds(max_rounds),
    }
    core.notify_runtime_config_changed();
}

impl TurnUi for WorkerTurnUi {
    fn on_turn_projection(&mut self, projection: &agent_core::TurnProjection) {
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::TurnProjection(projection.clone()));
    }

    fn apply_pending_runtime_updates(
        &mut self,
        core: &mut AgentCore,
        config: &mut ModelServiceConfig,
    ) -> bool {
        let updates = self
            .pending_runtime_updates
            .lock()
            .map(|mut updates| std::mem::take(&mut *updates))
            .unwrap_or_default();
        let changed = !updates.is_empty();
        for update in updates {
            apply_worker_runtime_update(core, config, update);
        }
        changed
    }

    fn is_cancel_requested(&mut self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }

    fn take_cancel_request(&mut self) -> bool {
        self.cancel_requested.swap(false, Ordering::SeqCst)
    }

    fn drain_user_supplements_with_context(&mut self) -> Vec<agent_core::UserSupplement> {
        if !self.accept_supplements {
            return Vec::new();
        }
        self.supplement_mailbox
            .lock()
            .map(|mut mailbox| self.accept_queued_supplements(std::mem::take(&mut mailbox.queue)))
            .unwrap_or_default()
    }

    fn continue_supplements_after_final_answer(&self) -> bool {
        self.continue_supplements_after_final_answer
    }

    fn on_model_api_request(
        &mut self,
        round: u32,
        request: &agent_core::ModelInteractionRequest,
        api_payload: &serde_json::Value,
    ) {
        let _ = self.event_tx.send(CoreSessionWorkerEvent::ModelRequest {
            round,
            emitted_at_ms: unix_timestamp_ms(),
            prompt: request.rendered_prompt.clone(),
            interaction_profile: self.interaction_profile.clone(),
            interaction_request: Some(Box::new(request.clone())),
            api_payload: Some(Box::new(api_payload.clone())),
        });
    }

    fn on_model_request_completed(&mut self, latency: Duration) {
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::ModelRequestCompleted { latency });
    }

    fn on_model_response_parsed(&mut self, tool_count: usize) {
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::ModelResponseParsed { tool_count });
    }

    fn on_interaction_profile(&mut self, profile: &agent_core::InteractionProfile) {
        self.interaction_profile = Some(profile.clone());
    }

    fn on_model_interaction_response(&mut self, round: u32, response: &agent_core::LlmResponse) {
        let _ = self.event_tx.send(CoreSessionWorkerEvent::ModelResponse {
            round,
            usage: response.usage.clone(),
            content: response.content.clone(),
            tool_calls: response.tool_calls.clone(),
            truncated: response.truncated,
            runtime_phase: self.phase.clone(),
        });
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        let events = self
            .runtime
            .enrich_topic_events(
                &self.session_id,
                events.to_vec(),
                self.current_turn_active.as_ref(),
            )
            .into_iter()
            .map(|mut event| {
                event.session_id = self.session_id.clone();
                if let Some(phase) = self.phase.as_deref() {
                    event.topic.attributes["runtime_phase"] = serde_json::json!(phase);
                    event.payload["runtime_phase"] = serde_json::json!(phase);
                }
                event.with_worker_scope(&self.context_id, &self.worker_id)
            })
            .collect();
        let _ = self.event_tx.send(CoreSessionWorkerEvent::Topics(events));
    }

    fn on_model_error(&mut self, error: &str) {
        let _ = self.event_tx.send(CoreSessionWorkerEvent::ModelError {
            error: error.to_string(),
        });
    }

    fn on_model_retry(&mut self, attempt: u32, max_attempts: u32, delay: Duration, error: &str) {
        let _ = self.event_tx.send(CoreSessionWorkerEvent::ModelRetry {
            attempt,
            max_attempts,
            delay,
            error: error.to_string(),
        });
    }

    fn request_host_decision_topic(
        &mut self,
        _session: &str,
        request: HostDecisionRequest,
    ) -> HostDecision {
        let mut event = request
            .topic_event(&self.session_id)
            .with_worker_scope(&self.context_id, &self.worker_id);
        if let Some(phase) = self.phase.as_deref() {
            event.topic.attributes["runtime_phase"] = serde_json::json!(phase);
            event.payload["runtime_phase"] = serde_json::json!(phase);
        }
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::Topics(vec![event.clone()]));
        let timeout = request.timeout();
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        loop {
            if self.cancel_requested.load(Ordering::SeqCst) {
                return request.safe_default().into();
            }
            let wait_for = match deadline {
                Some(deadline) => {
                    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                        return request.safe_default().into();
                    };
                    remaining.min(Duration::from_millis(50))
                }
                None => Duration::from_millis(50),
            };
            let reply = match self.reply_rx.recv_timeout(wait_for) {
                Ok(reply) => Some(reply),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return request.safe_default().into(),
            };
            let Some(reply) = reply else {
                continue;
            };
            if let Ok(decision) = agent_core::resolve_topic_reply(&event, None, &reply) {
                if reply.always_allow {
                    self.pending_bash_always_allow = true;
                }
                return decision;
            }
        }
    }

    fn take_bash_always_allow(&mut self) -> bool {
        let flag = self.pending_bash_always_allow;
        self.pending_bash_always_allow = false;
        flag
    }
}

impl agent_core::ActionRuntime for WorkerTurnUi {
    fn should_cancel(&mut self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        TurnUi::on_core_topic_events(self, events);
    }

    fn take_bash_always_allow(&mut self) -> bool {
        let flag = self.pending_bash_always_allow;
        self.pending_bash_always_allow = false;
        flag
    }
}

impl WorkerTurnUi {
    fn accept_queued_supplements(
        &self,
        queued: Vec<QueuedSupplement>,
    ) -> Vec<agent_core::UserSupplement> {
        queued
            .into_iter()
            .map(|queued| {
                if let Some(command_id) = queued.command_id {
                    let _ = self
                        .event_tx
                        .send(CoreSessionWorkerEvent::CommandAccepted { command_id });
                }
                agent_core::UserSupplement::new(queued.text, queued.additional_context)
            })
            .collect()
    }

    fn take_or_close_supplements_for_main_context(&mut self) -> Vec<agent_core::UserSupplement> {
        self.supplement_mailbox
            .lock()
            .map(|mut mailbox| {
                let supplements = std::mem::take(&mut mailbox.queue);
                if supplements.is_empty() {
                    mailbox.accepting = false;
                }
                self.accept_queued_supplements(supplements)
            })
            .unwrap_or_default()
    }

    fn close_supplements_for_main_context(&mut self) -> Vec<agent_core::UserSupplement> {
        self.supplement_mailbox
            .lock()
            .map(|mut mailbox| {
                mailbox.accepting = false;
                self.accept_queued_supplements(std::mem::take(&mut mailbox.queue))
            })
            .unwrap_or_default()
    }

    fn begin_toolgen_run(&mut self, tool_count: usize) {
        self.phase = Some("toolgen".to_string());
        let event = agent_core::toolgen_topic_event(
            &self.session_id,
            "started",
            tool_count,
            None,
            None,
            None,
        )
        .with_worker_scope(&self.context_id, &self.worker_id);
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::Topics(vec![event]));
    }

    fn finish_toolgen_run(
        &mut self,
        tool_count: usize,
        tool: Option<&agent_core::ToolSummary>,
        retrospect: &str,
        error: Option<&str>,
    ) {
        let phase = if error.is_some() || tool.is_none() {
            "failed"
        } else {
            "published"
        };
        let event = agent_core::toolgen_topic_event(
            &self.session_id,
            phase,
            tool_count,
            tool,
            (!retrospect.trim().is_empty()).then_some(retrospect.trim()),
            error,
        )
        .with_worker_scope(&self.context_id, &self.worker_id);
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::Topics(vec![event]));
        self.phase = None;
    }

    fn emit_toolgen_failure(&mut self, error: &str, tool_count: usize) {
        let event = agent_core::toolgen_topic_event(
            &self.session_id,
            "failed",
            tool_count,
            None,
            None,
            Some(error),
        )
        .with_worker_scope(&self.context_id, &self.worker_id);
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::Topics(vec![event]));
    }
}

#[cfg(test)]
#[path = "../tests/unit/session_worker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/message_queue_tests.rs"]
mod message_queue_tests;

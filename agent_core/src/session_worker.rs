use crate::{
    core_initialized_topic_event_with_worker, run_session_turn_with_model_client, AgentCore,
    CoreGlobalWorkerStatus, CoreSessionWorkerIdentity, CoreSessionWorkerWorkspace, CoreTopicEvent,
    HostDecision, HostDecisionRequest, ModelClient, ProviderConfig, ProviderModelClient,
    RuntimeProfiler, TopicReply, TurnInput, TurnOutcome, TurnUi, UsageStats,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CoreSessionWorkerConfig {
    pub identity: CoreSessionWorkerIdentity,
    pub workspace: CoreSessionWorkerWorkspace,
}

impl CoreSessionWorkerConfig {
    pub fn new(identity: CoreSessionWorkerIdentity, workspace: CoreSessionWorkerWorkspace) -> Self {
        Self {
            identity,
            workspace,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.identity.session_id
    }
}

#[derive(Clone, Debug)]
pub struct CoreSessionWorkerRuntime {
    working_workers: Arc<AtomicUsize>,
}

impl CoreSessionWorkerRuntime {
    pub fn new() -> Self {
        Self {
            working_workers: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn working_worker_count(&self) -> usize {
        self.working_workers.load(Ordering::SeqCst)
    }

    fn begin_worker_turn(&self) -> WorkingWorkerGuard {
        let active = Arc::new(AtomicBool::new(true));
        self.working_workers.fetch_add(1, Ordering::SeqCst);
        WorkingWorkerGuard {
            working_workers: Arc::clone(&self.working_workers),
            active,
        }
    }

    fn finish_worker_turn_if_active(&self, active: &Arc<AtomicBool>) -> usize {
        if active.swap(false, Ordering::SeqCst) {
            self.working_workers
                .fetch_sub(1, Ordering::SeqCst)
                .saturating_sub(1)
        } else {
            self.working_worker_count()
        }
    }

    fn model_response_global_status(
        &self,
        continue_work: bool,
        active: Option<&Arc<AtomicBool>>,
    ) -> CoreGlobalWorkerStatus {
        let visible_count = if continue_work {
            self.working_worker_count()
        } else if let Some(active) = active {
            self.finish_worker_turn_if_active(active)
        } else {
            self.working_worker_count()
        };
        CoreGlobalWorkerStatus::new(visible_count)
    }

    fn enrich_topic_events(
        &self,
        events: Vec<CoreTopicEvent>,
        active: Option<&Arc<AtomicBool>>,
    ) -> Vec<CoreTopicEvent> {
        events
            .into_iter()
            .map(|event| {
                let Some(model_response) = event.as_model_response() else {
                    return event;
                };
                event.with_global_worker_status(
                    self.model_response_global_status(model_response.continue_work, active),
                )
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
    working_workers: Arc<AtomicUsize>,
    active: Arc<AtomicBool>,
}

impl WorkingWorkerGuard {
    fn active_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.active)
    }
}

impl Drop for WorkingWorkerGuard {
    fn drop(&mut self) {
        if self.active.swap(false, Ordering::SeqCst) {
            self.working_workers.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[derive(Debug, Clone)]
pub enum CoreSessionWorkerEvent {
    Topics(Vec<CoreTopicEvent>),
    ModelRequest {
        round: u32,
    },
    ModelResponse {
        round: u32,
        usage: UsageStats,
    },
    ModelResponseDiscarded {
        round: u32,
        reason: String,
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
    TurnFinished {
        outcome: TurnOutcome,
    },
    WorkerStopped,
}

enum CoreSessionWorkerCommand {
    RunTurn {
        input: String,
        additional_context: Option<String>,
    },
    Rename {
        display_name: String,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct CoreSessionWorkerHandle {
    command_tx: Sender<CoreSessionWorkerCommand>,
    supplement_queue: Arc<Mutex<Vec<String>>>,
    cancel_requested: Arc<AtomicBool>,
    shutdown_requested: Arc<AtomicBool>,
    reply_tx: Sender<TopicReply>,
}

impl CoreSessionWorkerHandle {
    pub fn run_turn(
        &self,
        input: impl Into<String>,
        additional_context: Option<String>,
    ) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.cancel_requested.store(false, Ordering::SeqCst);
        self.command_tx
            .send(CoreSessionWorkerCommand::RunTurn {
                input: input.into(),
                additional_context,
            })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn add_user_supplement(&self, supplement: impl Into<String>) {
        if let Ok(mut queue) = self.supplement_queue.lock() {
            queue.push(supplement.into());
        }
    }

    pub fn cancel_current_turn(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
    }

    pub fn reply_to_request(&self, reply: TopicReply) -> Result<(), String> {
        self.reply_tx
            .send(reply)
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn request_shutdown(&self) -> Result<(), String> {
        self.shutdown_requested.store(true, Ordering::SeqCst);
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.command_tx
            .send(CoreSessionWorkerCommand::Shutdown)
            .map_err(|_| "core_session_worker_stopped".to_string())
    }

    pub fn rename(&self, display_name: impl Into<String>) -> Result<(), String> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return Err("core_session_worker_stopped".to_string());
        }
        self.command_tx
            .send(CoreSessionWorkerCommand::Rename {
                display_name: display_name.into(),
            })
            .map_err(|_| "core_session_worker_stopped".to_string())
    }
}

pub struct CoreSessionWorker {
    handle: CoreSessionWorkerHandle,
    event_rx: Receiver<CoreSessionWorkerEvent>,
    join: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSessionWorkerLifecycleState {
    Running,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSessionWorkerStatus {
    pub identity: CoreSessionWorkerIdentity,
    pub state: CoreSessionWorkerLifecycleState,
}

struct ManagedSessionWorker {
    identity: CoreSessionWorkerIdentity,
    state: CoreSessionWorkerLifecycleState,
    worker: CoreSessionWorker,
}

pub struct CoreSessionWorkerManager {
    runtime: CoreSessionWorkerRuntime,
    next_ordinal: u32,
    workers: BTreeMap<String, ManagedSessionWorker>,
}

impl CoreSessionWorkerManager {
    pub fn new() -> Self {
        Self {
            runtime: CoreSessionWorkerRuntime::new(),
            next_ordinal: 0,
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

    pub fn handle(&self, session_id: &str) -> Option<CoreSessionWorkerHandle> {
        self.workers
            .get(session_id)
            .map(|worker| worker.worker.handle())
    }

    pub fn ensure_default_worker(
        &mut self,
        core: AgentCore,
        config: ProviderConfig,
        workspace: CoreSessionWorkerWorkspace,
    ) -> Result<String, String> {
        if let Some(session_id) = self.workers.keys().next() {
            return Ok(session_id.clone());
        }
        self.spawn_worker(core, config, workspace, None, None)
    }

    pub fn ensure_default_worker_with_model_client<M>(
        &mut self,
        core: AgentCore,
        config: ProviderConfig,
        workspace: CoreSessionWorkerWorkspace,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        if let Some(session_id) = self.workers.keys().next() {
            return Ok(session_id.clone());
        }
        self.spawn_worker_with_model_client(core, config, workspace, None, None, model_client)
    }

    pub fn spawn_worker(
        &mut self,
        core: AgentCore,
        config: ProviderConfig,
        workspace: CoreSessionWorkerWorkspace,
        display_name: Option<String>,
        parent_session_id: Option<String>,
    ) -> Result<String, String> {
        self.spawn_worker_with_model_client(
            core,
            config,
            workspace,
            display_name,
            parent_session_id,
            ProviderModelClient,
        )
    }

    pub fn spawn_worker_with_model_client<M>(
        &mut self,
        core: AgentCore,
        config: ProviderConfig,
        workspace: CoreSessionWorkerWorkspace,
        display_name: Option<String>,
        parent_session_id: Option<String>,
        model_client: M,
    ) -> Result<String, String>
    where
        M: ModelClient + Send + 'static,
    {
        let ordinal = self.next_ordinal;
        self.next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .ok_or_else(|| "session_worker_ordinal_overflow".to_string())?;
        let session_id = format!("session_{ordinal}");
        let identity = CoreSessionWorkerIdentity::new(
            session_id.clone(),
            ordinal,
            display_name,
            parent_session_id,
        );
        let worker = CoreSessionWorker::spawn_with_runtime_model_client(
            core,
            config,
            CoreSessionWorkerConfig::new(identity.clone(), workspace),
            self.runtime.clone(),
            model_client,
        );
        self.workers.insert(
            session_id.clone(),
            ManagedSessionWorker {
                identity,
                state: CoreSessionWorkerLifecycleState::Running,
                worker,
            },
        );
        Ok(session_id)
    }

    pub fn try_recv_event(&mut self, session_id: &str) -> Option<CoreSessionWorkerEvent> {
        let managed = self.workers.get_mut(session_id)?;
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

    pub fn request_shutdown(&mut self, session_id: &str) -> Result<(), String> {
        let managed = self
            .workers
            .get_mut(session_id)
            .ok_or_else(|| "session_worker_not_found".to_string())?;
        managed.state = CoreSessionWorkerLifecycleState::Stopping;
        managed.worker.handle().request_shutdown()
    }

    pub fn remove_stopped(&mut self, session_id: &str) -> Result<(), String> {
        let Some(managed) = self.workers.get(session_id) else {
            return Err("session_worker_not_found".to_string());
        };
        if managed.state != CoreSessionWorkerLifecycleState::Stopped {
            return Err("session_worker_not_stopped".to_string());
        }
        let managed = self.workers.remove(session_id).unwrap();
        managed.worker.shutdown()
    }

    pub fn shutdown_all(mut self) -> Result<(), String> {
        for managed in self.workers.values_mut() {
            let _ = managed.worker.handle().request_shutdown();
            managed.state = CoreSessionWorkerLifecycleState::Stopping;
        }
        let mut first_error = None;
        for (_session_id, managed) in self.workers {
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
}

impl Default for CoreSessionWorkerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreSessionWorker {
    pub fn spawn(
        core: AgentCore,
        config: ProviderConfig,
        worker_config: CoreSessionWorkerConfig,
    ) -> Self {
        Self::spawn_with_runtime_model_client(
            core,
            config,
            worker_config,
            CoreSessionWorkerRuntime::new(),
            ProviderModelClient,
        )
    }

    pub fn spawn_with_model_client<M>(
        core: AgentCore,
        config: ProviderConfig,
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
        mut config: ProviderConfig,
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
        let supplement_queue = Arc::new(Mutex::new(Vec::new()));
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let handle = CoreSessionWorkerHandle {
            command_tx,
            supplement_queue: Arc::clone(&supplement_queue),
            cancel_requested: Arc::clone(&cancel_requested),
            shutdown_requested: Arc::clone(&shutdown_requested),
            reply_tx,
        };
        let join = thread::spawn(move || {
            let mut identity = worker_config.identity.clone();
            let workspace = worker_config.workspace.clone();
            core.set_assistant_speaker_name(&identity.display_name);
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
            );
            let _ = event_tx.send(CoreSessionWorkerEvent::Topics(vec![init_event]));
            let mut profiler = RuntimeProfiler::default();
            let mut ui = WorkerTurnUi {
                event_tx: event_tx.clone(),
                supplement_queue,
                cancel_requested,
                reply_rx,
                runtime: runtime.clone(),
                current_turn_active: None,
            };

            while let Ok(command) = command_rx.recv() {
                match command {
                    CoreSessionWorkerCommand::RunTurn { .. }
                    | CoreSessionWorkerCommand::Rename { .. }
                        if shutdown_requested.load(Ordering::SeqCst) =>
                    {
                        break;
                    }
                    CoreSessionWorkerCommand::RunTurn {
                        input,
                        additional_context,
                    } => {
                        let session_id = identity.session_id.clone();
                        let outcome = {
                            let working = runtime.begin_worker_turn();
                            ui.current_turn_active = Some(working.active_handle());
                            let outcome = run_session_turn_with_model_client(
                                &mut core,
                                &mut config,
                                TurnInput {
                                    input: &input,
                                    session: &session_id,
                                    audit_file: &workspace.audit_file,
                                    runtime: &workspace.runtime,
                                    run_bash_target: &workspace.run_bash_target,
                                    additional_context: additional_context.as_deref(),
                                },
                                &mut ui,
                                Some(&mut profiler),
                                &mut model_client,
                            );
                            ui.current_turn_active = None;
                            drop(working);
                            outcome
                        };
                        let _ = event_tx.send(CoreSessionWorkerEvent::TurnFinished { outcome });
                    }
                    CoreSessionWorkerCommand::Rename { display_name } => {
                        identity.rename(display_name);
                        core.set_assistant_speaker_name(&identity.display_name);
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
                        );
                        let _ = event_tx.send(CoreSessionWorkerEvent::Topics(vec![event]));
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
    supplement_queue: Arc<Mutex<Vec<String>>>,
    cancel_requested: Arc<AtomicBool>,
    reply_rx: Receiver<TopicReply>,
    runtime: CoreSessionWorkerRuntime,
    current_turn_active: Option<Arc<AtomicBool>>,
}

impl TurnUi for WorkerTurnUi {
    fn is_cancel_requested(&mut self) -> bool {
        self.cancel_requested.load(Ordering::SeqCst)
    }

    fn take_cancel_request(&mut self) -> bool {
        self.cancel_requested.swap(false, Ordering::SeqCst)
    }

    fn drain_user_supplements(&mut self) -> Vec<String> {
        self.supplement_queue
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }

    fn on_model_request(&mut self, round: u32, _prompt: &str) {
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::ModelRequest { round });
    }

    fn on_model_response(&mut self, round: u32, usage: &UsageStats, _content: &str) {
        let _ = self.event_tx.send(CoreSessionWorkerEvent::ModelResponse {
            round,
            usage: usage.clone(),
        });
    }

    fn on_model_response_discarded(&mut self, round: u32, reason: &str) {
        let _ = self
            .event_tx
            .send(CoreSessionWorkerEvent::ModelResponseDiscarded {
                round,
                reason: reason.to_string(),
            });
    }

    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        let events = self
            .runtime
            .enrich_topic_events(events.to_vec(), self.current_turn_active.as_ref());
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
        session: &str,
        request: HostDecisionRequest,
    ) -> HostDecision {
        let event = request.topic_event(session);
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
            if let Ok(decision) = crate::resolve_topic_reply(&event, None, &reply) {
                return decision;
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/session_worker_tests.rs"]
mod tests;

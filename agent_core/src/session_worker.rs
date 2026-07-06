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
mod tests {
    use super::*;
    use crate::{ApiProtocol, BashApprovalMode, CoreProfile, LlmResponse, UsageStats};
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::Instant;

    fn tmp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "timem_session_worker_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_config() -> ProviderConfig {
        ProviderConfig {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            base_url: "http://127.0.0.1/v1".to_string(),
            api_key: "dummy".to_string(),
            timeout_secs: 10,
            max_llm_output_tokens: 10_000,
            max_llm_input_tokens: 100_000,
            api_protocol: ApiProtocol::OpenAiCompatible,
            response_protocol: crate::ResponseProtocolKind::Markdown,
        }
    }

    fn test_worker_config(
        dir: &std::path::Path,
        session_id: &str,
        ordinal: u32,
    ) -> CoreSessionWorkerConfig {
        CoreSessionWorkerConfig::new(
            CoreSessionWorkerIdentity::new(session_id, ordinal, None, None),
            CoreSessionWorkerWorkspace::new(
                dir,
                dir.join("audit").join("api_audit.json"),
                "test_worker",
                "test_machine",
            ),
        )
    }

    struct SupplementReplayModel {
        calls: Arc<Mutex<u32>>,
    }

    impl ModelClient for SupplementReplayModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &std::path::Path,
            should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let call_no = *calls;
            drop(calls);
            if call_no == 1 {
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(200) {
                    if should_cancel() {
                        return Err("cancelled_by_user".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            let has_supplement = prompt.contains("## USER") && prompt.contains("SUPPLEMENT");
            let content = if has_supplement {
                "## Status\nfinished\n\n## Final_Answer\nSUPPLEMENT_WORKER_OK"
            } else {
                "## Status\nfinished\n\n## Final_Answer\nSTALE"
            };
            Ok(LlmResponse {
                content: content.to_string(),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: if has_supplement { 1_200 } else { 1_000 },
                    completion_tokens: 10,
                    total_tokens: if has_supplement { 1_210 } else { 1_010 },
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    #[test]
    fn session_worker_emits_lifecycle_runs_turn_and_accepts_mid_turn_supplement() {
        let dir = tmp_dir("supplement");
        let core = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        let calls = Arc::new(Mutex::new(0));
        let worker = CoreSessionWorker::spawn_with_model_client(
            core,
            test_config(),
            test_worker_config(&dir, "session_worker_test", 1),
            SupplementReplayModel {
                calls: Arc::clone(&calls),
            },
        );
        let handle = worker.handle();

        let lifecycle = worker
            .events()
            .recv_timeout(Duration::from_secs(2))
            .expect("worker should emit lifecycle topic");
        match lifecycle {
            CoreSessionWorkerEvent::Topics(events) => {
                let lifecycle = events
                    .first()
                    .and_then(CoreTopicEvent::as_lifecycle)
                    .expect("first worker topic should be lifecycle initialized");
                assert_eq!(lifecycle.event, crate::CoreLifecycleEvent::Initialized);
                assert_eq!(lifecycle.profile.model, "test-model");
                assert_eq!(
                    lifecycle
                        .worker
                        .as_ref()
                        .map(|worker| worker.display_name.as_str()),
                    Some("ID1")
                );
                assert_eq!(lifecycle.context.unwrap().visible_delta_count, 0);
            }
            other => panic!("unexpected first worker event: {other:?}"),
        }

        handle
            .run_turn("请等待补充后回答。", None)
            .expect("worker should accept run_turn");
        loop {
            match worker
                .events()
                .recv_timeout(Duration::from_secs(2))
                .expect("worker should emit model request")
            {
                CoreSessionWorkerEvent::ModelRequest { round } => {
                    assert_eq!(round, 1);
                    handle.add_user_supplement("补充：最终答案必须使用 SUPPLEMENT_WORKER_OK。");
                    break;
                }
                CoreSessionWorkerEvent::Topics(_) => {}
                other => panic!("unexpected event before first model request: {other:?}"),
            }
        }

        let mut saw_discard = false;
        let outcome = loop {
            match worker
                .events()
                .recv_timeout(Duration::from_secs(5))
                .expect("worker should finish supplemented turn")
            {
                CoreSessionWorkerEvent::ModelResponseDiscarded { round, reason } => {
                    assert_eq!(round, 1);
                    assert_eq!(reason, "user_supplement_preempted_stale_response");
                    saw_discard = true;
                }
                CoreSessionWorkerEvent::TurnFinished { outcome } => break outcome,
                CoreSessionWorkerEvent::Topics(_)
                | CoreSessionWorkerEvent::ModelRequest { .. }
                | CoreSessionWorkerEvent::ModelResponse { .. } => {}
                other => panic!("unexpected worker event: {other:?}"),
            }
        };
        assert!(saw_discard);
        assert_eq!(outcome.text, "SUPPLEMENT_WORKER_OK");
        assert_eq!(outcome.stats.llm_calls, 2);
        assert_eq!(outcome.stats.prompt_tokens, 2_200);
        assert_eq!(*calls.lock().unwrap(), 2);

        handle.request_shutdown().unwrap();
        loop {
            match worker
                .events()
                .recv_timeout(Duration::from_secs(2))
                .expect("worker should stop")
            {
                CoreSessionWorkerEvent::WorkerStopped => break,
                CoreSessionWorkerEvent::Topics(_) => {}
                other => panic!("unexpected event while stopping worker: {other:?}"),
            }
        }
        worker.shutdown().unwrap();
    }

    #[test]
    fn session_worker_rename_emits_updated_identity_topic() {
        let dir = tmp_dir("rename");
        let core = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        let worker = CoreSessionWorker::spawn_with_model_client(
            core,
            test_config(),
            test_worker_config(&dir, "session_worker_rename", 3),
            SupplementReplayModel {
                calls: Arc::new(Mutex::new(0)),
            },
        );
        let handle = worker.handle();

        let lifecycle = worker
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        match lifecycle {
            CoreSessionWorkerEvent::Topics(events) => {
                let lifecycle = events[0].as_lifecycle().unwrap();
                assert_eq!(lifecycle.worker.unwrap().display_name, "ID3");
            }
            other => panic!("unexpected first worker event: {other:?}"),
        }

        handle.rename("日志分析").unwrap();
        let lifecycle = worker
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        match lifecycle {
            CoreSessionWorkerEvent::Topics(events) => {
                let lifecycle = events[0].as_lifecycle().unwrap();
                assert_eq!(lifecycle.worker.unwrap().display_name, "日志分析");
            }
            other => panic!("unexpected rename worker event: {other:?}"),
        }

        worker.shutdown().unwrap();
    }

    struct ManagerOkModel;

    impl ModelClient for ManagerOkModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            _prompt: &str,
            _audit_file: &std::path::Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            Ok(LlmResponse {
                content: "## Status\nfinished\n\n## Final_Answer\nMANAGER_OK".to_string(),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: 10,
                    completion_tokens: 2,
                    total_tokens: 12,
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    fn wait_for_manager_event(
        manager: &mut CoreSessionWorkerManager,
        session_id: &str,
        label: &str,
    ) -> CoreSessionWorkerEvent {
        let started = Instant::now();
        loop {
            if let Some(event) = manager.try_recv_event(session_id) {
                return event;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "{label} timed out waiting for manager event"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn session_worker_manager_allocates_id0_default_and_tracks_lifecycle() {
        let dir = tmp_dir("manager_default");
        let core = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        let mut manager = CoreSessionWorkerManager::new();
        let session_id = manager
            .ensure_default_worker_with_model_client(
                core,
                test_config(),
                CoreSessionWorkerWorkspace::new(
                    &dir,
                    dir.join("api_audit.jsonl"),
                    "test-runtime",
                    "local",
                ),
                ManagerOkModel,
            )
            .expect("manager should spawn default worker");
        assert_eq!(session_id, "session_0");
        assert_eq!(manager.worker_count(), 1);
        assert_eq!(manager.statuses()[0].identity.display_name, "ID0");
        assert_eq!(
            manager.statuses()[0].state,
            CoreSessionWorkerLifecycleState::Running
        );

        match wait_for_manager_event(&mut manager, &session_id, "manager lifecycle") {
            CoreSessionWorkerEvent::Topics(events) => {
                let lifecycle = events[0].as_lifecycle().unwrap();
                assert_eq!(lifecycle.worker.unwrap().display_name, "ID0");
            }
            other => panic!("unexpected manager lifecycle event: {other:?}"),
        }

        let handle = manager.handle(&session_id).expect("manager handle");
        handle
            .run_turn("hello through manager", None)
            .expect("manager worker should accept turn");
        let outcome = loop {
            match wait_for_manager_event(&mut manager, &session_id, "manager turn") {
                CoreSessionWorkerEvent::TurnFinished { outcome } => break outcome,
                CoreSessionWorkerEvent::Topics(_)
                | CoreSessionWorkerEvent::ModelRequest { .. }
                | CoreSessionWorkerEvent::ModelResponse { .. } => {}
                other => panic!("unexpected manager turn event: {other:?}"),
            }
        };
        assert_eq!(outcome.text, "MANAGER_OK");

        manager
            .request_shutdown(&session_id)
            .expect("manager should request shutdown");
        assert_eq!(
            manager.statuses()[0].state,
            CoreSessionWorkerLifecycleState::Stopping
        );
        loop {
            match wait_for_manager_event(&mut manager, &session_id, "manager shutdown") {
                CoreSessionWorkerEvent::WorkerStopped => break,
                CoreSessionWorkerEvent::Topics(_) => {}
                other => panic!("unexpected manager shutdown event: {other:?}"),
            }
        }
        assert_eq!(
            manager.statuses()[0].state,
            CoreSessionWorkerLifecycleState::Stopped
        );
        manager
            .remove_stopped(&session_id)
            .expect("stopped worker should be removable");
        assert_eq!(manager.worker_count(), 0);
    }

    #[test]
    fn session_worker_manager_allocates_multiple_workers_from_id0() {
        let mut manager = CoreSessionWorkerManager::new();
        let mut session_ids = Vec::new();
        for idx in 0..2 {
            let dir = tmp_dir(&format!("manager_multi_{idx}"));
            let core = AgentCore::new(
                "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
                CoreProfile {
                    name: "test".to_string(),
                    provider: "test".to_string(),
                    model: "test-model".to_string(),
                },
                &dir,
            );
            let session_id = manager
                .spawn_worker_with_model_client(
                    core,
                    test_config(),
                    CoreSessionWorkerWorkspace::new(
                        &dir,
                        dir.join("api_audit.jsonl"),
                        "test-runtime",
                        "local",
                    ),
                    None,
                    None,
                    ManagerOkModel,
                )
                .expect("manager should spawn worker");
            session_ids.push(session_id);
        }
        assert_eq!(session_ids, vec!["session_0", "session_1"]);
        let names = manager
            .statuses()
            .into_iter()
            .map(|status| status.identity.display_name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ID0", "ID1"]);
        manager.shutdown_all().unwrap();
    }

    struct BlockingManagerModel {
        release: Arc<AtomicBool>,
    }

    impl ModelClient for BlockingManagerModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            _prompt: &str,
            _audit_file: &std::path::Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            while !self.release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(LlmResponse {
                content: "## Status\nfinished\n\n## Final_Answer\nCOUNT_OK".to_string(),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: 10,
                    completion_tokens: 2,
                    total_tokens: 12,
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    #[test]
    fn session_worker_manager_tracks_global_working_count() {
        let release = Arc::new(AtomicBool::new(false));
        let mut manager = CoreSessionWorkerManager::new();
        let mut session_ids = Vec::new();
        for idx in 0..2 {
            let dir = tmp_dir(&format!("manager_count_{idx}"));
            let core = AgentCore::new(
                "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
                CoreProfile {
                    name: "test".to_string(),
                    provider: "test".to_string(),
                    model: "test-model".to_string(),
                },
                &dir,
            );
            let session_id = manager
                .spawn_worker_with_model_client(
                    core,
                    test_config(),
                    CoreSessionWorkerWorkspace::new(
                        &dir,
                        dir.join("api_audit.jsonl"),
                        "test-runtime",
                        "local",
                    ),
                    None,
                    None,
                    BlockingManagerModel {
                        release: Arc::clone(&release),
                    },
                )
                .unwrap();
            let _ = wait_for_manager_event(&mut manager, &session_id, "manager count lifecycle");
            manager
                .handle(&session_id)
                .unwrap()
                .run_turn(format!("count {idx}"), None)
                .unwrap();
            session_ids.push(session_id);
        }

        for session_id in &session_ids {
            loop {
                match wait_for_manager_event(&mut manager, session_id, "manager count request") {
                    CoreSessionWorkerEvent::ModelRequest { .. } => break,
                    CoreSessionWorkerEvent::Topics(_) => {}
                    other => panic!("unexpected manager count pre-release event: {other:?}"),
                }
            }
        }
        assert_eq!(manager.working_worker_count(), 2);

        release.store(true, Ordering::SeqCst);
        for session_id in &session_ids {
            loop {
                match wait_for_manager_event(&mut manager, session_id, "manager count finish") {
                    CoreSessionWorkerEvent::TurnFinished { outcome } => {
                        assert_eq!(outcome.text, "COUNT_OK");
                        break;
                    }
                    CoreSessionWorkerEvent::Topics(_)
                    | CoreSessionWorkerEvent::ModelResponse { .. } => {}
                    other => panic!("unexpected manager count finish event: {other:?}"),
                }
            }
        }
        assert_eq!(manager.working_worker_count(), 0);
        manager.shutdown_all().unwrap();
    }

    struct ApprovalReplayModel {
        calls: Arc<Mutex<u32>>,
    }

    impl ModelClient for ApprovalReplayModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &std::path::Path,
            should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            if should_cancel() {
                return Err("cancelled_by_user".to_string());
            }
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            let call_no = *calls;
            drop(calls);
            let content = if prompt.contains("denied_by_user") {
                "## Status\nfinished\n\n## Final_Answer\nDENIED_OK"
            } else {
                r#"## Progress
需要用户确认后执行本地命令。

## Working_Still_Action
```action
{
  "action": "run_bash",
  "intent": "Run a command that requires approval.",
  "args": {
    "cmd": "printf approval-worker-ok",
    "timeout_ms": 5000
  }
}
```"#
            };
            Ok(LlmResponse {
                content: content.to_string(),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: if call_no == 1 { 1_000 } else { 1_100 },
                    completion_tokens: 20,
                    total_tokens: if call_no == 1 { 1_020 } else { 1_120 },
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    struct AssistantHeadingModel {
        expected_heading: String,
        calls: u32,
    }

    impl ModelClient for AssistantHeadingModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &std::path::Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.calls += 1;
            if self.calls == 1 {
                assert!(!prompt.contains(&self.expected_heading));
            } else {
                assert!(
                    prompt.contains(&self.expected_heading),
                    "prompt should contain assistant heading {}:\n{}",
                    self.expected_heading,
                    prompt
                );
            }
            Ok(LlmResponse {
                content: "## Status\nfinished\n\n## Final_Answer\nok".to_string(),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: 10,
                    completion_tokens: 2,
                    total_tokens: 12,
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    #[test]
    fn session_worker_identity_sets_prompt_assistant_heading() {
        let dir = tmp_dir("worker_assistant_heading");
        let core = AgentCore::new(
            "STATIC",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        let worker = CoreSessionWorker::spawn_with_model_client(
            core,
            test_config(),
            test_worker_config(&dir, "session_worker_heading", 4),
            AssistantHeadingModel {
                expected_heading: "## ID4".to_string(),
                calls: 0,
            },
        );

        worker
            .handle()
            .run_turn("hello", None)
            .expect("worker should accept run_turn");
        let first = wait_for_turn_finished(worker.events(), "heading first", false);
        assert_eq!(first.text, "ok");
        worker
            .handle()
            .run_turn("continue", None)
            .expect("worker should accept second run_turn");
        let second = wait_for_turn_finished(worker.events(), "heading second", false);
        assert_eq!(second.text, "ok");
        worker
            .handle()
            .request_shutdown()
            .expect("worker should accept shutdown");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_worker_shutdown_cancels_pending_host_decision() {
        let dir = tmp_dir("decision_shutdown");
        let mut core = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        core.set_bash_approval_mode(BashApprovalMode::Ask);
        let calls = Arc::new(Mutex::new(0));
        let worker = CoreSessionWorker::spawn_with_model_client(
            core,
            test_config(),
            test_worker_config(&dir, "session_worker_decision_shutdown", 2),
            ApprovalReplayModel {
                calls: Arc::clone(&calls),
            },
        );
        let handle = worker.handle();
        handle
            .run_turn("请执行需要确认的本地命令。", None)
            .expect("worker should accept run_turn");

        loop {
            match worker
                .events()
                .recv_timeout(Duration::from_secs(2))
                .expect("worker should request approval")
            {
                CoreSessionWorkerEvent::Topics(events) => {
                    if events.iter().any(|event| {
                        event
                            .as_host_decision_request()
                            .map(|topic| topic.request.kind() == "user_approval")
                            .unwrap_or(false)
                    }) {
                        break;
                    }
                }
                CoreSessionWorkerEvent::ModelRequest { .. }
                | CoreSessionWorkerEvent::ModelResponse { .. } => {}
                other => panic!("unexpected event while waiting for approval: {other:?}"),
            }
        }

        let shutdown_start = Instant::now();
        worker.shutdown().unwrap();
        assert!(
            shutdown_start.elapsed() < Duration::from_secs(2),
            "shutdown should cancel pending host decision promptly"
        );
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    struct CancellableCountingModel {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl ModelClient for CancellableCountingModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &std::path::Path,
            should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            let input_marker = if prompt.contains("second queued turn") {
                "second"
            } else {
                "first"
            };
            self.calls.lock().unwrap().push(input_marker.to_string());
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(500) {
                if should_cancel() {
                    return Err("cancelled_by_user".to_string());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(LlmResponse {
                content: "## Status\nfinished\n\n## Final_Answer\nDONE".to_string(),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: 1_000,
                    completion_tokens: 10,
                    total_tokens: 1_010,
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    #[test]
    fn session_worker_shutdown_skips_queued_turns() {
        let dir = tmp_dir("shutdown_skips_queued");
        let core = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let worker = CoreSessionWorker::spawn_with_model_client(
            core,
            test_config(),
            test_worker_config(&dir, "session_worker_shutdown_queue", 4),
            CancellableCountingModel {
                calls: Arc::clone(&calls),
            },
        );
        let handle = worker.handle();
        let _ = worker
            .events()
            .recv_timeout(Duration::from_secs(2))
            .expect("worker should emit lifecycle topic");

        handle
            .run_turn("first active turn", None)
            .expect("worker should accept first turn");
        loop {
            match worker
                .events()
                .recv_timeout(Duration::from_secs(2))
                .expect("worker should emit first model request")
            {
                CoreSessionWorkerEvent::ModelRequest { .. } => break,
                CoreSessionWorkerEvent::Topics(_) => {}
                other => panic!("unexpected event before first model request: {other:?}"),
            }
        }

        handle
            .run_turn("second queued turn", None)
            .expect("worker should accept queued second turn before shutdown");
        handle
            .request_shutdown()
            .expect("worker should accept shutdown");

        loop {
            match worker
                .events()
                .recv_timeout(Duration::from_secs(2))
                .expect("worker should stop after shutdown")
            {
                CoreSessionWorkerEvent::WorkerStopped => break,
                CoreSessionWorkerEvent::TurnFinished { .. }
                | CoreSessionWorkerEvent::ModelError { .. }
                | CoreSessionWorkerEvent::Topics(_)
                | CoreSessionWorkerEvent::ModelResponseDiscarded { .. } => {}
                other => panic!("unexpected event while shutting down worker: {other:?}"),
            }
        }
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["first".to_string()],
            "shutdown must not process queued turns after the active turn is cancelled"
        );
        assert_eq!(
            handle.run_turn("third after shutdown", None),
            Err("core_session_worker_stopped".to_string())
        );
        worker.shutdown().unwrap();
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ConcurrentModelCall {
        worker: String,
        has_supplement: bool,
    }

    struct ConcurrentWorkerModel {
        worker: &'static str,
        first_call_barrier: Arc<Barrier>,
        calls: Arc<Mutex<Vec<ConcurrentModelCall>>>,
    }

    impl ModelClient for ConcurrentWorkerModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &std::path::Path,
            should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            let has_supplement = prompt.contains("## USER") && prompt.contains("SUPPLEMENT");
            self.calls.lock().unwrap().push(ConcurrentModelCall {
                worker: self.worker.to_string(),
                has_supplement,
            });
            if !has_supplement {
                self.first_call_barrier.wait();
            }
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(150) {
                if should_cancel() {
                    return Err("cancelled_by_user".to_string());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let answer = match (self.worker, has_supplement) {
                ("ai1", true) => "AI1_SUPPLEMENT_OK",
                ("ai1", false) => "AI1_STALE",
                ("ai2", _) => "AI2_OK",
                _ => "UNKNOWN_WORKER",
            };
            Ok(LlmResponse {
                content: format!("## Status\nfinished\n\n## Final_Answer\n{answer}"),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: if has_supplement { 1_300 } else { 1_000 },
                    completion_tokens: 10,
                    total_tokens: if has_supplement { 1_310 } else { 1_010 },
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    #[test]
    fn session_workers_run_concurrently_without_cross_talk() {
        let dir_a = tmp_dir("concurrent_a");
        let dir_b = tmp_dir("concurrent_b");
        let core_a = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir_a,
        );
        let core_b = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir_b,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let first_call_barrier = Arc::new(Barrier::new(2));
        let worker_a = CoreSessionWorker::spawn_with_model_client(
            core_a,
            test_config(),
            test_worker_config(&dir_a, "concurrent_ai1", 1),
            ConcurrentWorkerModel {
                worker: "ai1",
                first_call_barrier: Arc::clone(&first_call_barrier),
                calls: Arc::clone(&calls),
            },
        );
        let worker_b = CoreSessionWorker::spawn_with_model_client(
            core_b,
            test_config(),
            test_worker_config(&dir_b, "concurrent_ai2", 2),
            ConcurrentWorkerModel {
                worker: "ai2",
                first_call_barrier,
                calls: Arc::clone(&calls),
            },
        );
        let handle_a = worker_a.handle();
        let handle_b = worker_b.handle();

        let lifecycle_a = worker_a
            .events()
            .recv_timeout(Duration::from_secs(2))
            .expect("ai1 should emit lifecycle");
        let lifecycle_b = worker_b
            .events()
            .recv_timeout(Duration::from_secs(2))
            .expect("ai2 should emit lifecycle");
        assert_eq!(
            lifecycle_a
                .as_topics_first_lifecycle()
                .expect("ai1 lifecycle topic")
                .worker
                .unwrap()
                .display_name,
            "ID1"
        );
        assert_eq!(
            lifecycle_b
                .as_topics_first_lifecycle()
                .expect("ai2 lifecycle topic")
                .worker
                .unwrap()
                .display_name,
            "ID2"
        );

        handle_a
            .run_turn("ai1 first turn waits for supplement", None)
            .expect("ai1 should accept turn");
        handle_b
            .run_turn("ai2 first turn should finish normally", None)
            .expect("ai2 should accept turn");
        wait_for_model_request(worker_a.events(), "ai1");
        wait_for_model_request(worker_b.events(), "ai2");
        handle_a.add_user_supplement("补充：ai1 必须输出 AI1_SUPPLEMENT_OK。");

        let outcome_a = wait_for_turn_finished(worker_a.events(), "ai1", true);
        let outcome_b = wait_for_turn_finished(worker_b.events(), "ai2", false);
        assert_eq!(outcome_a.text, "AI1_SUPPLEMENT_OK");
        assert_eq!(outcome_a.stats.llm_calls, 2);
        assert_eq!(outcome_b.text, "AI2_OK");
        assert_eq!(outcome_b.stats.llm_calls, 1);

        let calls = calls.lock().unwrap().clone();
        assert!(calls.contains(&ConcurrentModelCall {
            worker: "ai1".to_string(),
            has_supplement: false,
        }));
        assert!(calls.contains(&ConcurrentModelCall {
            worker: "ai1".to_string(),
            has_supplement: true,
        }));
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.worker == "ai2" && call.has_supplement)
                .count(),
            0,
            "ai1 supplement must not leak into ai2 context"
        );

        worker_a.shutdown().unwrap();
        worker_b.shutdown().unwrap();
    }

    struct WorkerCountModel {
        first_call_barrier: Arc<Barrier>,
        call_no: u32,
    }

    impl ModelClient for WorkerCountModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            _prompt: &str,
            _audit_file: &std::path::Path,
            _should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.call_no += 1;
            let content = if self.call_no == 1 {
                self.first_call_barrier.wait();
                "## Status\nworking\n\n## Progress\n正在执行并发计数测试。\n\n## Working_Still_Action\n```action\n{\"action\":\"run_bash\",\"intent\":\"执行一个短命令。\",\"args\":{\"cmd\":\"printf worker-count-ok\",\"timeout_ms\":5000}}\n```"
                    .to_string()
            } else {
                "## Status\nfinished\n\n## Final_Answer\nWORKER_COUNT_DONE".to_string()
            };
            Ok(LlmResponse {
                content,
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: 100,
                    completion_tokens: 10,
                    total_tokens: 110,
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    fn collect_model_response_worker_counts(
        events: &Receiver<CoreSessionWorkerEvent>,
        label: &str,
    ) -> Vec<usize> {
        let mut counts = Vec::new();
        loop {
            match events
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|_| panic!("{label} timed out waiting for turn finish"))
            {
                CoreSessionWorkerEvent::Topics(events) => {
                    counts.extend(
                        events
                            .iter()
                            .filter_map(CoreTopicEvent::as_model_response)
                            .map(|topic| topic.global.working_worker_count),
                    );
                }
                CoreSessionWorkerEvent::TurnFinished { outcome } => {
                    assert_eq!(outcome.text, "WORKER_COUNT_DONE");
                    return counts;
                }
                CoreSessionWorkerEvent::ModelRequest { .. }
                | CoreSessionWorkerEvent::ModelResponse { .. } => {}
                other => panic!("{label} unexpected event while collecting counts: {other:?}"),
            }
        }
    }

    #[test]
    fn shared_worker_runtime_publishes_global_working_count_on_model_response_topics() {
        let runtime = CoreSessionWorkerRuntime::new();
        let barrier = Arc::new(Barrier::new(2));
        let dir_a = tmp_dir("worker_count_a");
        let dir_b = tmp_dir("worker_count_b");
        let mut core_a = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir_a,
        );
        let mut core_b = AgentCore::new(
            "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
            CoreProfile {
                name: "test".to_string(),
                provider: "test".to_string(),
                model: "test-model".to_string(),
            },
            &dir_b,
        );
        core_a.set_bash_approval_mode(BashApprovalMode::Approve);
        core_b.set_bash_approval_mode(BashApprovalMode::Approve);
        let worker_a = CoreSessionWorker::spawn_with_runtime_model_client(
            core_a,
            test_config(),
            test_worker_config(&dir_a, "worker_count_a", 1),
            runtime.clone(),
            WorkerCountModel {
                first_call_barrier: Arc::clone(&barrier),
                call_no: 0,
            },
        );
        let worker_b = CoreSessionWorker::spawn_with_runtime_model_client(
            core_b,
            test_config(),
            test_worker_config(&dir_b, "worker_count_b", 2),
            runtime.clone(),
            WorkerCountModel {
                first_call_barrier: barrier,
                call_no: 0,
            },
        );
        let handle_a = worker_a.handle();
        let handle_b = worker_b.handle();
        let _ = worker_a
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let _ = worker_b
            .events()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        handle_a.run_turn("worker count a", None).unwrap();
        handle_b.run_turn("worker count b", None).unwrap();

        let counts_a = collect_model_response_worker_counts(worker_a.events(), "worker_count_a");
        let counts_b = collect_model_response_worker_counts(worker_b.events(), "worker_count_b");
        let mut all_counts = counts_a;
        all_counts.extend(counts_b);

        assert!(
            all_counts.contains(&2),
            "at least one progress response should see both workers active: {all_counts:?}"
        );
        assert!(
            all_counts.contains(&0),
            "the last final response should tell UI no workers remain active: {all_counts:?}"
        );
        assert_eq!(runtime.working_worker_count(), 0);

        worker_a.shutdown().unwrap();
        worker_b.shutdown().unwrap();
    }

    trait WorkerEventTestExt {
        fn as_topics_first_lifecycle(&self) -> Option<crate::CoreLifecycleTopic>;
    }

    impl WorkerEventTestExt for CoreSessionWorkerEvent {
        fn as_topics_first_lifecycle(&self) -> Option<crate::CoreLifecycleTopic> {
            match self {
                CoreSessionWorkerEvent::Topics(events) => {
                    events.first().and_then(CoreTopicEvent::as_lifecycle)
                }
                _ => None,
            }
        }
    }

    fn wait_for_model_request(events: &Receiver<CoreSessionWorkerEvent>, label: &str) {
        loop {
            match events
                .recv_timeout(Duration::from_secs(3))
                .unwrap_or_else(|_| panic!("{label} timed out waiting for model request"))
            {
                CoreSessionWorkerEvent::ModelRequest { round } => {
                    assert_eq!(round, 1, "{label} first request should be round 1");
                    return;
                }
                CoreSessionWorkerEvent::Topics(_) => {}
                other => panic!("{label} unexpected event before model request: {other:?}"),
            }
        }
    }

    fn wait_for_turn_finished(
        events: &Receiver<CoreSessionWorkerEvent>,
        label: &str,
        expect_discard: bool,
    ) -> TurnOutcome {
        let mut saw_discard = false;
        loop {
            match events
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|_| panic!("{label} timed out waiting for turn finish"))
            {
                CoreSessionWorkerEvent::ModelResponseDiscarded { reason, .. } => {
                    assert_eq!(reason, "user_supplement_preempted_stale_response");
                    saw_discard = true;
                }
                CoreSessionWorkerEvent::TurnFinished { outcome } => {
                    assert_eq!(
                        saw_discard, expect_discard,
                        "{label} discard expectation mismatch"
                    );
                    return outcome;
                }
                CoreSessionWorkerEvent::Topics(_)
                | CoreSessionWorkerEvent::ModelRequest { .. }
                | CoreSessionWorkerEvent::ModelResponse { .. } => {}
                other => panic!("{label} unexpected event while waiting finish: {other:?}"),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct StressModelCall {
        worker_idx: usize,
        turn_idx: usize,
        call_no: u32,
        target_actions: usize,
        completed_actions: usize,
        has_own_supplement: bool,
        saw_cross_session_marker: bool,
    }

    struct StressWorkerModel {
        worker_idx: usize,
        worker_count: usize,
        call_no: u32,
        first_call_barrier: Arc<Barrier>,
        calls: Arc<Mutex<Vec<StressModelCall>>>,
    }

    fn stress_marker(worker_idx: usize, turn_idx: usize, step_idx: usize) -> String {
        format!("STRESS_ACTION_DONE_W{worker_idx}_T{turn_idx}_S{step_idx}")
    }

    fn latest_stress_turn(prompt: &str, worker_idx: usize, turns_per_worker: usize) -> usize {
        (0..turns_per_worker)
            .filter_map(|turn_idx| {
                prompt
                    .rfind(&format!("stress worker {worker_idx} turn {turn_idx}"))
                    .map(|pos| (pos, turn_idx))
            })
            .max_by_key(|(pos, _)| *pos)
            .map(|(_, turn_idx)| turn_idx)
            .unwrap_or(0)
    }

    fn stress_target_actions(turn_idx: usize, long_turn_idx: usize, max_rounds: usize) -> usize {
        if turn_idx == long_turn_idx {
            max_rounds + 10
        } else {
            8
        }
    }

    fn completed_stress_actions(
        prompt: &str,
        worker_idx: usize,
        turn_idx: usize,
        target_actions: usize,
    ) -> usize {
        (0..target_actions)
            .filter(|step_idx| prompt.contains(&stress_marker(worker_idx, turn_idx, *step_idx)))
            .count()
    }

    fn stress_progress(worker_idx: usize, turn_idx: usize, step_idx: usize) -> String {
        if step_idx == 0 {
            format!(
                "stress worker {worker_idx} turn {turn_idx} long progress {}",
                "progress_chunk_".repeat(260)
            )
        } else {
            format!("stress worker {worker_idx} turn {turn_idx} step {step_idx}")
        }
    }

    fn stress_action_response(worker_idx: usize, turn_idx: usize, step_idx: usize) -> String {
        let marker = stress_marker(worker_idx, turn_idx, step_idx);
        let (action, intent, args) = match step_idx % 8 {
            1 => (
                "run_bash",
                format!("Run a short stress shell action for {marker}."),
                serde_json::json!({
                    "cmd": format!("printf {marker}"),
                    "timeout_ms": 5000,
                }),
            ),
            2 => (
                "run_bash",
                format!("Exercise long command validation for {marker}."),
                serde_json::json!({
                    "cmd": format!("printf {marker}; # {}", "x".repeat(2_100)),
                    "timeout_ms": 5000,
                }),
            ),
            3 => (
                "self_tool",
                format!("Read runtime paths while stress testing {marker}."),
                serde_json::json!({
                    "type": "mem_path",
                    "op": "read",
                }),
            ),
            _ => (
                "memmgr",
                format!("Query durable memory marker {marker}."),
                serde_json::json!({
                    "type": "durable",
                    "op": "query",
                    "query": marker,
                    "limit": 1,
                }),
            ),
        };
        serde_json::json!({
            "action": action,
            "intent": intent,
            "args": args,
        })
        .to_string()
    }

    impl ModelClient for StressWorkerModel {
        fn call_model(
            &mut self,
            _config: &ProviderConfig,
            prompt: &str,
            _audit_file: &std::path::Path,
            should_cancel: &mut dyn FnMut() -> bool,
        ) -> Result<LlmResponse, String> {
            self.call_no += 1;
            if self.call_no == 1 {
                self.first_call_barrier.wait();
            }
            const TURNS_PER_WORKER: usize = 30;
            const TEST_MAX_ROUNDS: usize = 12;
            const LONG_TURN_IDX: usize = TURNS_PER_WORKER - 1;
            let own_marker = format!("SUPP_MARKER_{}", self.worker_idx);
            let has_own_supplement = prompt.contains(&own_marker);
            let saw_cross_session_marker = (0..self.worker_count)
                .filter(|idx| *idx != self.worker_idx)
                .any(|idx| prompt.contains(&format!("SUPP_MARKER_{idx}")));
            let turn_idx = latest_stress_turn(prompt, self.worker_idx, TURNS_PER_WORKER);
            let target_actions = stress_target_actions(turn_idx, LONG_TURN_IDX, TEST_MAX_ROUNDS);
            let completed_actions =
                completed_stress_actions(prompt, self.worker_idx, turn_idx, target_actions);
            self.calls.lock().unwrap().push(StressModelCall {
                worker_idx: self.worker_idx,
                turn_idx,
                call_no: self.call_no,
                target_actions,
                completed_actions,
                has_own_supplement,
                saw_cross_session_marker,
            });
            if should_cancel() {
                return Err("cancelled_by_user".to_string());
            }
            if turn_idx == 1 && completed_actions == 0 && !has_own_supplement {
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(120) {
                    if should_cancel() {
                        return Err("cancelled_by_user".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            if completed_actions < target_actions {
                let content = format!(
                    "## Progress\n{}\n\n## Working_Still_Action\n```action\n{}\n```",
                    stress_progress(self.worker_idx, turn_idx, completed_actions),
                    stress_action_response(self.worker_idx, turn_idx, completed_actions)
                );
                return Ok(LlmResponse {
                    content,
                    model_name: "test-model".to_string(),
                    usage: UsageStats {
                        llm_calls: 1,
                        prompt_tokens: 2_000 + completed_actions as u32 * 10,
                        completion_tokens: if completed_actions == 0 { 2_500 } else { 120 },
                        total_tokens: 2_120 + completed_actions as u32 * 10,
                        ..UsageStats::zero()
                    },
                    truncated: false,
                });
            }
            let answer = if saw_cross_session_marker {
                format!("WORKER_{}_LEAK", self.worker_idx)
            } else if has_own_supplement {
                format!("WORKER_{}_TURN_{turn_idx}_SUPPLEMENTED", self.worker_idx)
            } else {
                format!("WORKER_{}_TURN_{turn_idx}_OK", self.worker_idx)
            };
            Ok(LlmResponse {
                content: format!("## Status\nfinished\n\n## Final_Answer\n{answer}"),
                model_name: "test-model".to_string(),
                usage: UsageStats {
                    llm_calls: 1,
                    prompt_tokens: if has_own_supplement { 1_500 } else { 1_000 },
                    completion_tokens: 10,
                    total_tokens: if has_own_supplement { 1_510 } else { 1_010 },
                    ..UsageStats::zero()
                },
                truncated: false,
            })
        }
    }

    #[test]
    fn session_workers_stress_ui_threads_supplements_and_renames() {
        const WORKERS: usize = 5;
        const TURNS_PER_WORKER: usize = 30;
        const TEST_MAX_ROUNDS: usize = 12;
        const LONG_TURN_IDX: usize = TURNS_PER_WORKER - 1;
        let first_call_barrier = Arc::new(Barrier::new(WORKERS));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut host_threads = Vec::new();

        for worker_idx in 0..WORKERS {
            let dir = tmp_dir(&format!("stress_worker_{worker_idx}"));
            let mut core = AgentCore::new(
                "You are Timem.\n{{ response_protocol }}\n{{ capability_catalog }}",
                CoreProfile {
                    name: "test".to_string(),
                    provider: "test".to_string(),
                    model: "test-model".to_string(),
                },
                &dir,
            );
            core.set_bash_approval_mode(BashApprovalMode::Approve);
            core.set_max_rounds(TEST_MAX_ROUNDS as u32);
            core.set_max_llm_input_tokens(1_000_000);
            let worker = CoreSessionWorker::spawn_with_model_client(
                core,
                test_config(),
                test_worker_config(
                    &dir,
                    &format!("stress_session_{worker_idx}"),
                    worker_idx as u32 + 1,
                ),
                StressWorkerModel {
                    worker_idx,
                    worker_count: WORKERS,
                    call_no: 0,
                    first_call_barrier: Arc::clone(&first_call_barrier),
                    calls: Arc::clone(&calls),
                },
            );

            host_threads.push(thread::spawn(move || {
                let handle = worker.handle();
                let lifecycle = worker
                    .events()
                    .recv_timeout(Duration::from_secs(2))
                    .expect("stress worker should emit lifecycle");
                assert_eq!(
                    lifecycle
                        .as_topics_first_lifecycle()
                        .expect("stress lifecycle topic")
                        .worker
                        .unwrap()
                        .display_name,
                    format!("ID{}", worker_idx + 1)
                );

                handle
                    .rename(format!("Stress-{worker_idx}"))
                    .expect("stress worker should accept rename");
                let renamed = worker
                    .events()
                    .recv_timeout(Duration::from_secs(2))
                    .expect("stress worker should emit rename lifecycle");
                assert_eq!(
                    renamed
                        .as_topics_first_lifecycle()
                        .expect("stress rename lifecycle topic")
                        .worker
                        .unwrap()
                        .display_name,
                    format!("Stress-{worker_idx}")
                );

                for turn in 0..TURNS_PER_WORKER {
                    let target_actions =
                        stress_target_actions(turn, LONG_TURN_IDX, TEST_MAX_ROUNDS);
                    handle
                        .run_turn(format!("stress worker {worker_idx} turn {turn}"), None)
                        .expect("stress worker should accept turn");
                    wait_for_model_request(
                        worker.events(),
                        &format!("stress worker {worker_idx} turn {turn}"),
                    );
                    if turn == 1 {
                        handle.add_user_supplement(format!(
                            "SUPP_MARKER_{worker_idx}: use the supplemented answer."
                        ));
                    }
                    let outcome = wait_for_stress_turn_finished(
                        worker.events(),
                        &handle,
                        &format!("stress worker {worker_idx} turn {turn}"),
                        turn == 1,
                        target_actions,
                        turn == LONG_TURN_IDX,
                    );
                    assert!(
                        !outcome.text.contains("LEAK"),
                        "worker {worker_idx} observed another session's supplement"
                    );
                    if turn >= 1 {
                        assert_eq!(
                            outcome.text,
                            format!("WORKER_{worker_idx}_TURN_{turn}_SUPPLEMENTED")
                        );
                    } else {
                        assert_eq!(outcome.text, format!("WORKER_{worker_idx}_TURN_{turn}_OK"));
                    }
                    assert_eq!(
                        outcome.stats.tool_calls as usize, target_actions,
                        "stress worker {worker_idx} turn {turn} should execute every action"
                    );
                }

                handle
                    .rename(format!("Stress-{worker_idx}-done"))
                    .expect("stress worker should accept final rename");
                let final_rename = worker
                    .events()
                    .recv_timeout(Duration::from_secs(2))
                    .expect("stress worker should emit final rename lifecycle");
                assert_eq!(
                    final_rename
                        .as_topics_first_lifecycle()
                        .expect("stress final lifecycle topic")
                        .worker
                        .unwrap()
                        .display_name,
                    format!("Stress-{worker_idx}-done")
                );

                handle
                    .request_shutdown()
                    .expect("stress worker should accept shutdown");
                loop {
                    match worker
                        .events()
                        .recv_timeout(Duration::from_secs(2))
                        .expect("stress worker should stop")
                    {
                        CoreSessionWorkerEvent::WorkerStopped => break,
                        CoreSessionWorkerEvent::Topics(_) => {}
                        other => {
                            panic!("stress worker {worker_idx} unexpected stop event: {other:?}")
                        }
                    }
                }
                worker.shutdown().unwrap();
            }));
        }

        for host_thread in host_threads {
            host_thread
                .join()
                .expect("stress host driver thread should not panic");
        }

        let calls = calls.lock().unwrap().clone();
        assert_eq!(
            calls.iter().filter(|call| call.call_no == 1).count(),
            WORKERS,
            "each worker should have reached the synchronized first model call"
        );
        assert!(
            calls.iter().all(|call| !call.saw_cross_session_marker),
            "no worker prompt should include another worker's supplement marker: {calls:?}"
        );
        assert!(
            calls.iter().any(|call| {
                call.target_actions == TEST_MAX_ROUNDS + 10
                    && call.completed_actions >= TEST_MAX_ROUNDS
            }),
            "stress should cross configured max rounds before finishing"
        );
        assert!(
            calls.iter().filter(|call| call.has_own_supplement).count() >= WORKERS,
            "each worker should see its own supplement during the supplemented turn"
        );
        for worker_idx in 0..WORKERS {
            assert!(
                calls.iter().any(|call| call.worker_idx == worker_idx),
                "missing calls for stress worker {worker_idx}"
            );
            for turn_idx in 0..TURNS_PER_WORKER {
                let target_actions =
                    stress_target_actions(turn_idx, LONG_TURN_IDX, TEST_MAX_ROUNDS);
                assert!(
                    calls.iter().any(|call| {
                        call.worker_idx == worker_idx
                            && call.turn_idx == turn_idx
                            && call.completed_actions == target_actions
                    }),
                    "missing completed stress call for worker {worker_idx} turn {turn_idx}"
                );
            }
        }
    }

    fn wait_for_stress_turn_finished(
        events: &Receiver<CoreSessionWorkerEvent>,
        handle: &CoreSessionWorkerHandle,
        label: &str,
        expect_discard: bool,
        expected_tool_calls: usize,
        expect_round_limit: bool,
    ) -> TurnOutcome {
        let mut saw_discard = false;
        let mut model_requests = 1usize;
        let mut model_responses = 0usize;
        let mut long_progress_seen = false;
        let mut round_limit_requests = 0usize;
        loop {
            match events
                .recv_timeout(Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("{label} timed out waiting for turn finish"))
            {
                CoreSessionWorkerEvent::ModelRequest { .. } => {
                    model_requests += 1;
                }
                CoreSessionWorkerEvent::ModelResponse { .. } => {
                    model_responses += 1;
                }
                CoreSessionWorkerEvent::ModelResponseDiscarded { reason, .. } => {
                    assert_eq!(reason, "user_supplement_preempted_stale_response");
                    saw_discard = true;
                }
                CoreSessionWorkerEvent::Topics(events) => {
                    for event in events {
                        if let Some(response) = event.as_model_response() {
                            if response.report_job_progress.len() > 2_000 {
                                long_progress_seen = true;
                            }
                        }
                        if event.is_blocking_request() {
                            let request = event
                                .as_host_decision_request()
                                .expect("blocking topic should decode as host decision request");
                            if request.kind == "round_limit_continue" {
                                round_limit_requests += 1;
                            }
                            let reply =
                                TopicReply::for_decision_request(&event, HostDecision::Accept)
                                    .expect("blocking topic should build accept reply");
                            handle.reply_to_request(reply).unwrap();
                        }
                    }
                }
                CoreSessionWorkerEvent::TurnFinished { outcome } => {
                    assert_eq!(
                        saw_discard, expect_discard,
                        "{label} discard expectation mismatch"
                    );
                    assert!(
                        long_progress_seen,
                        "{label} should surface at least one long progress topic"
                    );
                    assert_eq!(
                        outcome.stats.tool_calls as usize, expected_tool_calls,
                        "{label} tool call count mismatch"
                    );
                    assert!(
                        model_requests >= expected_tool_calls + 1,
                        "{label} should request the model for each action plus final answer"
                    );
                    assert!(
                        model_responses >= expected_tool_calls + 1,
                        "{label} should receive the model for each action plus final answer"
                    );
                    if expect_round_limit {
                        assert!(
                            round_limit_requests >= 1,
                            "{label} should request round-limit continuation"
                        );
                    } else {
                        assert_eq!(
                            round_limit_requests, 0,
                            "{label} should not hit round-limit continuation"
                        );
                    }
                    return outcome;
                }
                other => panic!("{label} unexpected worker event: {other:?}"),
            }
        }
    }
}

use crate::response_protocol::ParsedAction;
use crate::{
    ActionExecution, ActionOutcome, ActionRuntime, ActionStatus, AgentCore, ApprovalRequest,
    BashApprovalMode, BashResultEvidence, LongRunningCommandStatus, PendingApproval,
    PendingApprovedAction, StreamCaptureTruncation,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(all(test, unix))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(all(test, unix))]
static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
const LONG_RUNNING_COMMAND_PROMPT_AFTER: Duration = Duration::from_secs(60);
/// Default total polling wait budget (loop_timeout_ms) when loop_cmd is present.
const DEFAULT_LOOP_TIMEOUT_MS: i64 = 60_000;

pub(crate) fn is_local_shell_action(action: &str) -> bool {
    matches!(action, "run_bash" | "run_powershell")
}

fn configure_run_bash_environment(command: &mut Command) {
    command
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("TERM", "dumb");
}

const SHELL_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;
const LONG_RUNNING_ACTION_GUIDANCE: &str = "The command has not finished. Decide whether to wait, inspect, terminate, or take another appropriate action. Reflect and avoid long running ineffective actions if possible. Save time.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningShellJob {
    pub pid: u32,
    pub tool_call_id: String,
    pub kind: String,
    pub command: String,
    pub cwd: String,
    pub session_id: String,
    pub turn_id: String,
    pub created_at_ms: i64,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellJobExitUpdate {
    pub pid: u32,
    /// Whether a finish topic for this job was already published through the
    /// manager's exit listener, so later consumption must not duplicate it.
    pub topic_published: bool,
    pub tool_call_id: String,
    pub kind: String,
    pub command: String,
    pub cwd: String,
    pub session_id: String,
    pub turn_id: String,
    pub created_at_ms: i64,
    pub elapsed_ms: i64,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncation: Option<StreamCaptureTruncation>,
    pub stderr_truncation: Option<StreamCaptureTruncation>,
    pub output: String,
}

impl ShellJobExitUpdate {
    pub fn description(&self) -> &'static str {
        match self.kind.as_str() {
            "timeout" => "old timeout job",
            _ => "background job",
        }
    }
}

impl RunningShellJob {
    pub fn elapsed_ms(&self) -> i64 {
        now_ms().saturating_sub(self.created_at_ms).max(0)
    }
    pub fn description(&self) -> &'static str {
        match self.kind.as_str() {
            "timeout" => "old job timeout",
            _ => "background job",
        }
    }
}

#[derive(Debug)]
struct BoundedShellOutput {
    bytes: std::collections::VecDeque<u8>,
    retain_tail: bool,
    truncated: bool,
    original_bytes: usize,
}

impl BoundedShellOutput {
    fn new(retain_tail: bool) -> Self {
        Self {
            bytes: std::collections::VecDeque::with_capacity(SHELL_OUTPUT_LIMIT_BYTES),
            retain_tail,
            truncated: false,
            original_bytes: 0,
        }
    }
    fn push(&mut self, chunk: &[u8]) {
        self.original_bytes = self.original_bytes.saturating_add(chunk.len());
        if self.retain_tail {
            if chunk.len() >= SHELL_OUTPUT_LIMIT_BYTES {
                self.bytes.clear();
                self.bytes.extend(
                    chunk[chunk.len() - SHELL_OUTPUT_LIMIT_BYTES..]
                        .iter()
                        .copied(),
                );
                self.truncated = true;
                return;
            }
            let overflow = self
                .bytes
                .len()
                .saturating_add(chunk.len())
                .saturating_sub(SHELL_OUTPUT_LIMIT_BYTES);
            if overflow > 0 {
                self.bytes.drain(..overflow);
                self.truncated = true;
            }
            self.bytes.extend(chunk.iter().copied());
        } else {
            let remaining = SHELL_OUTPUT_LIMIT_BYTES.saturating_sub(self.bytes.len());
            self.bytes.extend(chunk.iter().take(remaining).copied());
            self.truncated = self.truncated || chunk.len() > remaining;
        }
    }
    fn snapshot(&self) -> ShellOutputSnapshot {
        let bytes = self.bytes.iter().copied().collect::<Vec<_>>();
        ShellOutputSnapshot {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            truncation: self.truncated.then_some(StreamCaptureTruncation {
                original_bytes: self.original_bytes,
                retained_bytes: bytes.len(),
                retained: if self.retain_tail { "tail" } else { "head" },
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct ShellOutputSnapshot {
    text: String,
    truncation: Option<StreamCaptureTruncation>,
}

type SharedShellOutput = Arc<Mutex<BoundedShellOutput>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellJobDelivery {
    Direct,
    Background,
    Delivered,
}

#[derive(Debug, Clone)]
struct FinishedShellJob {
    completion_sequence: u64,
    /// Wall-clock timestamp (ms) captured by the supervisor thread the moment
    /// the child actually exited, so later delivery does not inflate elapsed.
    finished_at_ms: i64,
    status: String,
    stdout: String,
    stderr: String,
    stdout_truncation: Option<StreamCaptureTruncation>,
    stderr_truncation: Option<StreamCaptureTruncation>,
    output: String,
}

#[derive(Debug)]
enum ShellJobLifecycle {
    Running,
    Finished(FinishedShellJob),
}

#[derive(Debug)]
struct ShellJobState {
    delivery: ShellJobDelivery,
    lifecycle: ShellJobLifecycle,
    /// Whether a terminal update for this job has already been published
    /// through the manager's exit listener.
    exit_published: bool,
}

#[derive(Debug)]
struct ManagedShellJob {
    pid: u32,
    tool_call_id: String,
    kind: String,
    command: String,
    cwd: String,
    session_id: String,
    turn_id: String,
    created_at_ms: i64,
    stdout: SharedShellOutput,
    stderr: SharedShellOutput,
    state: Mutex<ShellJobState>,
    changed: Condvar,
    supervisor: Mutex<Option<thread::JoinHandle<()>>>,
    completion_publication: Arc<Mutex<u64>>,
    exit_hooks: Arc<Mutex<ShellJobManagerExitHooks>>,
    process_job: Option<crate::os::ManagedProcessJob>,
}

impl ManagedShellJob {
    fn running(&self) -> RunningShellJob {
        RunningShellJob {
            pid: self.pid,
            tool_call_id: self.tool_call_id.clone(),
            kind: self.kind.clone(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            created_at_ms: self.created_at_ms,
            notes: String::new(),
        }
    }

    fn partial_streams(&self) -> (ShellOutputSnapshot, ShellOutputSnapshot) {
        (
            shell_output_snapshot(&self.stdout),
            shell_output_snapshot(&self.stderr),
        )
    }

    fn exit_update(&self, finished: &FinishedShellJob) -> ShellJobExitUpdate {
        ShellJobExitUpdate {
            pid: self.pid,
            topic_published: false,
            tool_call_id: self.tool_call_id.clone(),
            kind: self.kind.clone(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            created_at_ms: self.created_at_ms,
            elapsed_ms: finished
                .finished_at_ms
                .saturating_sub(self.created_at_ms)
                .max(0),
            status: finished.status.clone(),
            stdout: finished.stdout.clone(),
            stderr: finished.stderr.clone(),
            stdout_truncation: finished.stdout_truncation.clone(),
            stderr_truncation: finished.stderr_truncation.clone(),
            output: finished.output.clone(),
        }
    }

    fn signal(&self) {
        if self
            .process_job
            .as_ref()
            .is_none_or(|process_job| process_job.kill_all().is_err())
        {
            // Explicit degraded mode, or best-effort fallback if the native Job backend
            // control file becomes unavailable after spawn.
            crate::os::terminate_process_group(self.pid);
        }
    }

    fn join_supervisor(&self) {
        let supervisor = self
            .supervisor
            .lock()
            .ok()
            .and_then(|mut supervisor| supervisor.take());
        if let Some(supervisor) = supervisor {
            let _ = supervisor.join();
        }
    }
}

type ShellJobExitListener = Arc<dyn Fn(&ShellJobExitUpdate) + Send + Sync>;

#[derive(Default)]
pub(crate) struct ShellJobManagerExitHooks {
    /// Optional event-driven callback invoked once per job, immediately when
    /// the job's supervisor observes the exit.
    exit_listener: Option<ShellJobExitListener>,
}

struct ShellJobManagerState {
    jobs: Mutex<HashMap<u32, Arc<ManagedShellJob>>>,
    completion_publication: Arc<Mutex<u64>>,
    exit_hooks: Arc<Mutex<ShellJobManagerExitHooks>>,
}

impl std::fmt::Debug for ShellJobManagerExitHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellJobManagerExitHooks")
            .field("exit_listener", &self.exit_listener.is_some())
            .finish()
    }
}

impl std::fmt::Debug for ShellJobManagerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellJobManagerState")
            .field("jobs", &self.jobs)
            .field("completion_publication", &self.completion_publication)
            .field("exit_hooks", &self.exit_hooks)
            .finish()
    }
}

impl Drop for ShellJobManagerState {
    fn drop(&mut self) {
        let jobs = self
            .jobs
            .get_mut()
            .map(|jobs| jobs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for job in &jobs {
            if job_is_running(job) {
                job.signal();
            }
        }
        for job in jobs {
            job.join_supervisor();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellJobManager {
    state: Arc<ShellJobManagerState>,
    long_running_prompt_after: Duration,
    force_process_group_fallback: bool,
}

impl ShellJobManager {
    pub fn new(memory_dir: &Path) -> Self {
        cleanup_legacy_shell_job_artifacts(memory_dir);
        Self {
            state: Arc::new(ShellJobManagerState {
                jobs: Mutex::new(HashMap::new()),
                completion_publication: Arc::new(Mutex::new(0)),
                exit_hooks: Arc::new(Mutex::new(ShellJobManagerExitHooks::default())),
            }),
            long_running_prompt_after: LONG_RUNNING_COMMAND_PROMPT_AFTER,
            force_process_group_fallback: false,
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn set_long_running_prompt_after_for_tests(&mut self, duration: Duration) {
        self.long_running_prompt_after = duration.max(Duration::from_millis(1));
    }

    #[cfg(all(test, unix))]
    pub(crate) fn force_process_group_fallback_for_tests(&mut self) {
        self.force_process_group_fallback = true;
    }

    pub fn spawn_background(
        &self,
        command: &str,
        cwd: &Path,
        session_id: &str,
        turn_id: &str,
    ) -> String {
        self.spawn_background_outcome(
            command,
            cwd,
            session_id,
            turn_id,
            "unknown_tool_call",
            false,
        )
        .text
    }

    pub(crate) fn spawn_background_outcome(
        &self,
        command: &str,
        cwd: &Path,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        tail_out: bool,
    ) -> ActionOutcome {
        let clean = command.trim();
        if clean.is_empty() {
            let reason =
                "The background command was not started because no shell command was provided.";
            return bash_finished_error_outcome(
                bash_action_not_executed(None, reason),
                "InvalidInput",
                reason,
            );
        }
        let job = match self.spawn_managed(
            clean,
            cwd,
            "background",
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
            ShellJobDelivery::Background,
        ) {
            Ok(job) => job,
            Err(_) => {
                let reason = "The background command could not be started by the local shell.";
                return bash_finished_error_outcome(
                    bash_action_not_executed(Some(clean), reason),
                    "SpawnFailed",
                    reason,
                );
            }
        };
        ActionOutcome::background_running(format!(
            "Action result: {}\npid={}, now keeps running in background",
            crate::os::local_shell_tool_name(),
            job.pid
        ))
        .with_bash_result(BashResultEvidence {
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncation: None,
            stderr_truncation: None,
            exit_code: None,
            signal: None,
            pid: Some(job.pid),
            timed_out: false,
            pid_kind: Some(runtime_child_pid_kind().to_string()),
            error_type: None,
        })
    }

    fn select_process_job(
        result: std::io::Result<crate::os::ManagedProcessJob>,
    ) -> Option<crate::os::ManagedProcessJob> {
        result.ok()
    }

    fn configured_shell_command(clean: &str, cwd: &Path) -> std::io::Result<Command> {
        let mut command =
            crate::os::command_for_local_shell(clean).map_err(std::io::Error::other)?;
        configure_run_bash_environment(&mut command);
        command
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::os::configure_child_process_group(&mut command);
        Ok(command)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_managed(
        &self,
        clean: &str,
        cwd: &Path,
        kind: &str,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        tail_out: bool,
        delivery: ShellJobDelivery,
    ) -> std::io::Result<Arc<ManagedShellJob>> {
        let mut command = Self::configured_shell_command(clean, cwd)?;
        let mut process_job = if self.force_process_group_fallback {
            None
        } else {
            Self::select_process_job(crate::os::ManagedProcessJob::create_for_session(session_id))
        };
        if process_job
            .as_ref()
            .is_some_and(|job| job.configure_command(&mut command).is_err())
        {
            process_job = None;
            command = Self::configured_shell_command(clean, cwd)?;
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) if process_job.is_some() => {
                // The cgroup may become unavailable between setup and pre_exec.
                // No user code ran when pre_exec failed, so retry once without
                // exact containment instead of making the optional mechanism
                // block command execution.
                process_job = None;
                Self::configured_shell_command(clean, cwd)?.spawn()?
            }
            Err(error) => return Err(error),
        };
        let pid = child.id();
        let child_registration = crate::os::register_managed_child(pid);
        // OS-level containment: on Windows the child joins the runtime's
        // kill-on-close job object; on Unix this is a no-op and the subreaper
        // safety net covers escapees instead.
        if !crate::os::contain_child_process(pid) {
            terminate_process(pid);
            let _ = child.wait();
            return Err(std::io::Error::other(
                "spawned process could not be contained by the runtime job",
            ));
        }
        if !is_runtime_child_pid(pid) {
            terminate_process(pid);
            let _ = child.wait();
            return Err(std::io::Error::other(
                "spawned process did not satisfy managed-child invariant",
            ));
        }
        let stdout = Arc::new(Mutex::new(BoundedShellOutput::new(tail_out)));
        let stderr = Arc::new(Mutex::new(BoundedShellOutput::new(tail_out)));
        let stdout_drain = child
            .stdout
            .take()
            .map(|pipe| spawn_output_drain(pipe, Arc::clone(&stdout)));
        let stderr_drain = child
            .stderr
            .take()
            .map(|pipe| spawn_output_drain(pipe, Arc::clone(&stderr)));
        let job = Arc::new(ManagedShellJob {
            pid,
            tool_call_id: tool_call_id.trim().to_string(),
            kind: kind.to_string(),
            command: clean.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            session_id: session_id.trim().to_string(),
            turn_id: turn_id.trim().to_string(),
            created_at_ms: now_ms(),
            stdout,
            stderr,
            state: Mutex::new(ShellJobState {
                delivery,
                lifecycle: ShellJobLifecycle::Running,
                exit_published: false,
            }),
            changed: Condvar::new(),
            supervisor: Mutex::new(None),
            completion_publication: Arc::clone(&self.state.completion_publication),
            exit_hooks: Arc::clone(&self.state.exit_hooks),
            process_job,
        });
        let supervised = Arc::clone(&job);
        let supervisor = thread::spawn(move || {
            let _child_registration = child_registration;
            supervise_shell_job(supervised, child, stdout_drain, stderr_drain)
        });
        *job.supervisor
            .lock()
            .map_err(|_| std::io::Error::other("shell job supervisor poisoned"))? =
            Some(supervisor);
        self.state
            .jobs
            .lock()
            .map_err(|_| std::io::Error::other("shell job manager poisoned"))?
            .insert(pid, Arc::clone(&job));
        Ok(job)
    }

    pub fn run_with_timeout(
        &self,
        command: &str,
        cwd: &Path,
        timeout_ms: i64,
        session_id: &str,
        turn_id: &str,
        runtime: &mut dyn ActionRuntime,
    ) -> String {
        self.run_with_timeout_outcome(
            command,
            cwd,
            timeout_ms,
            session_id,
            turn_id,
            "unknown_tool_call",
            false,
            runtime,
        )
        .text
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_with_timeout_outcome(
        &self,
        command: &str,
        cwd: &Path,
        timeout_ms: i64,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        tail_out: bool,
        runtime: &mut dyn ActionRuntime,
    ) -> ActionOutcome {
        self.run_with_timeout_structured(
            command,
            cwd,
            timeout_ms,
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
            runtime,
        )
        .to_action_outcome(crate::os::local_shell_tool_name())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_with_timeout_structured(
        &self,
        command: &str,
        cwd: &Path,
        timeout_ms: i64,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        tail_out: bool,
        runtime: &mut dyn ActionRuntime,
    ) -> BashCommandOutput {
        let clean = command.trim();
        if timeout_ms <= 0 {
            return bash_error(clean, "invalid_timeout");
        }
        let job = match self.spawn_managed(
            clean,
            cwd,
            "timeout",
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
            ShellJobDelivery::Direct,
        ) {
            Ok(job) => job,
            Err(_) => return bash_error(clean, "command_failed"),
        };
        let started = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);
        loop {
            if runtime.should_cancel() {
                match cancel_or_take_direct_result(&job) {
                    CancelJobDecision::Finished(finished) => {
                        self.remove_job(job.pid);
                        job.join_supervisor();
                        return finished_output(clean, tail_out, &finished);
                    }
                    CancelJobDecision::Cancel => {
                        job.signal();
                        job.join_supervisor();
                        self.remove_job(job.pid);
                        return bash_error(clean, "cancelled");
                    }
                }
            }
            let elapsed = started.elapsed();
            let force_handoff = runtime.should_force_handoff() && elapsed < timeout;
            let handoff = if force_handoff
                || (elapsed >= self.long_running_prompt_after && elapsed < timeout)
            {
                Some((
                    true,
                    format!(
                        "long_running_still_running:{}:{}",
                        job.pid,
                        elapsed.as_millis()
                    ),
                ))
            } else if elapsed >= timeout {
                Some((false, format!("timeout_still_running:{}", job.pid)))
            } else {
                None
            };
            if let Some((long_running, error)) = handoff {
                match promote_or_take_direct_result(&job) {
                    DirectJobDecision::Finished(finished) => {
                        self.remove_job(job.pid);
                        job.join_supervisor();
                        return finished_output(clean, tail_out, &finished);
                    }
                    DirectJobDecision::Promoted => {
                        if long_running {
                            runtime.on_long_running_command(&LongRunningCommandStatus {
                                action: crate::os::local_shell_tool_name().to_string(),
                                command: clean.to_string(),
                                pid: job.pid,
                                elapsed,
                                timeout_ms: Some(timeout_ms),
                            });
                        }
                        return running_output_for_job(&job, clean, tail_out, error);
                    }
                }
            }
            if let Some(finished) = take_direct_result(&job) {
                self.remove_job(job.pid);
                job.join_supervisor();
                return finished_output(clean, tail_out, &finished);
            }
            wait_for_job_change(&job, Duration::from_millis(20));
        }
    }

    fn remove_job(&self, pid: u32) {
        if let Ok(mut jobs) = self.state.jobs.lock() {
            jobs.remove(&pid);
        }
    }

    fn selected_jobs(
        &self,
        predicate: impl Fn(&ManagedShellJob) -> bool,
    ) -> Vec<Arc<ManagedShellJob>> {
        let jobs = self
            .state
            .jobs
            .lock()
            .ok()
            .map(|jobs| jobs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        jobs.into_iter()
            .filter(|job| predicate(job) && job_is_cancellable(job))
            .collect()
    }

    pub fn terminate_owned_running(&self) -> usize {
        let jobs = self.selected_jobs(|_| true);
        for job in &jobs {
            job.signal();
        }
        jobs.len()
    }

    pub fn cancel_unfinished_for_session(&self, session_id: &str) -> Vec<String> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Vec::new();
        }
        let jobs = self.selected_jobs(|job| job.session_id == session_id);
        for job in &jobs {
            job.signal();
        }
        jobs.into_iter().map(|job| job.pid.to_string()).collect()
    }

    /// Read-only snapshot for exactly one Session; never consumes terminal updates.
    pub fn query_running_for_session(&self, session_id: &str) -> Vec<RunningShellJob> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Vec::new();
        }
        let jobs = self
            .state
            .jobs
            .lock()
            .ok()
            .map(|jobs| jobs.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut running = jobs
            .into_iter()
            .filter_map(|job| {
                let state = job.state.lock().ok()?;
                (job.session_id == session_id
                    && state.delivery != ShellJobDelivery::Delivered
                    && matches!(state.lifecycle, ShellJobLifecycle::Running))
                .then(|| job.running())
            })
            .collect::<Vec<_>>();
        running.sort_by_key(|job| (job.created_at_ms, job.pid));
        running
    }

    /// Atomically observes one Session and consumes each new terminal update once.
    /// Use `query_running_for_session` for a non-consuming query.
    pub fn consume_completed_for_session(
        &self,
        session_id: &str,
    ) -> (Vec<RunningShellJob>, Vec<ShellJobExitUpdate>) {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let Ok(_publication) = self.state.completion_publication.lock() else {
            return (Vec::new(), Vec::new());
        };
        let jobs = self
            .state
            .jobs
            .lock()
            .ok()
            .map(|jobs| {
                jobs.values()
                    .filter(|job| job.session_id == session_id)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut running = Vec::new();
        let mut exited = Vec::new();
        let mut remove = Vec::new();
        for job in jobs {
            let Ok(mut state) = job.state.lock() else {
                continue;
            };
            match (&state.delivery, &state.lifecycle) {
                (
                    ShellJobDelivery::Direct | ShellJobDelivery::Background,
                    ShellJobLifecycle::Running,
                ) => {
                    running.push(job.running());
                }
                (ShellJobDelivery::Background, ShellJobLifecycle::Finished(finished)) => {
                    let completion_sequence = finished.completion_sequence;
                    let topic_published = state.exit_published;
                    let mut update = job.exit_update(finished);
                    update.topic_published = topic_published;
                    state.delivery = ShellJobDelivery::Delivered;
                    exited.push((completion_sequence, update));
                    remove.push(Arc::clone(&job));
                }
                _ => {}
            }
        }
        for job in remove {
            self.remove_job(job.pid);
            job.join_supervisor();
        }
        running.sort_by_key(|job| (job.created_at_ms, job.pid));
        exited.sort_by_key(|(completion_sequence, _)| *completion_sequence);
        (
            running,
            exited.into_iter().map(|(_, update)| update).collect(),
        )
    }

    pub fn running_job_list_context(&self, session_id: &str) -> Option<String> {
        let jobs = self.query_running_for_session(session_id);
        if jobs.is_empty() {
            return None;
        }
        let mut out = String::from("RUNNING JOB LIST:");
        for job in jobs {
            out.push_str(&format!(
                "\npid={}, {}, cwd={}, cmd={}, elapsed_ms={}, still running",
                job.pid,
                job.description(),
                job.cwd,
                compact_text(&job.command, 500),
                job.elapsed_ms()
            ));
        }
        out.push('\n');
        out.push_str(LONG_RUNNING_ACTION_GUIDANCE);
        Some(out)
    }

    /// Registers a callback invoked exactly once per managed job, immediately
    /// when its supervisor observes the exit, from the supervisor thread.
    /// The callback must be cheap and must not touch job state back.
    pub fn set_exit_listener(
        &self,
        listener: impl Fn(&ShellJobExitUpdate) + Send + Sync + 'static,
    ) {
        if let Ok(mut hooks) = self.state.exit_hooks.lock() {
            hooks.exit_listener = Some(Arc::new(listener));
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn tracked_job_count_for_tests(&self) -> usize {
        self.state
            .jobs
            .lock()
            .map(|jobs| jobs.len())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
enum DirectJobDecision {
    Finished(FinishedShellJob),
    Promoted,
}

#[derive(Debug)]
enum CancelJobDecision {
    Finished(FinishedShellJob),
    Cancel,
}

fn cancel_or_take_direct_result(job: &ManagedShellJob) -> CancelJobDecision {
    let mut state = job.state.lock().expect("shell job state poisoned");
    match &state.lifecycle {
        ShellJobLifecycle::Finished(finished) => CancelJobDecision::Finished(finished.clone()),
        ShellJobLifecycle::Running => {
            state.delivery = ShellJobDelivery::Delivered;
            CancelJobDecision::Cancel
        }
    }
}

fn promote_or_take_direct_result(job: &ManagedShellJob) -> DirectJobDecision {
    let mut state = job.state.lock().expect("shell job state poisoned");
    match &state.lifecycle {
        ShellJobLifecycle::Finished(finished) => DirectJobDecision::Finished(finished.clone()),
        ShellJobLifecycle::Running => {
            state.delivery = ShellJobDelivery::Background;
            DirectJobDecision::Promoted
        }
    }
}

fn take_direct_result(job: &ManagedShellJob) -> Option<FinishedShellJob> {
    let state = job.state.lock().ok()?;
    if state.delivery != ShellJobDelivery::Direct {
        return None;
    }
    match &state.lifecycle {
        ShellJobLifecycle::Finished(finished) => Some(finished.clone()),
        ShellJobLifecycle::Running => None,
    }
}

fn wait_for_job_change(job: &ManagedShellJob, duration: Duration) {
    if let Ok(state) = job.state.lock() {
        if matches!(state.lifecycle, ShellJobLifecycle::Running) {
            let _ = job.changed.wait_timeout(state, duration);
        }
    }
}

fn job_is_running(job: &ManagedShellJob) -> bool {
    job.state
        .lock()
        .map(|state| matches!(state.lifecycle, ShellJobLifecycle::Running))
        .unwrap_or(false)
}

fn job_is_cancellable(job: &ManagedShellJob) -> bool {
    job.state
        .lock()
        .map(|state| {
            state.delivery != ShellJobDelivery::Delivered
                && matches!(state.lifecycle, ShellJobLifecycle::Running)
        })
        .unwrap_or(false)
}

fn running_output_for_job(
    job: &ManagedShellJob,
    command: &str,
    tail_out: bool,
    error: String,
) -> BashCommandOutput {
    let (stdout, stderr) = job.partial_streams();
    BashCommandOutput {
        command: command.to_string(),
        status: None,
        signal: None,
        output: combined_shell_output(&stdout.text, &stderr.text),
        stdout: stdout.text,
        stderr: stderr.text,
        stdout_truncation: stdout.truncation,
        stderr_truncation: stderr.truncation,
        error: Some(error),
        job_management: None,
        tail_out,
    }
}

fn finished_output(
    command: &str,
    tail_out: bool,
    finished: &FinishedShellJob,
) -> BashCommandOutput {
    let (status, signal) = parse_exit_status_text(&finished.status);
    BashCommandOutput {
        command: command.to_string(),
        status,
        signal,
        output: finished.output.clone(),
        stdout: finished.stdout.clone(),
        stderr: finished.stderr.clone(),
        stdout_truncation: finished.stdout_truncation.clone(),
        stderr_truncation: finished.stderr_truncation.clone(),
        error: None,
        job_management: None,
        tail_out,
    }
}

fn cleanup_legacy_shell_job_artifacts(memory_dir: &Path) {
    for dir in [
        memory_dir.join("shell_jobs"),
        memory_dir.join("memory").join("shell_jobs"),
    ] {
        let Ok(metadata) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if metadata.file_type().is_symlink() || metadata.is_file() {
            let _ = std::fs::remove_file(&dir);
        } else if metadata.is_dir() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

fn spawn_output_drain<R: std::io::Read + Send + 'static>(
    mut reader: R,
    output: SharedShellOutput,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        loop {
            match std::io::Read::read(&mut reader, &mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if let Ok(mut output) = output.lock() {
                        output.push(&chunk[..read]);
                    } else {
                        break;
                    }
                }
            }
        }
    })
}

fn join_output_drains(
    stdout: Option<thread::JoinHandle<()>>,
    stderr: Option<thread::JoinHandle<()>>,
) {
    if let Some(stdout) = stdout {
        let _ = stdout.join();
    }
    if let Some(stderr) = stderr {
        let _ = stderr.join();
    }
}

fn shell_output_snapshot(output: &SharedShellOutput) -> ShellOutputSnapshot {
    output
        .lock()
        .map(|output| output.snapshot())
        .unwrap_or(ShellOutputSnapshot {
            text: String::new(),
            truncation: None,
        })
}

fn supervise_shell_job(
    job: Arc<ManagedShellJob>,
    mut child: Child,
    stdout_drain: Option<thread::JoinHandle<()>>,
    stderr_drain: Option<thread::JoinHandle<()>>,
) {
    let status = match child.wait() {
        Ok(status) => {
            if exit_signal(&status).is_some() {
                job.signal();
            }
            exit_status_text(&status)
        }
        Err(_) => {
            job.signal();
            "unknown".to_string()
        }
    };
    if let Some(process_job) = &job.process_job {
        loop {
            match process_job.is_empty() {
                Ok(true) => break,
                Ok(false) => thread::sleep(Duration::from_millis(20)),
                Err(_) => {
                    job.signal();
                    break;
                }
            }
        }
    } else {
        while crate::os::process_group_running(job.pid) {
            thread::sleep(Duration::from_millis(20));
        }
    }
    // Drain threads run concurrently while descendants are alive, preventing
    // pipe backpressure. Join only after kernel ownership reports no members.
    join_output_drains(stdout_drain, stderr_drain);
    // A terminal update is published only after the kernel reports this job's
    // native Job empty. In degraded mode, only process-group membership is known.
    let stdout = shell_output_snapshot(&job.stdout);
    let stderr = shell_output_snapshot(&job.stderr);
    let Ok(mut publication_sequence) = job.completion_publication.lock() else {
        return;
    };
    *publication_sequence = publication_sequence.saturating_add(1);
    let finished_at_ms = now_ms();
    let finished = FinishedShellJob {
        completion_sequence: *publication_sequence,
        finished_at_ms,
        status,
        output: normalized_shell_output(&combined_shell_output(&stdout.text, &stderr.text)),
        stdout: stdout.text,
        stderr: stderr.text,
        stdout_truncation: stdout.truncation,
        stderr_truncation: stderr.truncation,
    };
    let listener = job
        .exit_hooks
        .lock()
        .ok()
        .and_then(|hooks| hooks.exit_listener.clone());
    let mut exit_update = None;
    if let Ok(mut state) = job.state.lock() {
        state.lifecycle = ShellJobLifecycle::Finished(finished);
        let publish_topic = matches!(state.delivery, ShellJobDelivery::Background)
            && listener.is_some()
            && !state.exit_published;
        if publish_topic {
            state.exit_published = true;
            match &state.lifecycle {
                ShellJobLifecycle::Finished(finished) => {
                    let mut update = job.exit_update(finished);
                    update.topic_published = true;
                    exit_update = Some(update);
                }
                ShellJobLifecycle::Running => unreachable!(),
            }
        }
        job.changed.notify_all();
    }
    if let (Some(listener), Some(update)) = (listener, exit_update) {
        listener(&update);
    }
}

fn parse_exit_status_text(status: &str) -> (Option<i32>, Option<i32>) {
    if let Ok(code) = status.parse::<i32>() {
        (Some(code), None)
    } else if let Some(signal) = status
        .strip_prefix("signal:")
        .and_then(|value| value.parse().ok())
    {
        (None, Some(signal))
    } else {
        (None, None)
    }
}

fn validate_local_shell_request(command: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        validate_powershell_request(command)
    }
    #[cfg(not(windows))]
    {
        validate_bash_request(command)
    }
}

fn validate_local_shell_lifecycle(command: &str, background: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        validate_powershell_lifecycle(command, background)
    }
    #[cfg(not(windows))]
    {
        validate_bash_lifecycle(command, background)
    }
}

#[cfg(windows)]
fn validate_powershell_request(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command_required".to_string());
    }
    let normalized = trimmed.to_ascii_lowercase().replace('`', "");
    let removes_items = normalized.contains("remove-item") || normalized.contains(" rm ");
    let recursive = normalized.contains("-recurse") || normalized.contains("-r ");
    let forced = normalized.contains("-force") || normalized.contains("-fo ");
    let root_target = ["c:\\", "d:\\", "\\", "$env:systemdrive\\"]
        .iter()
        .any(|target| normalized.contains(target));
    if removes_items && recursive && forced && root_target {
        return Err("dangerous_recursive_root_delete".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn validate_powershell_lifecycle(command: &str, background: bool) -> Result<(), String> {
    let normalized = command.to_ascii_lowercase().replace('`', "");
    // No detach keyword checks: escaped processes are contained by the
    // kill-on-close job object, not by semantic scanning.
    let _ = (normalized, background);
    Ok(())
}

pub fn validate_bash_request(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command_required".to_string());
    }
    validate_bash_safety(strip_heredoc_bodies(trimmed).as_str())?;
    Ok(())
}

fn validate_bash_safety(command: &str) -> Result<(), String> {
    let words = shell_words_for_safety_scan(command);
    let mut i = 0;
    while i < words.len() {
        if !is_command_separator(&words[i]) {
            i += 1;
            continue;
        }
        i += 1;
        let Some(rm_index) = rm_command_index(&words, i) else {
            continue;
        };
        i = rm_index;

        let mut recursive = false;
        let mut force = false;
        i += 1;
        while i < words.len() && !is_command_separator(&words[i]) {
            let word = &words[i];
            if word == "--" {
                i += 1;
                break;
            }
            if word.starts_with('-') && word != "-" {
                if word == "--recursive" {
                    recursive = true;
                    i += 1;
                    continue;
                }
                if word == "--force" {
                    force = true;
                    i += 1;
                    continue;
                }
                if !word.starts_with("--") {
                    recursive |= word.chars().skip(1).any(|ch| ch == 'r' || ch == 'R');
                    force |= word.chars().skip(1).any(|ch| ch == 'f');
                    i += 1;
                    continue;
                }
            }
            break;
        }

        while i < words.len() && !is_command_separator(&words[i]) {
            if recursive && force && is_dangerous_rm_target(&words[i]) {
                return Err("dangerous_recursive_root_delete".to_string());
            }
            i += 1;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_bash_lifecycle(command: &str, background: bool) -> Result<(), String> {
    let scanned = strip_heredoc_bodies(command);
    if !background
        && contains_unmanaged_shell_background(scanned.as_str())
        && !contains_shell_wait(scanned.as_str())
    {
        return Err("unmanaged_background_process".to_string());
    }
    // Deliberately no keyword blacklist here: `setsid`-style escapes are
    // contained by OS-level mechanisms (child subreaper adoption on Linux,
    // kill-on-close job objects on Windows), not by semantic scanning.
    for script in nested_shell_scripts(scanned.as_str()) {
        validate_bash_lifecycle(script.as_str(), background)?;
    }
    Ok(())
}

/// Replaces heredoc bodies with spaces so safety/lifecycle scans treat them
/// as data, not shell syntax. A heredoc body may legitimately contain
/// ampersands, quotes, or words like `setsid` that must not trip the
/// background/detach detectors. Bodies are bounded by the terminator word,
/// so removal is finite even for unterminated bodies.
fn strip_heredoc_bodies(command: &str) -> String {
    let chars: Vec<char> = command.chars().collect();
    let mut out = String::with_capacity(command.len());
    let mut index = 0_usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '<' && !in_single && !in_double && chars.get(index + 1).copied() == Some('<') {
            // A here-string (`<<<`) feeds a single word, not a heredoc body;
            // it must not be treated as one, or the delimiter scan would
            // swallow the rest of the command and bypass lifecycle checks.
            if chars.get(index + 2).copied() == Some('<') {
                out.push_str("<<<");
                index += 3;
                continue;
            }
            let mut cursor = index + 2;
            let strip_tabs = chars.get(cursor).copied() == Some('-');
            if strip_tabs {
                cursor += 1;
            }
            let delimiter_start = cursor;
            while cursor < chars.len() && !chars[cursor].is_whitespace() && chars[cursor] != ';' {
                cursor += 1;
            }
            if cursor > delimiter_start {
                // The delimiter word may be quoted (<<'EOF', <<"EOF"); the
                // terminating line always uses the bare word, so strip quotes.
                let raw: String = chars[delimiter_start..cursor].iter().collect();
                let delimiter = raw
                    .strip_prefix('\'')
                    .and_then(|r| r.strip_suffix('\''))
                    .or_else(|| raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
                    .map(str::to_string)
                    .unwrap_or(raw);
                out.push('<');
                out.push('<');
                if strip_tabs {
                    out.push('-');
                }
                out.push_str(delimiter.as_str());
                index = cursor;
                let mut line_start = index;
                let mut body_end = chars.len();
                while line_start < chars.len() {
                    let mut probe = line_start;
                    if strip_tabs {
                        while probe < chars.len() && chars[probe] == '\t' {
                            probe += 1;
                        }
                    }
                    let delimiter_len = delimiter.chars().count();
                    let candidate: String = chars[probe..(probe + delimiter_len).min(chars.len())]
                        .iter()
                        .collect();
                    let after = chars.get(probe + delimiter_len);
                    if candidate == delimiter && after.is_none_or(|c| c.is_whitespace()) {
                        body_end = line_start;
                        break;
                    }
                    match chars[line_start..].iter().position(|c| *c == '\n') {
                        Some(offset) => line_start += offset + 1,
                        None => break,
                    }
                }
                for ch in chars[index..body_end].iter() {
                    out.push(if ch.is_whitespace() { *ch } else { ' ' });
                }
                index = body_end;
                continue;
            }
        }
        out.push(ch);
        index += 1;
    }
    out
}

#[cfg(not(windows))]
fn contains_unmanaged_shell_background(command: &str) -> bool {
    let chars = command.chars().collect::<Vec<_>>();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            index += 1;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            index += 1;
            continue;
        }
        if ch != '&' || in_single || in_double {
            index += 1;
            continue;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        if previous == Some('&')
            || next == Some('&')
            || previous == Some('>')
            || previous == Some('<')
            || previous == Some('|')
            || next == Some('>')
        {
            index += 1;
            continue;
        }
        return true;
    }
    false
}

#[cfg(not(windows))]
fn contains_shell_wait(command: &str) -> bool {
    let words = shell_words_for_safety_scan(command);
    let mut index = 0;
    while index < words.len() {
        if !is_command_separator(&words[index]) {
            index += 1;
            continue;
        }
        index += 1;
        if shell_executable_index(&words, index)
            .is_some_and(|executable| words[executable] == "wait")
        {
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn nested_shell_scripts(command: &str) -> Vec<String> {
    let words = shell_words_for_safety_scan(command);
    let mut scripts = Vec::new();
    let mut index = 0;
    while index < words.len() {
        if !is_command_separator(&words[index]) {
            index += 1;
            continue;
        }
        index += 1;
        let Some(executable) = shell_executable_index(&words, index) else {
            continue;
        };
        if is_shell_interpreter(shell_command_basename(&words[executable])) {
            if let Some(script) = nested_shell_script(&words, executable + 1) {
                scripts.push(script.to_string());
            }
        }
        if shell_command_basename(&words[executable]) == "eval" {
            if let Some(script) = eval_script(&words, executable + 1) {
                scripts.push(script);
            }
        }
        index = executable + 1;
    }
    scripts
}

#[cfg(not(windows))]
fn shell_command_basename(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or(command)
}

#[cfg(not(windows))]
fn is_shell_interpreter(command: &str) -> bool {
    matches!(command, "sh" | "bash" | "dash" | "ksh" | "zsh")
}

#[cfg(not(windows))]
fn nested_shell_script(words: &[String], mut index: usize) -> Option<&str> {
    while index < words.len() && !is_command_separator(&words[index]) {
        let word = words[index].as_str();
        if word == "-c" || (word.starts_with('-') && !word.starts_with("--") && word.contains('c'))
        {
            return words.get(index + 1).map(String::as_str);
        }
        if word == "--" {
            return None;
        }
        index += 1;
    }
    None
}

#[cfg(not(windows))]
fn eval_script(words: &[String], mut index: usize) -> Option<String> {
    // `eval` re-parses its concatenated arguments as a shell script; surface
    // them as one string so the same lifecycle scans apply.
    let mut script = String::new();
    while index < words.len() && !is_command_separator(&words[index]) {
        if !script.is_empty() {
            script.push(' ');
        }
        script.push_str(words[index].as_str());
        index += 1;
    }
    (!script.is_empty()).then_some(script)
}

#[cfg(not(windows))]
fn shell_executable_index(words: &[String], mut index: usize) -> Option<usize> {
    while index < words.len() && !is_command_separator(&words[index]) {
        if is_assignment_word(&words[index])
            || matches!(
                words[index].as_str(),
                "command" | "builtin" | "exec" | "nohup"
            )
        {
            index += 1;
            continue;
        }
        if words[index] == "env" {
            index += 1;
            while index < words.len() && !is_command_separator(&words[index]) {
                let word = words[index].as_str();
                if is_assignment_word(word) {
                    index += 1;
                    continue;
                }
                if word == "--" {
                    index += 1;
                    break;
                }
                if !word.starts_with('-') || word == "-" {
                    break;
                }
                index += 1;
                if matches!(
                    word,
                    "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string"
                ) {
                    index += 1;
                }
            }
            continue;
        }
        if words[index] == "sudo" {
            index += 1;
            while index < words.len() && !is_command_separator(&words[index]) {
                let word = words[index].as_str();
                if word == "--" {
                    index += 1;
                    break;
                }
                if !word.starts_with('-') || word == "-" {
                    break;
                }
                index += 1;
                if matches!(word, "-u" | "-g" | "-h" | "-p" | "-C" | "-T") {
                    index += 1;
                }
            }
            continue;
        }
        return Some(index);
    }
    None
}

fn shell_words_for_safety_scan(command: &str) -> Vec<String> {
    let mut words = vec![";".to_string()];
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if !in_single => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '$' if !in_single && chars.peek() == Some(&'(') => {
                current.push('$');
                current.push(chars.next().unwrap_or('('));
                let mut depth = 1_u32;
                let mut sub_single = false;
                let mut sub_double = false;
                while let Some(next) = chars.next() {
                    current.push(next);
                    match next {
                        '\'' if !sub_double => sub_single = !sub_single,
                        '"' if !sub_single => sub_double = !sub_double,
                        '\\' if !sub_single => {
                            if let Some(escaped) = chars.next() {
                                current.push(escaped);
                            }
                        }
                        '(' if !sub_single && !sub_double => depth = depth.saturating_add(1),
                        ')' if !sub_single && !sub_double => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            '`' if !in_single && !in_double => {
                // Command-substitution boundaries delimit a nested command;
                // treating them as separators keeps `setsid true` inside a
                // substitution visible to the lifecycle/detach scans.
                push_shell_word(&mut words, &mut current);
                push_separator(&mut words);
            }
            ' ' | '\t' if !in_single && !in_double => {
                push_shell_word(&mut words, &mut current);
            }
            '\n' if !in_single && !in_double => {
                // A newline starts a new command like ';' does; treating it as
                // a command separator prevents newline-separated detach or
                // background launches from bypassing the lifecycle scan.
                push_shell_word(&mut words, &mut current);
                push_separator(&mut words);
            }
            ';' if !in_single && !in_double => {
                push_shell_word(&mut words, &mut current);
                push_separator(&mut words);
            }
            '&' | '|' if !in_single && !in_double => {
                push_shell_word(&mut words, &mut current);
                if chars.peek() == Some(&ch) {
                    let _ = chars.next();
                }
                push_separator(&mut words);
            }
            '(' | ')' if !in_single && !in_double => {
                push_shell_word(&mut words, &mut current);
                push_separator(&mut words);
            }
            _ => current.push(ch),
        }
    }
    push_shell_word(&mut words, &mut current);
    words
}

fn push_shell_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

fn push_separator(words: &mut Vec<String>) {
    if words.last().is_none_or(|word| word != ";") {
        words.push(";".to_string());
    }
}

fn is_command_separator(word: &str) -> bool {
    // `{` and `}` delimit brace command groups, so a standalone brace starts
    // or ends a command just like `;` does for the lifecycle scans.
    matches!(word, ";" | "{" | "}" | "then" | "do" | "else")
}

fn is_assignment_word(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn rm_command_index(words: &[String], mut i: usize) -> Option<usize> {
    while i < words.len() && !is_command_separator(&words[i]) {
        if is_assignment_word(&words[i]) {
            i += 1;
            continue;
        }
        match words[i].as_str() {
            "rm" => return Some(i),
            "command" | "builtin" | "exec" | "nohup" => {
                i += 1;
            }
            "sudo" => {
                i += 1;
                while i < words.len() && !is_command_separator(&words[i]) {
                    let word = words[i].as_str();
                    if word == "--" {
                        i += 1;
                        break;
                    }
                    if !word.starts_with('-') || word == "-" {
                        break;
                    }
                    i += 1;
                    if matches!(word, "-u" | "-g" | "-h" | "-p" | "-C" | "-T") {
                        i += 1;
                    }
                }
            }
            "env" => {
                i += 1;
                while i < words.len() && !is_command_separator(&words[i]) {
                    let word = words[i].as_str();
                    if is_assignment_word(word) {
                        i += 1;
                        continue;
                    }
                    if word == "--" {
                        i += 1;
                        break;
                    }
                    if !word.starts_with('-') || word == "-" {
                        break;
                    }
                    i += 1;
                    if matches!(
                        word,
                        "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string"
                    ) {
                        i += 1;
                    }
                }
            }
            _ => return None,
        }
    }
    None
}

fn is_dangerous_rm_target(target: &str) -> bool {
    let clean = target.trim();
    if clean.is_empty() {
        return false;
    }
    if clean.chars().all(|ch| ch == '/') {
        return true;
    }
    matches!(clean, "/." | "/*" | "/./" | "/./*") || starts_with_root_variable_expansion(clean)
}

fn starts_with_root_variable_expansion(target: &str) -> bool {
    let Some(rest) = expansion_tail(target) else {
        return false;
    };
    rest == "/" || rest == "/*" || rest.starts_with("//") || rest.starts_with("/./")
}

fn expansion_tail(target: &str) -> Option<&str> {
    if let Some(rest) = target.strip_prefix("${") {
        let end = rest.find('}')?;
        return Some(&rest[end + 1..]);
    }
    if let Some(rest) = target.strip_prefix("$(") {
        let mut depth = 1_i32;
        let mut in_single = false;
        let mut in_double = false;
        for (idx, ch) in rest.char_indices() {
            match ch {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '(' if !in_single && !in_double => depth += 1,
                ')' if !in_single && !in_double => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&rest[idx + ch.len_utf8()..]);
                    }
                }
                _ => {}
            }
        }
        return None;
    }
    if let Some(rest) = target.strip_prefix('$') {
        let end = rest
            .char_indices()
            .find_map(|(idx, ch)| {
                if ch == '_' || ch.is_ascii_alphanumeric() {
                    None
                } else {
                    Some(idx)
                }
            })
            .unwrap_or(rest.len());
        if end > 0 {
            return Some(&rest[end..]);
        }
    }
    None
}

pub(crate) fn execute_run_bash_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let loop_command = action.input_str("loop_cmd");
    if !loop_command.is_empty() && !action.input_str("cmd").is_empty() {
        let reason = "The action provided both cmd and loop_cmd. Use cmd for a normal/background command, or loop_cmd with interval_ms for polling.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(None, reason),
            "InvalidInput",
            reason,
        ));
    }
    let is_regular_command = loop_command.is_empty();
    let command_to_run = if is_regular_command {
        command_from_action(action)
    } else {
        loop_command.clone()
    };
    let interval_ms = action.input_u64("interval_ms");
    let timeout_ms = if is_regular_command {
        action.timeout_ms_i64(5000)
    } else {
        action
            .input_i64("loop_timeout_ms")
            .unwrap_or(DEFAULT_LOOP_TIMEOUT_MS)
    };
    let session_id = core.current_session_id();
    let turn_id = core.current_action_turn_id();
    let cwd = core.current_prompt_cwd().to_path_buf();
    let tail_out = action
        .raw_input
        .get("tail_out")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let edited_files = action.input_list("edit");
    let tool_call_id = action.call_id.as_str();
    execute_run_bash_with_tail(
        &command_to_run,
        &cwd,
        action.background(),
        timeout_ms,
        interval_ms,
        action.input_u64("once_timeout_ms").unwrap_or(5000),
        edited_files,
        core.bash_approval_mode,
        &core.shell_jobs,
        &session_id,
        &turn_id,
        tool_call_id,
        is_regular_command,
        tail_out,
        runtime,
    )
}

#[cfg(all(test, unix))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_run_bash(
    command: &str,
    cwd: &Path,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    approval_mode: BashApprovalMode,
    shell_jobs: &ShellJobManager,
    session_id: &str,
    turn_id: &str,
    is_regular_command: bool,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    execute_run_bash_with_tail(
        command,
        cwd,
        background,
        timeout_ms,
        interval_ms,
        once_timeout_ms,
        Vec::new(),
        approval_mode,
        shell_jobs,
        session_id,
        turn_id,
        "unknown_tool_call",
        is_regular_command,
        false,
        runtime,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_run_bash_with_tail(
    command: &str,
    cwd: &Path,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    edited_files: Vec<String>,
    approval_mode: BashApprovalMode,
    shell_jobs: &ShellJobManager,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    is_regular_command: bool,
    tail_out: bool,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let command_to_run = command.trim();
    if command_to_run.is_empty() {
        let reason = "The command was not executed because no shell command was provided.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(None, reason),
            "InvalidInput",
            reason,
        ));
    }
    if let Err(reason) = validate_local_shell_request(command_to_run) {
        let message = bash_validation_message(&reason);
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), message),
            "InvalidInput",
            message,
        ));
    }
    if let Err(reason) = validate_local_shell_lifecycle(command_to_run, background) {
        let message = bash_validation_message(&reason);
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), message),
            "InvalidInput",
            message,
        ));
    }
    if !background && is_regular_command && timeout_ms <= 0 {
        let reason =
            "timeout_ms must be a positive integer. Choose a wait budget that matches the command.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), reason),
            "InvalidInput",
            reason,
        ));
    }
    if !background && !is_regular_command && timeout_ms <= 0 {
        let reason = "loop_timeout_ms must be a positive integer. Choose a total polling wait budget that matches the external state you are waiting for.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), reason),
            "InvalidInput",
            reason,
        ));
    }
    if !background && is_regular_command && contains_long_local_shell_sleep(command_to_run) {
        let reason = "The command contains a long sleep in normal mode. Use loop_cmd with interval_ms to poll external status, or background=true for long local work that should continue across turns. If the `sleep N` text is only data inside a script or fixture, put it inside a quoted string or a heredoc (<<'EOF') body and retry.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), reason),
            "InvalidInput",
            reason,
        ));
    }
    if background && interval_ms.is_some() {
        let reason = "Polling mode and background mode cannot be combined. Use loop_cmd with interval_ms for polling, or background=true for a persistent background command.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), reason),
            "InvalidInput",
            reason,
        ));
    }
    if interval_ms.is_some() && is_regular_command {
        let reason = "interval_ms is only valid with loop_cmd. Move the check command to loop_cmd, or remove interval_ms for a normal command.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), reason),
            "InvalidInput",
            reason,
        ));
    }
    if interval_ms.is_none() && !is_regular_command {
        let reason =
            "loop_cmd needs interval_ms so the runtime knows how often to check the condition.";
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), reason),
            "InvalidInput",
            reason,
        ));
    }
    if approval_mode == BashApprovalMode::Ask {
        return ActionExecution::NeedsApproval(PendingApproval {
            request: ApprovalRequest {
                approval_id: format!("approval_{}", now_ms()),
                action: crate::os::local_shell_tool_name().to_string(),
                command: command_to_run.to_string(),
                reason: format!(
                    "{}_requires_user_approval",
                    crate::os::local_shell_tool_name()
                ),
                risk: "local_command_execution".to_string(),
            },
            approved_action: PendingApprovedAction::RunBash {
                command: command_to_run.to_string(),
                background,
                timeout_ms,
                interval_ms,
                once_timeout_ms,
                edited_files,
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                tool_call_id: tool_call_id.to_string(),
                cwd: cwd.to_path_buf(),
                tail_out,
            },
            action_name: None,
            action_call_id: tool_call_id.to_string(),
            continuation: None,
        });
    }
    let mut outcome = if background {
        shell_jobs.spawn_background_outcome(
            command_to_run,
            cwd,
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
        )
    } else if let Some(interval_ms) = interval_ms {
        execute_polling_bash_outcome_with_tail(
            command_to_run,
            cwd,
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            tail_out,
            runtime,
        )
    } else {
        shell_jobs.run_with_timeout_outcome(
            command_to_run,
            cwd,
            timeout_ms,
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
            runtime,
        )
    };
    append_edited_files_note(&mut outcome.text, &edited_files);
    ActionExecution::Completed(outcome)
}

fn append_edited_files_note(text: &mut String, edited_files: &[String]) {
    if edited_files.is_empty() {
        return;
    }
    text.push_str(
        "
edited_files: ",
    );
    text.push_str(&edited_files.join(", "));
}

#[cfg(all(test, unix))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_approved_bash(
    command: &str,
    cwd: &Path,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    session_id: &str,
    turn_id: &str,
    is_regular_command: bool,
    request: &ApprovalRequest,
    shell_jobs: &ShellJobManager,
    runtime: &mut dyn ActionRuntime,
) -> ActionOutcome {
    execute_approved_bash_with_tail(
        command,
        cwd,
        background,
        timeout_ms,
        interval_ms,
        once_timeout_ms,
        session_id,
        turn_id,
        "unknown_tool_call",
        is_regular_command,
        false,
        &[],
        request,
        shell_jobs,
        runtime,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_approved_bash_with_tail(
    command: &str,
    cwd: &Path,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    _is_regular_command: bool,
    tail_out: bool,
    edited_files: &[String],
    request: &ApprovalRequest,
    shell_jobs: &ShellJobManager,
    runtime: &mut dyn ActionRuntime,
) -> ActionOutcome {
    let clean = command.trim();
    if let Err(reason) = validate_local_shell_request(clean) {
        let message = bash_validation_message(&reason);
        let outcome = bash_finished_error_outcome(
            bash_action_not_executed(Some(clean), message),
            "InvalidInput",
            message,
        );
        return outcome
            .with_runtime_metadata("approval_id", request.approval_id.clone())
            .with_runtime_metadata("approval_status", "approved_by_user");
    }
    if let Err(reason) = validate_local_shell_lifecycle(clean, background) {
        let message = bash_validation_message(&reason);
        let outcome = bash_finished_error_outcome(
            bash_action_not_executed(Some(clean), message),
            "InvalidInput",
            message,
        );
        return outcome
            .with_runtime_metadata("approval_id", request.approval_id.clone())
            .with_runtime_metadata("approval_status", "approved_by_user");
    }
    let mut outcome = if background {
        shell_jobs.spawn_background_outcome(clean, cwd, session_id, turn_id, tool_call_id, tail_out)
    } else if let Some(interval_ms) = interval_ms {
        execute_polling_bash_outcome_with_tail(
            clean,
            cwd,
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            tail_out,
            runtime,
        )
    } else {
        shell_jobs.run_with_timeout_outcome(
            clean,
            cwd,
            timeout_ms,
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
            runtime,
        )
    };
    append_edited_files_note(&mut outcome.text, edited_files);
    outcome
        .with_runtime_metadata("approval_id", request.approval_id.clone())
        .with_runtime_metadata("approval_status", "approved_by_user")
}

pub fn execute_one_bash(command: &str, timeout_ms: i64, runtime: &mut dyn ActionRuntime) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    execute_one_bash_structured(command, &cwd, timeout_ms, runtime)
        .to_action_result(crate::os::local_shell_tool_name())
}

#[cfg(all(test, unix))]
pub(crate) fn execute_polling_bash_outcome(
    command: &str,
    cwd: &Path,
    interval_ms: u64,
    timeout_ms: i64,
    once_timeout_ms: u64,
    runtime: &mut dyn ActionRuntime,
) -> ActionOutcome {
    execute_polling_bash_outcome_with_tail(
        command,
        cwd,
        interval_ms,
        timeout_ms,
        once_timeout_ms,
        false,
        runtime,
    )
}

pub(crate) fn execute_polling_bash_outcome_with_tail(
    command: &str,
    cwd: &Path,
    interval_ms: u64,
    timeout_ms: i64,
    once_timeout_ms: u64,
    tail_out: bool,
    runtime: &mut dyn ActionRuntime,
) -> ActionOutcome {
    if timeout_ms <= 0 {
        return polling_result(
            command,
            "not_executed",
            0,
            Duration::ZERO,
            None,
            None,
            "",
            "",
            "",
            tail_out,
            Some("loop_timeout_ms must be a positive integer."),
        );
    }
    if interval_ms == 0 {
        return polling_result(
            command,
            "not_executed",
            0,
            Duration::ZERO,
            None,
            None,
            "",
            "",
            "",
            tail_out,
            Some("interval_ms must be a positive integer."),
        );
    }
    if once_timeout_ms == 0 {
        return polling_result(
            command,
            "not_executed",
            0,
            Duration::ZERO,
            None,
            None,
            "",
            "",
            "",
            tail_out,
            Some("once_timeout_ms must be a positive integer."),
        );
    }
    let interval = Duration::from_millis(interval_ms);
    let max_wait = Duration::from_millis(timeout_ms as u64);
    let started = Instant::now();
    let mut attempts = 0_u64;
    let mut last_status = None;
    let mut last_output = String::new();
    let mut last_stdout = String::new();
    let mut last_stderr = String::new();
    let mut last_signal = None;
    let mut last_error = None;

    loop {
        if runtime.should_cancel() {
            return polling_result(
                command,
                "cancelled",
                attempts,
                started.elapsed(),
                last_status,
                last_signal,
                &last_stdout,
                &last_stderr,
                &last_output,
                tail_out,
                last_error.as_deref(),
            );
        }
        if runtime.should_force_handoff() {
            return polling_result(
                command,
                "dispatch_timeout_interrupted",
                attempts,
                started.elapsed(),
                last_status,
                last_signal,
                &last_stdout,
                &last_stderr,
                &last_output,
                tail_out,
                last_error.as_deref(),
            );
        }

        attempts = attempts.saturating_add(1);
        let result = execute_one_bash_structured(command, cwd, once_timeout_ms as i64, runtime);
        last_status = result.status;
        last_signal = result.signal;
        last_stdout = result.stdout;
        last_stderr = result.stderr;
        last_output = result.output;
        last_error = result.error;

        if let Some(status) = last_status {
            if status == 0 {
                return polling_result(
                    command,
                    "finished",
                    attempts,
                    started.elapsed(),
                    last_status,
                    last_signal,
                    &last_stdout,
                    &last_stderr,
                    &last_output,
                    tail_out,
                    None,
                );
            }
        }

        if started.elapsed() >= max_wait {
            return polling_result(
                command,
                "timeout",
                attempts,
                started.elapsed(),
                last_status,
                last_signal,
                &last_stdout,
                &last_stderr,
                &last_output,
                tail_out,
                last_error.as_deref(),
            );
        }

        let wait = interval.min(max_wait.saturating_sub(started.elapsed()));
        sleep_cancelable(wait, &mut || runtime.should_cancel());
    }
}

#[allow(clippy::too_many_arguments)]
fn polling_result(
    _command: &str,
    state: &str,
    attempts: u64,
    elapsed: Duration,
    last_status: Option<i32>,
    last_signal: Option<i32>,
    stdout: &str,
    stderr: &str,
    output: &str,
    _tail_out: bool,
    error: Option<&str>,
) -> ActionOutcome {
    let state_sentence = match state {
        "finished" => "The polling command finished because the check command exited with code 0.",
        "timeout" => "The polling command stopped because the total wait budget was reached before the check command exited with code 0.",
        "cancelled" => "The polling command was cancelled before the check command exited with code 0.",
        "dispatch_timeout_interrupted" => "The polling wait was interrupted because an accepted user supplement reached its dispatch deadline; the waited task was not cancelled by the user.",
        _ => "The polling command stopped.",
    };
    let mut out = format!(
        "Action result: {}\n{state_sentence}\nPolling state: {}\nAttempts: {}\nElapsed: {} ms\nSuccess condition: exit code 0",
        crate::os::local_shell_tool_name(),
        state,
        attempts,
        elapsed.as_millis()
    );
    out.push_str("\nExit-code semantics: exit_code is from the last loop_cmd execution; polling does not know the waited task's own exit code unless loop_cmd reads and reports it.");
    if let Some(status) = last_status {
        out.push_str(&format!("\nLast loop_cmd exit code: {status}"));
    }
    if let Some(error) = error {
        out.push_str(&format!("\nLast execution problem: {error}"));
    }
    if !output.trim().is_empty() {
        out.push_str("\nLast output:\n");
        out.push_str(output);
    }
    let status = match state {
        "finished" => ActionStatus::Completed,
        "timeout" => ActionStatus::Timeout,
        "cancelled" | "dispatch_timeout_interrupted" => ActionStatus::Cancelled,
        _ => ActionStatus::Failed,
    };
    let mut outcome = ActionOutcome::new(status, out)
        .with_runtime_metadata("polling_state", state)
        .with_runtime_metadata("polling_attempts", attempts)
        .with_runtime_metadata("polling_elapsed_ms", elapsed.as_millis() as u64)
        .with_runtime_metadata("success_condition", "loop_cmd exit code 0")
        .with_runtime_metadata(
            "exit_code_semantics",
            "exit_code belongs to the last loop_cmd execution, not automatically to the waited task",
        )
        .with_bash_result(BashResultEvidence {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            stdout_truncation: None,
            stderr_truncation: None,
            exit_code: last_status,
            signal: last_signal,
            pid: None,
            timed_out: false,
            pid_kind: None,
            error_type: match state {
                "cancelled" | "dispatch_timeout_interrupted" => Some("Cancelled".to_string()),
                "not_executed" => Some("InvalidInput".to_string()),
                _ => None,
            },
        });
    if let Some(error) = error {
        outcome = outcome.with_runtime_metadata("last_execution_problem", error);
    }
    outcome
}

fn sleep_cancelable(duration: Duration, cancelled: &mut impl FnMut() -> bool) {
    let started = Instant::now();
    while started.elapsed() < duration {
        if cancelled() {
            return;
        }
        let remaining = duration.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

fn command_from_action(action: &ParsedAction) -> String {
    action.input_str("cmd")
}

fn contains_long_local_shell_sleep(command: &str) -> bool {
    #[cfg(windows)]
    {
        contains_long_powershell_sleep(command)
    }
    #[cfg(not(windows))]
    {
        contains_long_normal_sleep(command)
    }
}

#[cfg(windows)]
fn contains_long_powershell_sleep(command: &str) -> bool {
    let tokens = command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '(' | ')'))
        .filter(|token| !token.is_empty())
        .map(|token| {
            token
                .trim_matches(|ch| ch == '\'' || ch == '"')
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    tokens.windows(2).any(|pair| {
        matches!(pair[0].as_str(), "start-sleep" | "sleep")
            && pair[1]
                .trim_start_matches("-seconds:")
                .parse::<f64>()
                .ok()
                .is_some_and(|seconds| seconds >= 30.0)
    }) || tokens.windows(3).any(|triple| {
        matches!(triple[0].as_str(), "start-sleep" | "sleep")
            && triple[1] == "-seconds"
            && triple[2]
                .parse::<f64>()
                .ok()
                .is_some_and(|seconds| seconds >= 30.0)
    })
}

#[cfg(not(windows))]
fn contains_long_normal_sleep(command: &str) -> bool {
    // Heredoc bodies and quoted strings are data, not commands: sample
    // command text inside a script or heredoc must not trip the sleep scan.
    // Scan word segments and only accept `sleep` in executable position.
    let words = shell_words_for_safety_scan(strip_heredoc_bodies(command).as_str());
    let mut index = 0;
    while index < words.len() {
        if !is_command_separator(&words[index]) {
            index += 1;
            continue;
        }
        index += 1;
        let Some(executable) = shell_executable_index(&words, index) else {
            continue;
        };
        if words[executable] == "sleep"
            && words
                .get(executable + 1)
                .and_then(|arg| sleep_arg_seconds(arg))
                .is_some_and(|seconds| seconds >= 30.0)
        {
            return true;
        }
        index = executable + 1;
    }
    false
}
fn sleep_arg_seconds(arg: &str) -> Option<f64> {
    let clean = arg.trim();
    let (number, multiplier) = if let Some(number) = clean.strip_suffix('s') {
        (number, 1.0)
    } else if let Some(number) = clean.strip_suffix('m') {
        (number, 60.0)
    } else if let Some(number) = clean.strip_suffix('h') {
        (number, 3600.0)
    } else {
        (clean, 1.0)
    };
    number.parse::<f64>().ok().map(|value| value * multiplier)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashCommandOutput {
    pub command: String,
    pub status: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncation: Option<StreamCaptureTruncation>,
    pub stderr_truncation: Option<StreamCaptureTruncation>,
    pub output: String,
    pub error: Option<String>,
    pub job_management: Option<String>,
    pub tail_out: bool,
}

impl BashCommandOutput {
    pub fn to_action_result(&self, action_name: &str) -> String {
        self.to_action_outcome(action_name).text
    }

    pub(crate) fn to_action_outcome(&self, action_name: &str) -> ActionOutcome {
        let text = self.render_action_result(action_name);
        let status = if let Some(error) = self.error.as_deref() {
            match error {
                "timeout" => ActionStatus::Timeout,
                "cancelled" | "cancelled_by_user" => ActionStatus::Cancelled,
                _ if error.starts_with("timeout_still_running:")
                    || error.starts_with("long_running_still_running:") =>
                {
                    ActionStatus::BackgroundRunning
                }
                _ => ActionStatus::Failed,
            }
        } else if self.signal.is_some() {
            ActionStatus::Failed
        } else if self.status == Some(0) {
            ActionStatus::Completed
        } else {
            ActionStatus::Failed
        };
        let running_error = self.error.as_deref();
        let pid = running_error.and_then(bash_running_pid);
        let timed_out =
            running_error.is_some_and(|error| error.starts_with("timeout_still_running:"));
        let pid_kind = pid.map(|_| runtime_child_pid_kind().to_string());
        ActionOutcome::new(status, text).with_bash_result(BashResultEvidence {
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
            stdout_truncation: self.stdout_truncation.clone(),
            stderr_truncation: self.stderr_truncation.clone(),
            exit_code: self.status,
            signal: self.signal,
            pid,
            timed_out,
            pid_kind,
            error_type: self
                .error
                .as_deref()
                .and_then(bash_error_type)
                .map(str::to_string),
        })
    }

    fn render_action_result(&self, action_name: &str) -> String {
        if let Some(error) = &self.error {
            if let Some(details) = error.strip_prefix("long_running_still_running:") {
                let (pid, elapsed_ms) = details.split_once(':').unwrap_or((details, "unknown"));
                let mut out = format!(
                    "Action result: {}\nLONG_RUNNING_COMMAND_STATUS:\nPID: {}\nElapsed: {} ms\nStatus: still running\n{}",
                    action_name, pid, elapsed_ms, LONG_RUNNING_ACTION_GUIDANCE
                );
                if !self.output.trim().is_empty() {
                    out.push_str("\nPartial return:\n");
                    out.push_str(&self.output);
                }
                return out;
            }
            if let Some(pid) = error.strip_prefix("timeout_still_running:") {
                let mut out = format!(
                    "Action result: {}\npid={}, timeout, but is still running\nTimeout means Timem stopped waiting; the process was not killed and there is no final exit code yet.",
                    action_name, pid
                );
                if !self.output.trim().is_empty() {
                    out.push_str("\nPartial return:\n");
                    out.push_str(&self.output);
                }
                return out;
            }
            return format!(
                "Action result: {}\n{}",
                action_name,
                bash_runtime_error_message(error)
            );
        }
        if let Some(signal) = self.signal {
            return format!(
                "Action result: {}\nThe command terminated because of a process signal.\nSignal: {}\nReturn:\n{}",
                action_name,
                signal,
                self.output
            );
        }
        format!(
            "Action result: {}\nThe command finished.\nExit code: {}\nReturn:\n{}",
            action_name,
            self.status.unwrap_or(-1),
            self.output
        )
    }
}

pub fn execute_one_bash_structured(
    command: &str,
    cwd: &Path,
    timeout_ms: i64,
    runtime: &mut dyn ActionRuntime,
) -> BashCommandOutput {
    execute_one_bash_structured_with_prompt_after(
        command,
        cwd,
        timeout_ms,
        runtime,
        LONG_RUNNING_COMMAND_PROMPT_AFTER,
    )
}

fn execute_one_bash_structured_with_prompt_after(
    command: &str,
    cwd: &Path,
    timeout_ms: i64,
    runtime: &mut dyn ActionRuntime,
    long_running_prompt_after: Duration,
) -> BashCommandOutput {
    if timeout_ms <= 0 {
        return bash_error(command, "invalid_timeout");
    }
    let mut shell = match crate::os::command_for_local_shell(command) {
        Ok(command) => command,
        Err(_) => return bash_error(command, "command_failed"),
    };
    configure_run_bash_environment(&mut shell);
    shell
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::os::configure_child_process_group(&mut shell);
    let mut child = match shell.spawn() {
        Ok(child) => child,
        Err(_) => return bash_error(command, "command_failed"),
    };
    let _child_registration = crate::os::register_managed_child(child.id());
    let stdout = Arc::new(Mutex::new(BoundedShellOutput::new(false)));
    let stderr = Arc::new(Mutex::new(BoundedShellOutput::new(false)));
    let stdout_drain = child
        .stdout
        .take()
        .map(|pipe| spawn_output_drain(pipe, Arc::clone(&stdout)));
    let stderr_drain = child
        .stderr
        .take()
        .map(|pipe| spawn_output_drain(pipe, Arc::clone(&stderr)));
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms as u64);
    let mut next_long_running_check = long_running_prompt_after;
    let exit_status = loop {
        if runtime.should_cancel() {
            terminate_process(child.id());
            let _ = child.wait();
            join_output_drains(stdout_drain, stderr_drain);
            return bash_error(command, "cancelled");
        }
        if started.elapsed() >= next_long_running_check && started.elapsed() < timeout {
            let status = LongRunningCommandStatus {
                action: crate::os::local_shell_tool_name().to_string(),
                command: command.to_string(),
                pid: child.id(),
                elapsed: started.elapsed(),
                timeout_ms: Some(timeout_ms),
            };
            runtime.on_long_running_command(&status);
            next_long_running_check =
                next_long_running_check.saturating_add(long_running_prompt_after);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_process(child.id());
                let _ = child.wait();
                join_output_drains(stdout_drain, stderr_drain);
                return bash_error(command, "timeout");
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                terminate_process(child.id());
                let _ = child.wait();
                join_output_drains(stdout_drain, stderr_drain);
                return bash_error(command, "command_failed");
            }
        }
    };
    join_output_drains(stdout_drain, stderr_drain);
    let stdout = shell_output_snapshot(&stdout);
    let stderr = shell_output_snapshot(&stderr);
    BashCommandOutput {
        command: command.to_string(),
        status: exit_status.code(),
        signal: exit_signal(&exit_status),
        output: combined_shell_output(&stdout.text, &stderr.text),
        stdout: stdout.text,
        stderr: stderr.text,
        stdout_truncation: stdout.truncation,
        stderr_truncation: stderr.truncation,
        error: None,
        job_management: None,
        tail_out: false,
    }
}

fn combined_shell_output(stdout: &str, stderr: &str) -> String {
    let mut combined = String::new();
    if !stdout.trim().is_empty() {
        combined.push_str(stdout.trim_end());
    }
    if !stderr.trim().is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("stderr: ");
        combined.push_str(stderr.trim_end());
    }
    if combined.is_empty() {
        "<no output>".to_string()
    } else {
        combined
    }
}

fn bash_error_type(error: &str) -> Option<&'static str> {
    match error {
        "cancelled" | "cancelled_by_user" => Some("Cancelled"),
        "invalid_timeout" => Some("InvalidInput"),
        "command_failed" => Some("SpawnFailed"),
        _ if error.starts_with("timeout_still_running:")
            || error.starts_with("long_running_still_running:") =>
        {
            None
        }
        "timeout" => None,
        _ => Some("InternalError"),
    }
}

fn bash_finished_error_outcome(
    text: String,
    error_type: &'static str,
    error_message: impl Into<String>,
) -> ActionOutcome {
    ActionOutcome::failed(text).with_bash_result(BashResultEvidence {
        stdout: String::new(),
        stderr: error_message.into(),
        stdout_truncation: None,
        stderr_truncation: None,
        exit_code: None,
        signal: None,
        pid: None,
        timed_out: false,
        pid_kind: None,
        error_type: Some(error_type.to_string()),
    })
}

fn is_runtime_child_pid(pid: u32) -> bool {
    crate::os::is_runtime_child_process_group(pid)
}

fn runtime_child_pid_kind() -> &'static str {
    crate::os::runtime_child_pid_kind()
}

fn bash_running_pid(error: &str) -> Option<u32> {
    if let Some(pid) = error.strip_prefix("timeout_still_running:") {
        return pid.parse().ok();
    }
    error
        .strip_prefix("long_running_still_running:")
        .and_then(|details| details.split(':').next())
        .and_then(|pid| pid.parse().ok())
}

fn bash_error(command: &str, error: &str) -> BashCommandOutput {
    let diagnostic = bash_runtime_error_message(error).to_string();
    BashCommandOutput {
        command: command.to_string(),
        status: None,
        signal: None,
        stdout: String::new(),
        stderr: diagnostic.clone(),
        stdout_truncation: None,
        stderr_truncation: None,
        output: diagnostic,
        error: Some(error.to_string()),
        job_management: None,
        tail_out: false,
    }
}

fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    crate::os::exit_signal(status)
}

fn bash_action_not_executed(_command: Option<&str>, reason: &str) -> String {
    format!(
        "Action result: {}\nThe command was not executed.\nReason: {reason}",
        crate::os::local_shell_tool_name()
    )
}

fn bash_validation_message(reason: &str) -> &'static str {
    match reason {
        "command_required" => "No shell command was provided.",
        "dangerous_recursive_root_delete" => {
            "The local command was blocked by Timem safety policy because it may recursively delete the filesystem root."
        }
        "unmanaged_background_process" => {
            "检测到命令可能创建脱离 Runtime 管理的后台进程（未加 wait 的 `&` 后台启动）。请改用当前平台命令工具的 background=true；若 `&` 只是数据/样例文本，请放入引号字符串或 heredoc（<<'EOF'）正文中后重试。"
        }
        _ => "The local command request did not pass runtime validation.",
    }
}

fn bash_runtime_error_message(error: &str) -> &'static str {
    match error {
        "timeout" => {
            "Timem stopped waiting because the configured timeout was reached. This message does not by itself mean the process was killed. For long local work, use background=true; for waiting on external state, use loop_cmd with interval_ms."
        }
        "cancelled" | "cancelled_by_user" => {
            "The command was cancelled before it completed."
        }
        "invalid_timeout" => {
            "The command was not executed because timeout_ms must be a positive integer."
        }
        "command_failed" => {
            "The local shell could not start or wait for the command successfully."
        }
        _ => "The command did not complete successfully.",
    }
}

fn exit_status_text(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return code.to_string();
    }
    if let Some(signal) = exit_signal(status) {
        return format!("signal:{signal}");
    }
    "unknown".to_string()
}

fn normalized_shell_output(output: &str) -> String {
    let clean = output.trim_end();
    if clean.trim().is_empty() {
        "<no output>".to_string()
    } else {
        clean.to_string()
    }
}

#[cfg(all(test, unix))]
fn process_running(pid: u32) -> bool {
    crate::os::child_process_running(pid)
}

#[cfg(all(test, unix))]
fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn terminate_process(pid: u32) {
    crate::os::terminate_process(pid);
}

pub(crate) fn compact_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized;
    }
    let mut result = normalized.chars().take(max_chars).collect::<String>();
    result.push('…');
    result
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(all(test, unix))]
fn unique_shell_id(prefix: &str) -> String {
    let seq = SHELL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, now_ms(), seq)
}

#[cfg(all(test, unix))]
#[path = "../../../core/agent/tests/unit/capability_tool_run_bash_tests.rs"]
mod tests;

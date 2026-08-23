use crate::response_protocol::ParsedAction;
use crate::MemGuard;
use crate::{
    ActionExecution, ActionOutcome, ActionRuntime, ActionStatus, AgentCore, ApprovalRequest,
    BashApprovalMode, BashResultEvidence, LongRunningCommandStatus, PendingApproval,
    PendingApprovedAction,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
const LONG_RUNNING_COMMAND_PROMPT_AFTER: Duration = Duration::from_secs(60);

fn configure_run_bash_environment(command: &mut Command) {
    command
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("TERM", "dumb");
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellJobRecord {
    pub id: String,
    pub created_at_ms: i64,
    #[serde(default = "default_shell_job_kind")]
    pub kind: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    pub pid: u32,
    #[serde(default)]
    pub process_identity: Option<String>,
    #[serde(default)]
    pub tool_call_id: String,
    #[serde(default)]
    pub owner_id: Option<String>,
    pub command: String,
    #[serde(default)]
    pub cwd: String,
    pub output_file: String,
    #[serde(default)]
    pub stderr_file: String,
    pub status_file: String,
    #[serde(default)]
    pub tail_out: bool,
}

fn default_shell_job_kind() -> String {
    "background".to_string()
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellJobExitUpdate {
    pub pid: u32,
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

#[derive(Debug, Clone)]
pub struct FileShellJobStore {
    dir: PathBuf,
    index_file: PathBuf,
    guard: MemGuard,
    watcher: ShellJobWatcher,
    long_running_prompt_after: Duration,
}

#[derive(Debug, Clone)]
struct ShellJobWatcher {
    state: Arc<ShellJobWatcherState>,
}

#[derive(Debug)]
struct ShellJobWatcherState {
    jobs: Mutex<HashMap<u32, WatchedShellChild>>,
    changed: Condvar,
    started: AtomicBool,
}

#[derive(Debug)]
struct WatchedShellChild {
    child: Child,
    status_file: PathBuf,
    launcher_status: Option<String>,
}

impl ShellJobWatcher {
    fn new() -> Self {
        Self {
            state: Arc::new(ShellJobWatcherState {
                jobs: Mutex::new(HashMap::new()),
                changed: Condvar::new(),
                started: AtomicBool::new(false),
            }),
        }
    }

    fn register(&self, pid: u32, child: Child, status_file: PathBuf) {
        self.ensure_started();
        if let Ok(mut jobs) = self.state.jobs.lock() {
            jobs.insert(
                pid,
                WatchedShellChild {
                    child,
                    status_file,
                    launcher_status: None,
                },
            );
            self.state.changed.notify_one();
        }
    }

    fn is_watching(&self, pid: u32) -> bool {
        self.state
            .jobs
            .lock()
            .ok()
            .map(|jobs| jobs.contains_key(&pid))
            .unwrap_or(false)
    }

    fn refresh_pid(&self, pid: u32) {
        let Ok(mut jobs) = self.state.jobs.lock() else {
            return;
        };
        let Some(watched) = jobs.get_mut(&pid) else {
            return;
        };
        refresh_watched_shell_child(pid, watched);
        if watched_shell_child_finished(pid, watched) {
            if let Some(watched) = jobs.remove(&pid) {
                write_status_if_empty(
                    &watched.status_file,
                    watched.launcher_status.as_deref().unwrap_or("unknown"),
                );
            }
        }
    }

    fn ensure_started(&self) {
        if self
            .state
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let state = Arc::clone(&self.state);
        thread::spawn(move || shell_job_watcher_loop(state));
    }
}

fn watched_shell_child_finished(pid: u32, watched: &WatchedShellChild) -> bool {
    watched
        .launcher_status
        .as_deref()
        .is_some_and(|status| status.starts_with("signal:"))
        || (watched.launcher_status.is_some() && !crate::os::process_group_running(pid))
}

fn refresh_watched_shell_child(pid: u32, watched: &mut WatchedShellChild) {
    if watched.launcher_status.is_some() {
        return;
    }
    watched.launcher_status = match watched.child.try_wait() {
        Ok(Some(status)) => {
            if exit_signal(&status).is_some() {
                crate::os::kill_process_group(pid);
            }
            Some(exit_status_text(&status))
        }
        Ok(None) => None,
        Err(_) => Some("unknown".to_string()),
    };
}

fn shell_job_watcher_loop(state: Arc<ShellJobWatcherState>) {
    let mut jobs = match state.jobs.lock() {
        Ok(jobs) => jobs,
        Err(_) => return,
    };
    loop {
        while jobs.is_empty() {
            jobs = match state.changed.wait(jobs) {
                Ok(jobs) => jobs,
                Err(_) => return,
            };
        }

        let mut finished = Vec::new();
        for (pid, watched) in jobs.iter_mut() {
            refresh_watched_shell_child(*pid, watched);
            if watched_shell_child_finished(*pid, watched) {
                finished.push(*pid);
            }
        }
        for pid in finished {
            if let Some(watched) = jobs.remove(&pid) {
                write_status_if_empty(
                    &watched.status_file,
                    watched.launcher_status.as_deref().unwrap_or("unknown"),
                );
            }
        }

        if jobs.is_empty() {
            continue;
        }
        jobs = match state.changed.wait_timeout(jobs, Duration::from_millis(100)) {
            Ok((jobs, _)) => jobs,
            Err(_) => return,
        };
    }
}

fn write_status_if_empty(path: &Path, status: &str) {
    if fs::read_to_string(path)
        .ok()
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        return;
    }
    let _ = fs::write(path, status);
}

impl FileShellJobStore {
    pub fn new(memory_dir: &Path) -> Self {
        let dir = memory_dir.join("shell_jobs");
        let _ = fs::create_dir_all(&dir);
        Self {
            index_file: dir.join("jobs.jsonl"),
            dir,
            guard: MemGuard::for_memory_domain(memory_dir, "shell-jobs"),
            watcher: ShellJobWatcher::new(),
            long_running_prompt_after: LONG_RUNNING_COMMAND_PROMPT_AFTER,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_long_running_prompt_after_for_tests(&mut self, duration: Duration) {
        self.long_running_prompt_after = duration.max(Duration::from_millis(1));
    }

    #[cfg(test)]
    pub(crate) fn forget_watched_job_for_tests(&self, pid: u32) {
        if let Ok(mut jobs) = self.watcher.state.jobs.lock() {
            jobs.remove(&pid);
        }
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
        let record = match self.spawn_record(
            clean,
            cwd,
            "background",
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
        ) {
            Ok(record) => record,
            Err(_) => {
                let reason = "The background command could not be started by the local shell.";
                return bash_finished_error_outcome(
                    bash_action_not_executed(Some(clean), reason),
                    "SpawnFailed",
                    reason,
                );
            }
        };
        let _ = self.append(&record);
        ActionOutcome::background_running(format!(
            "Action result: run_bash\npid={}, now keeps running in background",
            record.pid
        ))
        .with_bash_result(BashResultEvidence {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            signal: None,
            pid: Some(record.pid),
            timed_out: false,
            pid_kind: Some(runtime_child_pid_kind().to_string()),
            error_type: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_record(
        &self,
        clean: &str,
        cwd: &Path,
        kind: &str,
        session_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        tail_out: bool,
    ) -> std::io::Result<ShellJobRecord> {
        fs::create_dir_all(&self.dir)?;
        let id = unique_shell_id("job");
        let output_file = self.dir.join(format!("{id}.out"));
        let stderr_file = self.dir.join(format!("{id}.err"));
        let status_file = self.dir.join(format!("{id}.status"));
        let output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&output_file)?;
        let stderr = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&stderr_file)?;
        let mut command = Command::new(crate::os::BASH_EXECUTABLE);
        configure_run_bash_environment(&mut command);
        command
            .arg("-c")
            .arg(clean)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(stderr));
        crate::os::configure_child_process_group(&mut command);
        let child = command.spawn()?;
        let pid = child.id();
        if !is_runtime_child_pid(pid) {
            terminate_process(pid);
            return Err(std::io::Error::other(
                "spawned process did not satisfy the managed-child PID invariant",
            ));
        }
        self.watcher.register(pid, child, status_file.clone());
        Ok(ShellJobRecord {
            id,
            created_at_ms: now_ms(),
            kind: kind.to_string(),
            session_id: session_id.trim().to_string(),
            turn_id: turn_id.trim().to_string(),
            pid,
            process_identity: crate::os::process_identity(pid),
            tool_call_id: tool_call_id.trim().to_string(),
            owner_id: Some(crate::runtime_process_owner_id().to_string()),
            command: clean.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            output_file: output_file.to_string_lossy().to_string(),
            stderr_file: stderr_file.to_string_lossy().to_string(),
            status_file: status_file.to_string_lossy().to_string(),
            tail_out,
        })
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
        .to_action_outcome("run_bash")
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
        let Ok(record) = self.spawn_record(
            clean,
            cwd,
            "timeout",
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
        ) else {
            return bash_error(clean, "command_failed");
        };
        let started = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);
        let next_long_running_check = self.long_running_prompt_after;
        loop {
            if runtime.should_cancel() {
                terminate_process(record.pid);
                write_status_if_empty(Path::new(&record.status_file), "cancelled");
                return bash_error(clean, "cancelled");
            }
            self.watcher.refresh_pid(record.pid);
            if let Some(status) = read_process_status(&record.status_file) {
                let (stdout, stderr) = read_shell_job_streams(&record);
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: status.code,
                    signal: status.signal,
                    output: normalized_shell_output(&combined_shell_output(&stdout, &stderr)),
                    stdout,
                    stderr,
                    error: None,
                    tail_out,
                };
            }
            if started.elapsed() >= next_long_running_check && started.elapsed() < timeout {
                let status = LongRunningCommandStatus {
                    action: "run_bash".to_string(),
                    command: clean.to_string(),
                    pid: record.pid,
                    elapsed: started.elapsed(),
                    timeout_ms: Some(timeout_ms),
                };
                runtime.on_long_running_command(&status);
                let _ = self.append(&record);
                let (stdout, stderr) = read_shell_job_streams(&record);
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: None,
                    signal: None,
                    output: combined_shell_output(&stdout, &stderr),
                    stdout,
                    stderr,
                    error: Some(format!(
                        "long_running_still_running:{}:{}",
                        record.pid,
                        status.elapsed.as_millis()
                    )),
                    tail_out,
                };
            }
            if started.elapsed() >= timeout {
                let _ = self.append(&record);
                let (stdout, stderr) = read_shell_job_streams(&record);
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: None,
                    signal: None,
                    output: combined_shell_output(&stdout, &stderr),
                    stdout,
                    stderr,
                    error: Some(format!("timeout_still_running:{}", record.pid)),
                    tail_out,
                };
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Terminates unfinished shell jobs launched by this process.
    ///
    /// Historical records without the current process-unique owner identity
    /// are ignored, including records from a process whose PID was later reused.
    pub fn terminate_owned_running(&self) -> usize {
        let owner_id = crate::runtime_process_owner_id();
        let records = self.records_unlocked();
        let mut terminated = 0;
        for record in records {
            if record.owner_id.as_deref() != Some(owner_id) || self.record_finished(&record) {
                continue;
            }
            terminate_process(record.pid);
            write_status_if_empty(Path::new(&record.status_file), "cancelled");
            terminated += 1;
        }
        terminated
    }

    pub fn cancel_unfinished_for_session(&self, session_id: &str) -> Vec<String> {
        let clean_session = session_id.trim();
        if clean_session.is_empty() {
            return Vec::new();
        }
        let records = self.records_unlocked();
        let owner_id = crate::runtime_process_owner_id();
        let mut cancelled = Vec::new();
        for record in records {
            if record.owner_id.as_deref() != Some(owner_id)
                || record.session_id != clean_session
                || self.record_finished(&record)
            {
                continue;
            }
            terminate_process(record.pid);
            write_status_if_empty(Path::new(&record.status_file), "cancelled");
            cancelled.push(record.id);
        }
        cancelled
    }

    pub fn running_for_session(&self, session_id: &str) -> Vec<RunningShellJob> {
        let (running, _) = self.refresh_for_session(session_id);
        running
    }

    pub fn refresh_for_session(
        &self,
        session_id: &str,
    ) -> (Vec<RunningShellJob>, Vec<ShellJobExitUpdate>) {
        let clean_session = session_id.trim();
        if clean_session.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let mut running = Vec::new();
        let mut exited = Vec::new();
        let owner_id = crate::runtime_process_owner_id();
        for record in self.records_unlocked().into_iter().filter(|record| {
            record.owner_id.as_deref() == Some(owner_id) && record.session_id == clean_session
        }) {
            match self.refresh_record_unlocked(record) {
                ShellJobRefresh::Running(job) => running.push(job),
                ShellJobRefresh::Exited(update) => exited.push(update),
                ShellJobRefresh::Finished => {}
            }
        }
        (running, exited)
    }

    pub fn running_job_list_context(&self, session_id: &str) -> Option<String> {
        let jobs = self.running_for_session(session_id);
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
        out.push_str(
            "\nContinue the task by deciding whether to wait, inspect, terminate, or take another appropriate action. Do not ask the user merely because a command is still running.",
        );
        Some(out)
    }

    fn append(&self, record: &ShellJobRecord) -> std::io::Result<()> {
        self.guard
            .with_write(|| self.append_unlocked(record))
            .map_err(std::io::Error::other)?
    }

    fn append_unlocked(&self, record: &ShellJobRecord) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.index_file)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(record).unwrap_or_default()
        )
    }

    fn records_unlocked(&self) -> Vec<ShellJobRecord> {
        let Ok(file) = OpenOptions::new().read(true).open(&self.index_file) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<ShellJobRecord>(&line) else {
                continue;
            };
            records.push(record);
        }
        records
    }

    fn record_finished(&self, record: &ShellJobRecord) -> bool {
        fs::read_to_string(&record.status_file)
            .ok()
            .map(|text| !text.trim().is_empty())
            .unwrap_or(false)
    }

    fn refresh_record_unlocked(&self, record: ShellJobRecord) -> ShellJobRefresh {
        if self.record_finished(&record) {
            return self.exit_update_once_unlocked(record);
        }
        if self.watcher.is_watching(record.pid) {
            self.watcher.refresh_pid(record.pid);
            if self.record_finished(&record) {
                return self.exit_update_once_unlocked(record);
            }
            return ShellJobRefresh::Running(RunningShellJob {
                pid: record.pid,
                tool_call_id: record.tool_call_id.clone(),
                kind: record.kind,
                command: record.command,
                cwd: record.cwd,
                session_id: record.session_id,
                turn_id: record.turn_id,
                created_at_ms: record.created_at_ms,
            });
        }
        if !process_running(record.pid) {
            write_status_if_empty(Path::new(&record.status_file), "exited");
            return self.exit_update_once_unlocked(record);
        }
        let identity_matches = record.process_identity.as_deref().is_some_and(|expected| {
            crate::os::process_identity(record.pid).as_deref() == Some(expected)
        });
        if !identity_matches {
            write_status_if_empty(Path::new(&record.status_file), "pid_identity_changed");
            return self.exit_update_once_unlocked(record);
        }
        ShellJobRefresh::Running(RunningShellJob {
            pid: record.pid,
            tool_call_id: record.tool_call_id.clone(),
            kind: record.kind,
            command: record.command,
            cwd: record.cwd,
            session_id: record.session_id,
            turn_id: record.turn_id,
            created_at_ms: record.created_at_ms,
        })
    }

    fn exit_update_once_unlocked(&self, record: ShellJobRecord) -> ShellJobRefresh {
        let notified_file = format!("{}.notified", record.status_file);
        let claimed = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&notified_file)
            .and_then(|mut file| write!(file, "{}", now_ms()));
        if claimed.is_err() {
            return ShellJobRefresh::Finished;
        }
        let (stdout, stderr) = read_shell_job_streams(&record);
        let output = normalized_shell_output(&combined_shell_output(&stdout, &stderr));
        ShellJobRefresh::Exited(ShellJobExitUpdate {
            pid: record.pid,
            tool_call_id: record.tool_call_id,
            kind: record.kind,
            command: record.command,
            cwd: record.cwd,
            session_id: record.session_id,
            turn_id: record.turn_id,
            created_at_ms: record.created_at_ms,
            elapsed_ms: now_ms().saturating_sub(record.created_at_ms),
            status: fs::read_to_string(&record.status_file)
                .unwrap_or_else(|_| "unknown".to_string())
                .trim()
                .to_string(),
            stdout,
            stderr,
            output,
        })
    }
}

enum ShellJobRefresh {
    Running(RunningShellJob),
    Exited(ShellJobExitUpdate),
    Finished,
}

pub fn validate_bash_request(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command_required".to_string());
    }
    validate_bash_safety(trimmed)?;
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

fn validate_bash_lifecycle(command: &str, background: bool) -> Result<(), String> {
    if !background && contains_unmanaged_shell_background(command) && !contains_shell_wait(command)
    {
        return Err("unmanaged_background_process".to_string());
    }
    if contains_explicit_process_detach(command) {
        return Err("explicit_process_detach".to_string());
    }
    Ok(())
}

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

fn contains_explicit_process_detach(command: &str) -> bool {
    let words = shell_words_for_safety_scan(command);
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
        if matches!(
            words[executable].as_str(),
            "setsid" | "disown" | "daemon" | "daemonize" | "start-stop-daemon"
        ) {
            return true;
        }
        index = executable + 1;
    }
    false
}

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
            ' ' | '\t' | '\n' if !in_single && !in_double => {
                push_shell_word(&mut words, &mut current);
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
    matches!(word, ";" | "then" | "do" | "else")
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
        action.input_i64("loop_timeout_ms").unwrap_or(600_000)
    };
    let session_id = core.current_session_id();
    let turn_id = core.current_action_turn_id();
    let cwd = core.current_prompt_cwd().to_path_buf();
    let tail_out = action.input_bool("tail_out");
    let tool_call_id = action.call_id.as_str();
    execute_run_bash_with_tail(
        &command_to_run,
        &cwd,
        action.background(),
        timeout_ms,
        interval_ms,
        action.input_u64("once_timeout_ms").unwrap_or(5000),
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

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_run_bash(
    command: &str,
    cwd: &Path,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    approval_mode: BashApprovalMode,
    shell_jobs: &FileShellJobStore,
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
    approval_mode: BashApprovalMode,
    shell_jobs: &FileShellJobStore,
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
    if let Err(reason) = validate_bash_request(command_to_run) {
        let message = bash_validation_message(&reason);
        return ActionExecution::Completed(bash_finished_error_outcome(
            bash_action_not_executed(Some(command_to_run), message),
            "InvalidInput",
            message,
        ));
    }
    if let Err(reason) = validate_bash_lifecycle(command_to_run, background) {
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
    if !background && is_regular_command && contains_long_normal_sleep(command_to_run) {
        let reason = "The command contains a long sleep in normal mode. Use loop_cmd with interval_ms to poll external status, or background=true for long local work that should continue across turns.";
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
                action: "run_bash".to_string(),
                command: command_to_run.to_string(),
                reason: "run_bash_requires_user_approval".to_string(),
                risk: "local_command_execution".to_string(),
            },
            approved_action: PendingApprovedAction::RunBash {
                command: command_to_run.to_string(),
                background,
                timeout_ms,
                interval_ms,
                once_timeout_ms,
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
    if background {
        return ActionExecution::Completed(shell_jobs.spawn_background_outcome(
            command_to_run,
            cwd,
            session_id,
            turn_id,
            tool_call_id,
            tail_out,
        ));
    }
    if let Some(interval_ms) = interval_ms {
        return ActionExecution::Completed(execute_polling_bash_outcome_with_tail(
            command_to_run,
            cwd,
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            tail_out,
            runtime,
        ));
    }
    ActionExecution::Completed(shell_jobs.run_with_timeout_outcome(
        command_to_run,
        cwd,
        timeout_ms,
        session_id,
        turn_id,
        tool_call_id,
        tail_out,
        runtime,
    ))
}

#[cfg(test)]
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
    shell_jobs: &FileShellJobStore,
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
    request: &ApprovalRequest,
    shell_jobs: &FileShellJobStore,
    runtime: &mut dyn ActionRuntime,
) -> ActionOutcome {
    let clean = command.trim();
    if let Err(reason) = validate_bash_request(clean) {
        let message = bash_validation_message(&reason);
        let mut outcome = bash_finished_error_outcome(
            bash_action_not_executed(Some(clean), message),
            "InvalidInput",
            message,
        );
        outcome.text.push_str(&format!(
            "\napproval_id: {}\napproval_status: approved_by_user",
            request.approval_id
        ));
        return outcome;
    }
    if let Err(reason) = validate_bash_lifecycle(clean, background) {
        let message = bash_validation_message(&reason);
        let mut outcome = bash_finished_error_outcome(
            bash_action_not_executed(Some(clean), message),
            "InvalidInput",
            message,
        );
        outcome.text.push_str(&format!(
            "\napproval_id: {}\napproval_status: approved_by_user",
            request.approval_id
        ));
        return outcome;
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
    outcome.text.push_str(&format!(
        "\napproval_id: {}\napproval_status: approved_by_user",
        request.approval_id
    ));
    outcome
}

pub fn execute_one_bash(command: &str, timeout_ms: i64, runtime: &mut dyn ActionRuntime) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    execute_one_bash_structured(command, &cwd, timeout_ms, runtime).to_action_result("run_bash")
}

#[cfg(test)]
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
        _ => "The polling command stopped.",
    };
    let mut out = format!(
        "Action result: run_bash\n{state_sentence}\nPolling state: {}\nAttempts: {}\nElapsed: {} ms\nSuccess condition: exit code 0",
        state,
        attempts,
        elapsed.as_millis()
    );
    if let Some(status) = last_status {
        out.push_str(&format!("\nLast observed exit code: {status}"));
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
        "cancelled" => ActionStatus::Cancelled,
        _ => ActionStatus::Failed,
    };
    ActionOutcome::new(status, out).with_bash_result(BashResultEvidence {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        exit_code: last_status,
        signal: last_signal,
        pid: None,
        timed_out: false,
        pid_kind: None,
        error_type: match state {
            "cancelled" => Some("Cancelled".to_string()),
            "not_executed" => Some("InvalidInput".to_string()),
            _ => None,
        },
    })
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

fn contains_long_normal_sleep(command: &str) -> bool {
    let tokens = shell_words_for_sleep_scan(command);
    tokens.windows(2).any(|pair| {
        pair[0] == "sleep" && sleep_arg_seconds(&pair[1]).is_some_and(|seconds| seconds >= 30.0)
    })
}

fn shell_words_for_sleep_scan(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in command.chars() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' if !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
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
    pub output: String,
    pub error: Option<String>,
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
                    "Action result: {}\nLONG_RUNNING_COMMAND_STATUS:\nPID: {}\nElapsed: {} ms\nStatus: still running\nThe command has not finished. Continue the task by deciding whether to wait, inspect, terminate, or take another appropriate action. Do not ask the user merely because this command is still running.",
                    action_name, pid, elapsed_ms
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
    let mut shell = Command::new(crate::os::BASH_EXECUTABLE);
    configure_run_bash_environment(&mut shell);
    shell
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::os::configure_child_process_group(&mut shell);
    let spawn = shell.spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(_) => return bash_error(command, "command_failed"),
    };
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms as u64);
    let mut next_long_running_check = long_running_prompt_after;
    loop {
        if runtime.should_cancel() {
            terminate_process(child.id());
            let _ = child.wait();
            return bash_error(command, "cancelled");
        }
        if started.elapsed() >= next_long_running_check && started.elapsed() < timeout {
            let status = LongRunningCommandStatus {
                action: "run_bash".to_string(),
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
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_process(child.id());
                let _ = child.wait();
                return bash_error(command, "timeout");
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return bash_error(command, "command_failed"),
        }
    }
    match child.wait_with_output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let combined = combined_shell_output(&stdout, &stderr);
            BashCommandOutput {
                command: command.to_string(),
                status: output.status.code(),
                signal: exit_signal(&output.status),
                stdout,
                stderr,
                output: combined,
                error: None,
                tail_out: false,
            }
        }
        Err(_) => bash_error(command, "command_failed"),
    }
}

fn read_shell_job_streams(record: &ShellJobRecord) -> (String, String) {
    let stdout = fs::read_to_string(&record.output_file).unwrap_or_default();
    let stderr = if record.stderr_file.trim().is_empty() {
        // Historical records merged both streams into output_file. Treat that
        // file as stdout and never guess which lines originally came from stderr.
        String::new()
    } else {
        fs::read_to_string(&record.stderr_file).unwrap_or_default()
    };
    (stdout, stderr)
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
        output: diagnostic,
        error: Some(error.to_string()),
        tail_out: false,
    }
}

fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    crate::os::exit_signal(status)
}

fn bash_action_not_executed(_command: Option<&str>, reason: &str) -> String {
    format!("Action result: run_bash\nThe command was not executed.\nReason: {reason}")
}

fn bash_validation_message(reason: &str) -> &'static str {
    match reason {
        "command_required" => "No shell command was provided.",
        "dangerous_recursive_root_delete" => {
            "The shell command was blocked by Timem safety policy because it may recursively delete the filesystem root."
        }
        "unmanaged_background_process" => {
            "检测到命令可能创建脱离 Runtime 管理的后台进程。请改用 run_bash(background=true)。"
        }
        "explicit_process_detach" => {
            "检测到命令可能创建脱离 Runtime 管理的后台进程。请改用 run_bash(background=true)，并移除 setsid、disown 或 daemon 等主动脱离方式。"
        }
        _ => "The shell command request did not pass runtime validation.",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessStatus {
    code: Option<i32>,
    signal: Option<i32>,
}

fn read_process_status(status_file: &str) -> Option<ProcessStatus> {
    let text = fs::read_to_string(status_file)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())?;
    if let Some(signal) = text.strip_prefix("signal:") {
        return signal.parse::<i32>().ok().map(|signal| ProcessStatus {
            code: None,
            signal: Some(signal),
        });
    }
    text.parse::<i32>().ok().map(|code| ProcessStatus {
        code: Some(code),
        signal: None,
    })
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

fn process_running(pid: u32) -> bool {
    crate::os::child_process_running(pid)
}

#[cfg(test)]
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

fn unique_shell_id(prefix: &str) -> String {
    let seq = SHELL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, now_ms(), seq)
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_run_bash_tests.rs"]
mod tests;

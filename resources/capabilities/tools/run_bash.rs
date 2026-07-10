use crate::response_protocol::ParsedAction;
use crate::MemGuard;
use crate::{
    ActionExecution, ActionRuntime, AgentCore, ApprovalRequest, BashApprovalMode,
    LongRunningCommandDecision, LongRunningCommandStatus, PendingApproval, PendingApprovedAction,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::MutexGuard;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static LONG_RUNNING_COMMAND_PROMPT_AFTER_MS: AtomicU64 = AtomicU64::new(60_000);
#[cfg(test)]
static LONG_RUNNING_COMMAND_PROMPT_AFTER_LOCK: Mutex<()> = Mutex::new(());

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
    pub command: String,
    #[serde(default)]
    pub cwd: String,
    pub output_file: String,
    pub status_file: String,
}

fn default_shell_job_kind() -> String {
    "background".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningShellJob {
    pub pid: u32,
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
    pub kind: String,
    pub command: String,
    pub cwd: String,
    pub session_id: String,
    pub turn_id: String,
    pub created_at_ms: i64,
    pub elapsed_ms: i64,
    pub status: String,
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
    children: Arc<Mutex<HashMap<u32, Child>>>,
}

impl FileShellJobStore {
    pub fn new(memory_dir: &Path) -> Self {
        let dir = memory_dir.join("shell_jobs");
        let _ = fs::create_dir_all(&dir);
        Self {
            index_file: dir.join("jobs.jsonl"),
            dir,
            guard: MemGuard::for_memory_dir(memory_dir),
            children: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn spawn_background(
        &self,
        command: &str,
        cwd: &Path,
        session_id: &str,
        turn_id: &str,
    ) -> String {
        let clean = command.trim();
        if clean.is_empty() {
            return bash_action_not_executed(
                None,
                "The background command was not started because no shell command was provided.",
            );
        }
        let record = match self.spawn_record(clean, cwd, "background", session_id, turn_id) {
            Ok(record) => record,
            Err(_) => {
                return bash_action_not_executed(
                    Some(clean),
                    "The background command could not be started by the local shell.",
                )
            }
        };
        let _ = self.append(&record);
        format!(
            "Action result: run_bash\npid={}, now keeps running in background\nCommand: {}",
            record.pid, clean
        )
    }

    fn spawn_record(
        &self,
        clean: &str,
        cwd: &Path,
        kind: &str,
        session_id: &str,
        turn_id: &str,
    ) -> std::io::Result<ShellJobRecord> {
        fs::create_dir_all(&self.dir)?;
        let id = unique_shell_id("job");
        let output_file = self.dir.join(format!("{id}.out"));
        let status_file = self.dir.join(format!("{id}.status"));
        let script = format!(
            "({}) > {} 2>&1; printf '%s' \"$?\" > {}",
            clean,
            shell_quote_path(&output_file),
            shell_quote_path(&status_file)
        );
        let mut command = Command::new("/bin/sh");
        command
            .arg("-lc")
            .arg(script)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let child = command.spawn()?;
        let pid = child.id();
        if let Ok(mut children) = self.children.lock() {
            children.insert(pid, child);
        }
        Ok(ShellJobRecord {
            id,
            created_at_ms: now_ms(),
            kind: kind.to_string(),
            session_id: session_id.trim().to_string(),
            turn_id: turn_id.trim().to_string(),
            pid,
            command: clean.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            output_file: output_file.to_string_lossy().to_string(),
            status_file: status_file.to_string_lossy().to_string(),
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
        let result = self
            .run_with_timeout_structured(command, cwd, timeout_ms, session_id, turn_id, runtime);
        result.to_action_result("run_bash")
    }

    pub fn run_with_timeout_structured(
        &self,
        command: &str,
        cwd: &Path,
        timeout_ms: i64,
        session_id: &str,
        turn_id: &str,
        runtime: &mut dyn ActionRuntime,
    ) -> BashCommandOutput {
        let clean = command.trim();
        if timeout_ms <= 0 {
            return bash_error(clean, "invalid_timeout");
        }
        let Ok(record) = self.spawn_record(clean, cwd, "timeout", session_id, turn_id) else {
            return bash_error(clean, "command_failed");
        };
        let started = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);
        let mut next_long_running_check = long_running_command_prompt_after();
        loop {
            if runtime.should_cancel() {
                terminate_process(record.pid);
                let _ = fs::write(&record.status_file, "cancelled");
                return bash_error(clean, "cancelled");
            }
            if started.elapsed() >= next_long_running_check && started.elapsed() < timeout {
                let status = LongRunningCommandStatus {
                    action: "run_bash".to_string(),
                    command: clean.to_string(),
                    elapsed: started.elapsed(),
                    timeout_ms: Some(timeout_ms),
                };
                if runtime.on_long_running_command(&status) == LongRunningCommandDecision::Cancel {
                    terminate_process(record.pid);
                    let _ = fs::write(&record.status_file, "cancelled");
                    return bash_error(clean, "cancelled_by_user");
                }
                next_long_running_check =
                    next_long_running_check.saturating_add(long_running_command_prompt_after());
            }
            if let Some(code) = read_status_code(&record.status_file) {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: Some(code),
                    output: normalized_shell_output(&output),
                    error: None,
                };
            }
            if started.elapsed() >= timeout {
                let _ = self.append(&record);
                let partial = fs::read_to_string(&record.output_file).unwrap_or_default();
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: None,
                    output: compact_text(&partial, 2000),
                    error: Some(format!("timeout_still_running:{}", record.pid)),
                };
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn cancel_unfinished_for_session(&self, session_id: &str) -> Vec<String> {
        let clean_session = session_id.trim();
        if clean_session.is_empty() {
            return Vec::new();
        }
        let records = self
            .guard
            .with_read(|| self.records_unlocked())
            .unwrap_or_default();
        let mut cancelled = Vec::new();
        for record in records {
            if record.session_id != clean_session || self.record_finished(&record) {
                continue;
            }
            terminate_process(record.pid);
            let _ = fs::write(&record.status_file, "cancelled");
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
        self.guard
            .with_write(|| {
                let mut running = Vec::new();
                let mut exited = Vec::new();
                for record in self
                    .records_unlocked()
                    .into_iter()
                    .filter(|record| record.session_id == clean_session)
                {
                    match self.refresh_record_unlocked(record) {
                        ShellJobRefresh::Running(job) => running.push(job),
                        ShellJobRefresh::Exited(update) => exited.push(update),
                        ShellJobRefresh::Finished => {}
                    }
                }
                (running, exited)
            })
            .unwrap_or_default()
    }

    pub fn running_job_list_context(&self, session_id: &str) -> Option<String> {
        let jobs = self.running_for_session(session_id);
        if jobs.is_empty() {
            return None;
        }
        let mut out = String::from("RUNNING JOB LIST:");
        for job in jobs {
            out.push_str(&format!(
                "\npid={}, {}, cwd={}, cmd={}, still running",
                job.pid,
                job.description(),
                job.cwd,
                compact_text(&job.command, 500)
            ));
        }
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
        if let Some(refresh) = self.refresh_child_handle_unlocked(&record) {
            return refresh;
        }
        if self.record_finished(&record) {
            return self.exit_update_once_unlocked(record);
        }
        if !process_running(record.pid) {
            let _ = fs::write(&record.status_file, "exited");
            return self.exit_update_once_unlocked(record);
        }
        ShellJobRefresh::Running(RunningShellJob {
            pid: record.pid,
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
        if Path::new(&notified_file).exists() {
            return ShellJobRefresh::Finished;
        }
        let _ = fs::write(&notified_file, now_ms().to_string());
        ShellJobRefresh::Exited(ShellJobExitUpdate {
            pid: record.pid,
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
            output: compact_text(
                &normalized_shell_output(
                    &fs::read_to_string(&record.output_file).unwrap_or_default(),
                ),
                4000,
            ),
        })
    }

    fn refresh_child_handle_unlocked(&self, record: &ShellJobRecord) -> Option<ShellJobRefresh> {
        let mut children = self.children.lock().ok()?;
        let child = children.get_mut(&record.pid)?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1).to_string();
                let _ = fs::write(&record.status_file, code);
                children.remove(&record.pid);
                Some(self.exit_update_once_unlocked(record.clone()))
            }
            Ok(None) => Some(ShellJobRefresh::Running(RunningShellJob {
                pid: record.pid,
                kind: record.kind.clone(),
                command: record.command.clone(),
                cwd: record.cwd.clone(),
                session_id: record.session_id.clone(),
                turn_id: record.turn_id.clone(),
                created_at_ms: record.created_at_ms,
            })),
            Err(_) => {
                children.remove(&record.pid);
                None
            }
        }
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
    Ok(())
}

pub(crate) fn execute_run_bash_action(
    core: &mut AgentCore,
    action: &ParsedAction,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let loop_command = action.input_str("loop_cmd");
    if !loop_command.is_empty() && !action.input_str("cmd").is_empty() {
        return ActionExecution::Completed(bash_action_not_executed(
            None,
            "The action provided both cmd and loop_cmd. Use cmd for a normal/background command, or loop_cmd with interval_ms for polling.",
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
    execute_run_bash(
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
        is_regular_command,
        runtime,
    )
}

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
    let command_to_run = command.trim();
    if command_to_run.is_empty() {
        return ActionExecution::Completed(bash_action_not_executed(
            None,
            "The command was not executed because no shell command was provided.",
        ));
    }
    if let Err(reason) = validate_bash_request(command_to_run) {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            bash_validation_message(&reason),
        ));
    }
    if !background && is_regular_command && timeout_ms <= 0 {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            "timeout_ms must be a positive integer. Choose a wait budget that matches the command.",
        ));
    }
    if !background && !is_regular_command && timeout_ms <= 0 {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            "loop_timeout_ms must be a positive integer. Choose a total polling wait budget that matches the external state you are waiting for.",
        ));
    }
    if !background && is_regular_command && contains_long_normal_sleep(command_to_run) {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            "The command contains a long sleep in normal mode. Use loop_cmd with interval_ms to poll external status, or background=true for long local work that should continue across turns.",
        ));
    }
    if background && interval_ms.is_some() {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            "Polling mode and background mode cannot be combined. Use loop_cmd with interval_ms for polling, or background=true for a persistent background command.",
        ));
    }
    if interval_ms.is_some() && is_regular_command {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            "interval_ms is only valid with loop_cmd. Move the check command to loop_cmd, or remove interval_ms for a normal command.",
        ));
    }
    if interval_ms.is_none() && !is_regular_command {
        return ActionExecution::Completed(bash_action_not_executed(
            Some(command_to_run),
            "loop_cmd needs interval_ms so the runtime knows how often to check the condition.",
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
                cwd: cwd.to_path_buf(),
            },
            continuation: None,
        });
    }
    if background {
        return ActionExecution::Completed(shell_jobs.spawn_background(
            command_to_run,
            cwd,
            session_id,
            turn_id,
        ));
    }
    if let Some(interval_ms) = interval_ms {
        return ActionExecution::Completed(execute_polling_bash(
            command_to_run,
            cwd,
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            runtime,
        ));
    }
    ActionExecution::Completed(shell_jobs.run_with_timeout(
        command_to_run,
        cwd,
        timeout_ms,
        session_id,
        turn_id,
        runtime,
    ))
}

pub(crate) fn execute_approved_bash(
    command: &str,
    cwd: &Path,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    session_id: &str,
    turn_id: &str,
    _is_regular_command: bool,
    request: &ApprovalRequest,
    shell_jobs: &FileShellJobStore,
    runtime: &mut dyn ActionRuntime,
) -> String {
    let mut result = if background {
        shell_jobs.spawn_background(command.trim(), cwd, session_id, turn_id)
    } else if let Some(interval_ms) = interval_ms {
        execute_polling_bash(
            command.trim(),
            cwd,
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            runtime,
        )
    } else {
        shell_jobs.run_with_timeout(
            command.trim(),
            cwd,
            timeout_ms,
            session_id,
            turn_id,
            runtime,
        )
    };
    result.push_str(&format!(
        "\napproval_id: {}\napproval_status: approved_by_user",
        request.approval_id
    ));
    result
}

pub fn execute_one_bash(command: &str, timeout_ms: i64, runtime: &mut dyn ActionRuntime) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    execute_one_bash_structured(command, &cwd, timeout_ms, runtime).to_action_result("run_bash")
}

pub(crate) fn execute_polling_bash(
    command: &str,
    cwd: &Path,
    interval_ms: u64,
    timeout_ms: i64,
    once_timeout_ms: u64,
    runtime: &mut dyn ActionRuntime,
) -> String {
    if timeout_ms <= 0 {
        return polling_result(
            command,
            "not_executed",
            0,
            Duration::ZERO,
            None,
            "",
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
            "",
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
            "",
            Some("once_timeout_ms must be a positive integer."),
        );
    }
    let interval = Duration::from_millis(interval_ms);
    let max_wait = Duration::from_millis(timeout_ms as u64);
    let started = Instant::now();
    let mut attempts = 0_u64;
    let mut last_status = None;
    let mut last_output = String::new();
    let mut last_error = None;

    loop {
        if runtime.should_cancel() {
            return polling_result(
                command,
                "cancelled",
                attempts,
                started.elapsed(),
                last_status,
                &last_output,
                last_error.as_deref(),
            );
        }

        attempts = attempts.saturating_add(1);
        let result = execute_one_bash_structured(command, cwd, once_timeout_ms as i64, runtime);
        last_status = result.status;
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
                    &last_output,
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
                &last_output,
                last_error.as_deref(),
            );
        }

        let wait = interval.min(max_wait.saturating_sub(started.elapsed()));
        sleep_cancelable(wait, &mut || runtime.should_cancel());
    }
}

fn polling_result(
    command: &str,
    state: &str,
    attempts: u64,
    elapsed: Duration,
    last_status: Option<i32>,
    output: &str,
    error: Option<&str>,
) -> String {
    let state_sentence = match state {
        "finished" => "The polling command finished because the check command exited with code 0.",
        "timeout" => "The polling command stopped because the total wait budget was reached before the check command exited with code 0.",
        "cancelled" => "The polling command was cancelled before the check command exited with code 0.",
        _ => "The polling command stopped.",
    };
    let mut out = format!(
        "Action result: run_bash\n{state_sentence}\nCommand: {}\nPolling state: {}\nAttempts: {}\nElapsed: {} ms\nSuccess condition: exit code 0",
        command,
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
        out.push_str(&compact_text(output, 4000));
    }
    out
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
    pub output: String,
    pub error: Option<String>,
}

impl BashCommandOutput {
    pub fn to_action_result(&self, action_name: &str) -> String {
        if let Some(error) = &self.error {
            if let Some(pid) = error.strip_prefix("timeout_still_running:") {
                let mut out = format!(
                    "Action result: {}\npid={}, timeout, but is still running\nTimeout means Timem stopped waiting; the process was not killed and there is no final exit code yet.\nCommand: {}",
                    action_name, pid, self.command
                );
                if !self.output.trim().is_empty() {
                    out.push_str("\nPartial output:\n");
                    out.push_str(&compact_text(&self.output, 2000));
                }
                return out;
            }
            return format!(
                "Action result: {}\nCommand: {}\n{}",
                action_name,
                self.command,
                bash_runtime_error_message(error)
            );
        }
        format!(
            "Action result: {}\nThe command finished.\nCommand: {}\nExit code: {}\nOutput:\n{}",
            action_name,
            self.command,
            self.status.unwrap_or(-1),
            compact_text(&self.output, 4000)
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
        long_running_command_prompt_after(),
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
    let spawn = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(_) => return bash_error(command, "command_failed"),
    };
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms as u64);
    let mut next_long_running_check = long_running_prompt_after;
    loop {
        if runtime.should_cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return bash_error(command, "cancelled");
        }
        if started.elapsed() >= next_long_running_check && started.elapsed() < timeout {
            let status = LongRunningCommandStatus {
                action: "run_bash".to_string(),
                command: command.to_string(),
                elapsed: started.elapsed(),
                timeout_ms: Some(timeout_ms),
            };
            if runtime.on_long_running_command(&status) == LongRunningCommandDecision::Cancel {
                let _ = child.kill();
                let _ = child.wait();
                return bash_error(command, "cancelled_by_user");
            }
            next_long_running_check =
                next_long_running_check.saturating_add(long_running_prompt_after);
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return bash_error(command, "timeout");
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => return bash_error(command, "command_failed"),
        }
    }
    match child.wait_with_output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
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
                combined = "<no output>".to_string();
            }
            BashCommandOutput {
                command: command.to_string(),
                status: Some(output.status.code().unwrap_or(-1)),
                output: combined,
                error: None,
            }
        }
        Err(_) => bash_error(command, "command_failed"),
    }
}

fn long_running_command_prompt_after() -> Duration {
    #[cfg(test)]
    {
        return Duration::from_millis(
            LONG_RUNNING_COMMAND_PROMPT_AFTER_MS
                .load(Ordering::Relaxed)
                .max(1),
        );
    }
    #[cfg(not(test))]
    {
        Duration::from_secs(60)
    }
}

#[cfg(test)]
pub(crate) struct LongRunningPromptAfterGuard {
    previous_ms: u64,
    _lock: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for LongRunningPromptAfterGuard {
    fn drop(&mut self) {
        LONG_RUNNING_COMMAND_PROMPT_AFTER_MS.store(self.previous_ms, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) fn set_long_running_command_prompt_after_for_tests(
    duration: Duration,
) -> LongRunningPromptAfterGuard {
    let lock = LONG_RUNNING_COMMAND_PROMPT_AFTER_LOCK
        .lock()
        .expect("long running prompt threshold test lock should not be poisoned");
    let previous_ms = LONG_RUNNING_COMMAND_PROMPT_AFTER_MS
        .swap(duration.as_millis().max(1) as u64, Ordering::Relaxed);
    LongRunningPromptAfterGuard {
        previous_ms,
        _lock: lock,
    }
}

fn bash_error(command: &str, error: &str) -> BashCommandOutput {
    BashCommandOutput {
        command: command.to_string(),
        status: None,
        output: String::new(),
        error: Some(error.to_string()),
    }
}

fn bash_action_not_executed(command: Option<&str>, reason: &str) -> String {
    let mut out = String::from("Action result: run_bash\nThe command was not executed.\n");
    if let Some(command) = command.map(str::trim).filter(|command| !command.is_empty()) {
        out.push_str("Command: ");
        out.push_str(command);
        out.push('\n');
    }
    out.push_str("Reason: ");
    out.push_str(reason);
    out
}

fn bash_validation_message(reason: &str) -> &'static str {
    match reason {
        "command_required" => "No shell command was provided.",
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

fn read_status_code(status_file: &str) -> Option<i32> {
    fs::read_to_string(status_file)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .and_then(|text| text.parse::<i32>().ok())
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
    #[cfg(unix)]
    {
        let mut status = 0;
        let wait = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if wait == pid as libc::pid_t {
            return false;
        }
        if wait == 0 {
            return true;
        }
        if let Ok(output) = Command::new("/bin/ps")
            .arg("-o")
            .arg("stat=")
            .arg("-p")
            .arg(pid.to_string())
            .output()
        {
            if output.status.success() {
                let stat = String::from_utf8_lossy(&output.stdout);
                let state = stat.trim();
                if state.starts_with('Z') || state.contains('Z') {
                    return false;
                }
                return !state.is_empty();
            }
        }
    }
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .is_some_and(|status| status.success())
}

fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn terminate_process(pid: u32) {
    #[cfg(unix)]
    {
        let group = format!("-{}", pid);
        let status = Command::new("/bin/kill")
            .arg("-TERM")
            .arg(&group)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.as_ref().is_ok_and(|s| s.success()) {
            thread::sleep(Duration::from_millis(100));
            if process_running(pid) {
                let _ = Command::new("/bin/kill")
                    .arg("-KILL")
                    .arg(&group)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            return;
        }
    }
    let status = Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if status.as_ref().is_ok_and(|s| s.success()) {
        thread::sleep(Duration::from_millis(100));
        if process_running(pid) {
            let _ = Command::new("/bin/kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct NeverCancelRuntime;

    impl ActionRuntime for NeverCancelRuntime {
        fn should_cancel(&mut self) -> bool {
            false
        }
    }

    struct ToggleCancelRuntime<'a> {
        cancelled: &'a AtomicBool,
    }

    impl ActionRuntime for ToggleCancelRuntime<'_> {
        fn should_cancel(&mut self) -> bool {
            let previous = self.cancelled.swap(true, Ordering::Relaxed);
            previous
        }
    }

    #[derive(Default)]
    struct CancelAfterLongRunningPromptRuntime {
        prompts: Vec<LongRunningCommandStatus>,
    }

    impl ActionRuntime for CancelAfterLongRunningPromptRuntime {
        fn should_cancel(&mut self) -> bool {
            false
        }

        fn on_long_running_command(
            &mut self,
            status: &LongRunningCommandStatus,
        ) -> LongRunningCommandDecision {
            self.prompts.push(status.clone());
            LongRunningCommandDecision::Cancel
        }
    }

    fn tmp_memory_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "timem_shell_exec_test_{}_{}",
            name,
            unique_shell_id("case")
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tmp_cwd(name: &str) -> PathBuf {
        tmp_memory_dir(&format!("cwd_{name}"))
    }

    #[test]
    fn normal_bash_reports_status_and_output() {
        let mut runtime = NeverCancelRuntime;
        let result = execute_one_bash("printf shell_ok", 1000, &mut runtime);
        assert!(result.contains("Action result: run_bash"));
        assert!(result.contains("Exit code: 0"));
        assert!(result.contains("shell_ok"));
    }

    #[test]
    fn normal_bash_timeout_is_bounded() {
        let mut runtime = NeverCancelRuntime;
        let result = execute_one_bash("sleep 2", 1000, &mut runtime);
        assert!(result.contains("Timem stopped waiting"), "{result}");
        assert!(
            result.contains("does not by itself mean the process was killed"),
            "{result}"
        );
    }

    #[test]
    fn normal_bash_rejects_non_positive_timeout() {
        let mut runtime = NeverCancelRuntime;
        let marker = tmp_memory_dir("invalid_timeout").join("marker.txt");
        let command = format!("printf should_not_run > {}", marker.display());
        let result = execute_one_bash(&command, -1, &mut runtime);
        assert!(
            result.contains("timeout_ms must be a positive integer"),
            "{result}"
        );
        assert!(!result.contains("Exit code: 0"), "{result}");
        assert!(!marker.exists(), "{result}");
    }

    #[test]
    fn normal_bash_positive_timeout_reports_long_running_status_to_runtime() {
        let _guard = set_long_running_command_prompt_after_for_tests(Duration::from_millis(50));
        let mut runtime = CancelAfterLongRunningPromptRuntime::default();
        let result = execute_one_bash("sleep 2; printf should_not_finish", 5000, &mut runtime);

        assert!(result.contains("cancelled before it completed"), "{result}");
        assert_eq!(runtime.prompts.len(), 1);
        assert_eq!(runtime.prompts[0].action, "run_bash");
        assert_eq!(
            runtime.prompts[0].command,
            "sleep 2; printf should_not_finish"
        );
        assert_eq!(runtime.prompts[0].timeout_ms, Some(5000));
        assert!(runtime.prompts[0].elapsed >= Duration::from_millis(50));
    }

    #[test]
    fn normal_run_bash_rejects_long_sleep_commands() {
        let store = FileShellJobStore::new(&tmp_memory_dir("long_sleep_guard"));
        let cwd = tmp_cwd("long_sleep_guard");
        let result = execute_run_bash(
            "sleep 90 && printf done",
            &cwd,
            false,
            5000,
            None,
            5000,
            BashApprovalMode::Approve,
            &store,
            "session_a",
            "turn_a",
            true,
            &mut NeverCancelRuntime,
        );
        match result {
            ActionExecution::Completed(text) => {
                assert!(text.contains("long sleep in normal mode"), "{text}");
                assert!(text.contains("interval_ms"));
            }
            ActionExecution::NeedsApproval(_) => {
                panic!("long sleep should be rejected before approval")
            }
        }
    }

    #[test]
    fn normal_run_bash_allows_short_sleep_commands() {
        let store = FileShellJobStore::new(&tmp_memory_dir("short_sleep_guard"));
        let cwd = tmp_cwd("short_sleep_guard");
        let result = execute_run_bash(
            "sleep 1; printf done",
            &cwd,
            false,
            3000,
            None,
            5000,
            BashApprovalMode::Approve,
            &store,
            "session_a",
            "turn_a",
            true,
            &mut NeverCancelRuntime,
        );
        match result {
            ActionExecution::Completed(text) => {
                assert!(text.contains("Exit code: 0"));
                assert!(text.contains("done"));
            }
            ActionExecution::NeedsApproval(_) => panic!("approve mode should not request approval"),
        }
    }

    #[test]
    fn run_bash_poll_mode_finishes_when_command_exits_zero() {
        let dir = tmp_memory_dir("poll_success");
        let marker = dir.join("ready.flag");
        let command = format!(
            "test -f {} || (touch {}; exit 1)",
            shell_quote_path(&marker),
            shell_quote_path(&marker)
        );
        let mut runtime = NeverCancelRuntime;
        let result = execute_polling_bash(&command, &dir, 1000, 5000, 1000, &mut runtime);
        assert!(result.contains("Action result: run_bash"), "{result}");
        assert!(result.contains("Polling state: finished"), "{result}");
        assert!(result.contains("Attempts: 2"), "{result}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn run_bash_poll_mode_times_out_when_command_stays_nonzero() {
        let mut runtime = NeverCancelRuntime;
        let cwd = tmp_cwd("poll_timeout");
        let result = execute_polling_bash(
            "printf waiting; exit 7",
            &cwd,
            1000,
            1100,
            1000,
            &mut runtime,
        );
        assert!(result.contains("Polling state: timeout"), "{result}");
        assert!(result.contains("Last observed exit code: 7"), "{result}");
        assert!(result.contains("waiting"), "{result}");
    }

    #[test]
    fn run_bash_poll_mode_can_be_cancelled_during_wait() {
        let cancelled = AtomicBool::new(false);
        let mut runtime = ToggleCancelRuntime {
            cancelled: &cancelled,
        };
        let cwd = tmp_cwd("poll_cancel");
        let result = execute_polling_bash("exit 1", &cwd, 1000, 10_000, 1000, &mut runtime);
        assert!(result.contains("Polling state: cancelled"), "{result}");
    }

    #[test]
    fn run_bash_poll_mode_requests_user_approval_in_ask_mode() {
        let store = FileShellJobStore::new(&tmp_memory_dir("poll_approval"));
        let cwd = tmp_cwd("poll_approval");
        let result = execute_run_bash(
            "test -f /tmp/timem_poll_marker",
            &cwd,
            false,
            5000,
            Some(1000),
            1000,
            BashApprovalMode::Ask,
            &store,
            "session_a",
            "turn_a",
            false,
            &mut NeverCancelRuntime,
        );
        match result {
            ActionExecution::NeedsApproval(pending) => {
                assert_eq!(pending.request.action, "run_bash");
                assert_eq!(pending.request.risk, "local_command_execution");
            }
            other => panic!("expected run_bash approval request, got {other:?}"),
        }
    }

    #[test]
    fn run_bash_polling_requires_loop_cmd_and_interval_pair() {
        let store = FileShellJobStore::new(&tmp_memory_dir("poll_pairing"));
        let cwd = tmp_cwd("poll_pairing");
        let cmd_with_interval = execute_run_bash(
            "test -f /tmp/timem_poll_marker",
            &cwd,
            false,
            5000,
            Some(1000),
            1000,
            BashApprovalMode::Approve,
            &store,
            "session_a",
            "turn_a",
            true,
            &mut NeverCancelRuntime,
        );
        match cmd_with_interval {
            ActionExecution::Completed(text) => {
                assert!(
                    text.contains("interval_ms is only valid with loop_cmd"),
                    "{text}"
                );
            }
            other => panic!("expected pairing error, got {other:?}"),
        }

        let loop_without_interval = execute_run_bash(
            "test -f /tmp/timem_poll_marker",
            &cwd,
            false,
            5000,
            None,
            1000,
            BashApprovalMode::Approve,
            &store,
            "session_a",
            "turn_a",
            false,
            &mut NeverCancelRuntime,
        );
        match loop_without_interval {
            ActionExecution::Completed(text) => {
                assert!(text.contains("loop_cmd needs interval_ms"), "{text}");
            }
            other => panic!("expected pairing error, got {other:?}"),
        }
    }

    #[test]
    fn polling_bash_waits_until_async_file_appears() {
        let dir = tmp_memory_dir("poll_async_file");
        let flag = dir.join("done.flag");
        let flag_path = shell_quote_path(&flag);
        let mut runtime = NeverCancelRuntime;
        let _ = fs::remove_file(&flag);
        let mut child = Command::new("/bin/sh")
            .arg("-lc")
            .arg(format!("sleep 0.3; touch {flag_path}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn delayed flag creator");

        let started = Instant::now();
        let result = execute_polling_bash(
            &format!("test -f {flag_path}"),
            &dir,
            100,
            2000,
            1000,
            &mut runtime,
        );
        let elapsed = started.elapsed();

        assert!(result.contains("Polling state: finished"), "{result}");
        assert!(
            result.contains("Success condition: exit code 0"),
            "{result}"
        );
        assert!(
            elapsed >= Duration::from_millis(200),
            "poll should wait for asynchronous file creation, elapsed={elapsed:?}\n{result}"
        );
        assert!(
            elapsed < Duration::from_millis(1500),
            "poll should return soon after condition succeeds, elapsed={elapsed:?}\n{result}"
        );
        let _ = child.wait();
    }

    #[test]
    fn background_job_reports_pid_and_running_list_until_exit() {
        let dir = tmp_memory_dir("background_job");
        let store = FileShellJobStore::new(&dir);
        let started =
            store.spawn_background("sleep 1; printf background_ok", &dir, "session_a", "turn_a");
        assert!(
            started.contains("now keeps running in background"),
            "{started}"
        );
        let pid = started
            .lines()
            .find_map(|line| line.strip_prefix("pid="))
            .and_then(|rest| rest.split(',').next())
            .and_then(|pid| pid.parse::<u32>().ok())
            .unwrap();
        let running = store.running_for_session("session_a");
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].pid, pid);
        assert_eq!(running[0].kind, "background");

        thread::sleep(Duration::from_millis(1300));
        let (running, updates) = store.refresh_for_session("session_a");
        assert!(running.is_empty());
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].pid, pid);
        assert_eq!(updates[0].description(), "background job");
        assert_eq!(updates[0].status, "0");
        assert_eq!(updates[0].output, "background_ok");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn timeout_job_reports_pid_and_later_exit_update() {
        let dir = tmp_memory_dir("timeout_job");
        let store = FileShellJobStore::new(&dir);
        let mut runtime = NeverCancelRuntime;
        let result = store.run_with_timeout(
            "printf started; sleep 1; printf done",
            &dir,
            100,
            "session_a",
            "turn_a",
            &mut runtime,
        );
        assert!(result.contains("timeout, but is still running"), "{result}");
        assert!(result.contains("process was not killed"), "{result}");
        assert!(result.contains("no final exit code yet"), "{result}");
        let pid = result
            .lines()
            .find_map(|line| line.strip_prefix("pid="))
            .and_then(|rest| rest.split(',').next())
            .and_then(|pid| pid.parse::<u32>().ok())
            .expect("pid");

        let running = store.running_for_session("session_a");
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].pid, pid);
        assert_eq!(running[0].kind, "timeout");

        let mut running = Vec::new();
        let mut updates = Vec::new();
        let wait_started = Instant::now();
        while wait_started.elapsed() < Duration::from_secs(3) {
            (running, updates) = store.refresh_for_session("session_a");
            if running.is_empty() && !updates.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(running.is_empty(), "timed-out job should eventually exit");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].pid, pid);
        assert_eq!(updates[0].description(), "old timeout job");
        assert_eq!(updates[0].status, "0");
        assert_eq!(updates[0].output, "starteddone");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn process_running_treats_zombie_as_not_running() {
        let mut child = Command::new("/bin/sh")
            .arg("-lc")
            .arg("exit 0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn short child");
        let pid = child.id();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) && process_running(pid) {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !process_running(pid),
            "exited child pid {pid} should not be reported as running"
        );
        let _ = child.wait();
    }

    #[test]
    fn running_job_list_context_uses_pid_kind_and_command() {
        let dir = tmp_memory_dir("running_context");
        let store = FileShellJobStore::new(&dir);

        let _ = store.spawn_background("sleep 10", &dir, "session_owned", "turn_a");
        let _ = store.spawn_background("sleep 10", &dir, "session_other", "turn_a");
        let context = store
            .running_job_list_context("session_owned")
            .expect("running context");

        assert!(context.starts_with("RUNNING JOB LIST:"), "{context}");
        assert!(context.contains("background job"), "{context}");
        assert!(context.contains("cmd=sleep 10"), "{context}");
        assert!(!context.contains("session_other"), "{context}");
        for job in store.running_for_session("session_owned") {
            terminate_process(job.pid);
        }
        for job in store.running_for_session("session_other") {
            terminate_process(job.pid);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bash_validation_rejects_empty_and_allows_long_commands() {
        assert_eq!(
            validate_bash_request(""),
            Err("command_required".to_string())
        );
        let huge = "x".repeat(2001);
        assert!(validate_bash_request(&huge).is_ok());
        assert!(validate_bash_request("printf ok").is_ok());
    }
}

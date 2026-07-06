use crate::response_protocol::ParsedAction;
use crate::MemGuard;
use crate::{
    ActionExecution, ActionRuntime, AgentCore, ApprovalRequest, BashApprovalMode,
    LongRunningCommandDecision, LongRunningCommandStatus, PendingApproval, PendingApprovedAction,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static LONG_RUNNING_COMMAND_PROMPT_AFTER_MS: AtomicU64 = AtomicU64::new(60_000);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellJobRecord {
    pub id: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    pub pid: u32,
    pub command: String,
    pub output_file: String,
    pub status_file: String,
}

#[derive(Debug, Clone)]
pub struct FileShellJobStore {
    dir: PathBuf,
    index_file: PathBuf,
    guard: MemGuard,
}

impl FileShellJobStore {
    pub fn new(memory_dir: &Path) -> Self {
        let dir = memory_dir.join("shell_jobs");
        let _ = fs::create_dir_all(&dir);
        Self {
            index_file: dir.join("jobs.jsonl"),
            dir,
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    pub fn spawn(&self, command: &str, session_id: &str, turn_id: &str) -> String {
        let clean = command.trim();
        if clean.is_empty() {
            return bash_action_not_executed(
                None,
                "The background command was not started because no shell command was provided.",
            );
        }
        let _ = fs::create_dir_all(&self.dir);
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let spawn = command.spawn();
        let child = match spawn {
            Ok(child) => child,
            Err(_) => {
                return bash_action_not_executed(
                    Some(clean),
                    "The background command could not be started by the local shell.",
                )
            }
        };
        let record = ShellJobRecord {
            id: id.clone(),
            created_at_ms: now_ms(),
            session_id: session_id.trim().to_string(),
            turn_id: turn_id.trim().to_string(),
            pid: child.id(),
            command: clean.to_string(),
            output_file: output_file.to_string_lossy().to_string(),
            status_file: status_file.to_string_lossy().to_string(),
        };
        let _ = self.append(&record);
        format!(
            "Action result: run_bash\nThe background command has started.\nCommand: {}\nJob id for shell_job_status: {}\nProcess id: {}\nOutput file: {}\nStatus file: {}",
            clean, record.id, record.pid, record.output_file, record.status_file
        )
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

    pub fn status(&self, job_id: &str, wait_ms: u64) -> String {
        let clean_id = job_id.trim();
        if clean_id.is_empty() {
            return "Action result: shell_job_status\nThe job status was not checked because no job_id was provided. Use the job id returned when the background command was started.".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: shell_job_status\nJob id: {}\nNo background job with this id was found in the current shell job store.",
                clean_id
            );
        };
        let wait = Duration::from_millis(wait_ms.min(15000));
        let started = Instant::now();
        loop {
            if let Some(code) = fs::read_to_string(&record.status_file)
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
            {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                if code == "cancelled" {
                    return format!(
                        "Action result: shell_job_status\nThe background job was cancelled.\nJob id: {}\nState: cancelled\nWaited: {} ms\nOutput file: {}\nPartial output:\n{}",
                        record.id,
                        started.elapsed().as_millis(),
                        record.output_file,
                        compact_text(&output, 2000)
                    );
                }
                return format!(
                    "Action result: shell_job_status\nThe background job has finished.\nJob id: {}\nState: finished\nExit code: {}\nWaited: {} ms\nOutput file: {}\nOutput:\n{}",
                    record.id,
                    code,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 4000)
                );
            }
            if started.elapsed() >= wait {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return format!(
                    "Action result: shell_job_status\nThe background job is still running.\nJob id: {}\nState: running\nProcess id: {}\nWaited: {} ms\nOutput file: {}\nPartial output:\n{}",
                    record.id,
                    record.pid,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 2000)
                );
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn cancel(&self, job_id: &str) -> String {
        let clean_id = job_id.trim();
        if clean_id.is_empty() {
            return "Action result: shell_job_status\nThe background job was not cancelled because no job_id was provided.".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: shell_job_status\nJob id: {}\nNo background job with this id was found, so there was nothing to cancel.",
                clean_id
            );
        };
        if let Some(code) = fs::read_to_string(&record.status_file)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        {
            let state = if code == "cancelled" {
                "cancelled"
            } else {
                "finished"
            };
            return format!(
                "Action result: shell_job_status\nThe background job was already completed.\nJob id: {}\nState: {}",
                record.id, state
            );
        }

        terminate_process(record.pid);
        let _ = fs::write(&record.status_file, "cancelled");
        let output = fs::read_to_string(&record.output_file).unwrap_or_default();
        format!(
            "Action result: shell_job_status\nThe background job has been cancelled.\nJob id: {}\nState: cancelled\nProcess id: {}\nOutput file: {}\nPartial output:\n{}",
            record.id,
            record.pid,
            record.output_file,
            compact_text(&output, 2000)
        )
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

    fn find(&self, job_id: &str) -> Option<ShellJobRecord> {
        self.guard.with_read(|| self.find_unlocked(job_id)).ok()?
    }

    fn find_unlocked(&self, job_id: &str) -> Option<ShellJobRecord> {
        self.records_unlocked()
            .into_iter()
            .filter(|record| record.id == job_id)
            .last()
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
}

pub fn validate_bash_request(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("command_required".to_string());
    }
    if trimmed.len() > 2000 {
        return Err("command_too_long".to_string());
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
    execute_run_bash(
        &command_to_run,
        action.background(),
        timeout_ms,
        interval_ms,
        action.input_u64("once_timeout_ms").unwrap_or(5000),
        core.bash_approval_mode,
        &action.intent,
        &core.shell_jobs,
        &session_id,
        &turn_id,
        is_regular_command,
        runtime,
    )
}

pub(crate) fn execute_run_bash(
    command: &str,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    once_timeout_ms: u64,
    approval_mode: BashApprovalMode,
    intent: &str,
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
    if !background
        && timeout_ms >= 0
        && is_regular_command
        && contains_long_normal_sleep(command_to_run)
    {
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
                intent: intent.to_string(),
            },
            approved_action: PendingApprovedAction::RunBash {
                command: command_to_run.to_string(),
                background,
                timeout_ms,
                interval_ms,
                once_timeout_ms,
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
            },
            intent: intent.to_string(),
        });
    }
    if background {
        return ActionExecution::Completed(shell_jobs.spawn(command_to_run, session_id, turn_id));
    }
    if let Some(interval_ms) = interval_ms {
        return ActionExecution::Completed(execute_polling_bash(
            command_to_run,
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            runtime,
        ));
    }
    ActionExecution::Completed(execute_one_bash(command_to_run, timeout_ms, runtime))
}

pub(crate) fn execute_approved_bash(
    command: &str,
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
        shell_jobs.spawn(command.trim(), session_id, turn_id)
    } else if let Some(interval_ms) = interval_ms {
        execute_polling_bash(
            command.trim(),
            interval_ms,
            timeout_ms,
            once_timeout_ms,
            runtime,
        )
    } else {
        execute_one_bash(command.trim(), timeout_ms, runtime)
    };
    result.push_str(&format!(
        "\napproval_id: {}\napproval_status: approved_by_user",
        request.approval_id
    ));
    result
}

pub fn execute_one_bash(command: &str, timeout_ms: i64, runtime: &mut dyn ActionRuntime) -> String {
    execute_one_bash_structured(command, timeout_ms, runtime).to_action_result("run_bash")
}

pub(crate) fn execute_polling_bash(
    command: &str,
    interval_ms: u64,
    timeout_ms: i64,
    once_timeout_ms: u64,
    runtime: &mut dyn ActionRuntime,
) -> String {
    let interval = Duration::from_millis(interval_ms.clamp(1000, 60_000));
    let max_wait =
        (timeout_ms >= 0).then(|| Duration::from_millis((timeout_ms as u64).clamp(1000, 900_000)));
    let once_timeout_ms = once_timeout_ms.clamp(1000, 15_000);
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
        let result = execute_one_bash_structured(command, once_timeout_ms as i64, runtime);
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

        if max_wait.is_some_and(|max_wait| started.elapsed() >= max_wait) {
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

        let wait = max_wait
            .map(|max_wait| interval.min(max_wait.saturating_sub(started.elapsed())))
            .unwrap_or(interval);
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
    timeout_ms: i64,
    runtime: &mut dyn ActionRuntime,
) -> BashCommandOutput {
    execute_one_bash_structured_with_prompt_after(
        command,
        timeout_ms,
        runtime,
        long_running_command_prompt_after(),
    )
}

fn execute_one_bash_structured_with_prompt_after(
    command: &str,
    timeout_ms: i64,
    runtime: &mut dyn ActionRuntime,
    long_running_prompt_after: Duration,
) -> BashCommandOutput {
    let spawn = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(_) => return bash_error(command, "command_failed"),
    };
    let started = Instant::now();
    let timeout =
        (timeout_ms >= 0).then(|| Duration::from_millis((timeout_ms as u64).clamp(1000, 15000)));
    let mut next_long_running_check = long_running_prompt_after;
    loop {
        if runtime.should_cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return bash_error(command, "cancelled");
        }
        if timeout.is_none() && started.elapsed() >= next_long_running_check {
            let status = LongRunningCommandStatus {
                action: "run_bash".to_string(),
                command: command.to_string(),
                elapsed: started.elapsed(),
                timeout_ms: None,
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
            Ok(None) if timeout.is_some_and(|timeout| started.elapsed() >= timeout) => {
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
    let previous_ms = LONG_RUNNING_COMMAND_PROMPT_AFTER_MS
        .swap(duration.as_millis().max(1) as u64, Ordering::Relaxed);
    LongRunningPromptAfterGuard { previous_ms }
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
        "command_too_long" => {
            "The shell command is too long for a single run_bash action. Split the work into smaller commands or write a short script file first."
        }
        _ => "The shell command request did not pass runtime validation.",
    }
}

fn bash_runtime_error_message(error: &str) -> &'static str {
    match error {
        "timeout" => {
            "The command was stopped because it exceeded the configured timeout. For long local work, use background=true; for waiting on external state, use loop_cmd with interval_ms."
        }
        "cancelled" | "cancelled_by_user" => {
            "The command was cancelled before it completed."
        }
        "command_failed" => {
            "The local shell could not start or wait for the command successfully."
        }
        _ => "The command did not complete successfully.",
    }
}

fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn terminate_process(pid: u32) {
    #[cfg(unix)]
    {
        let group = format!("-{}", pid);
        let status = Command::new("/bin/kill").arg("-TERM").arg(&group).status();
        if status.as_ref().is_ok_and(|s| s.success()) {
            return;
        }
    }
    let _ = Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
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
        assert!(
            result.contains("exceeded the configured timeout"),
            "{result}"
        );
    }

    #[test]
    fn normal_bash_timeout_minus_one_waits_without_runtime_timeout() {
        let mut runtime = NeverCancelRuntime;
        let result = execute_one_bash("sleep 1; printf no_timeout_ok", -1, &mut runtime);
        assert!(result.contains("Exit code: 0"), "{result}");
        assert!(result.contains("no_timeout_ok"), "{result}");
    }

    #[test]
    fn normal_bash_timeout_minus_one_reports_long_running_status_to_runtime() {
        let _guard = set_long_running_command_prompt_after_for_tests(Duration::from_millis(50));
        let mut runtime = CancelAfterLongRunningPromptRuntime::default();
        let result = execute_one_bash("sleep 2; printf should_not_finish", -1, &mut runtime);

        assert!(result.contains("cancelled before it completed"), "{result}");
        assert_eq!(runtime.prompts.len(), 1);
        assert_eq!(runtime.prompts[0].action, "run_bash");
        assert_eq!(
            runtime.prompts[0].command,
            "sleep 2; printf should_not_finish"
        );
        assert_eq!(runtime.prompts[0].timeout_ms, None);
        assert!(runtime.prompts[0].elapsed >= Duration::from_millis(50));
    }

    #[test]
    fn normal_run_bash_rejects_long_sleep_commands() {
        let store = FileShellJobStore::new(&tmp_memory_dir("long_sleep_guard"));
        let result = execute_run_bash(
            "sleep 90 && printf done",
            false,
            5000,
            None,
            5000,
            BashApprovalMode::Approve,
            "Wait then check.",
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
        let result = execute_run_bash(
            "sleep 1; printf done",
            false,
            3000,
            None,
            5000,
            BashApprovalMode::Approve,
            "Short wait.",
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
        let result = execute_polling_bash(&command, 1000, 5000, 1000, &mut runtime);
        assert!(result.contains("Action result: run_bash"), "{result}");
        assert!(result.contains("Polling state: finished"), "{result}");
        assert!(result.contains("Attempts: 2"), "{result}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn run_bash_poll_mode_times_out_when_command_stays_nonzero() {
        let mut runtime = NeverCancelRuntime;
        let result = execute_polling_bash("printf waiting; exit 7", 1000, 1100, 1000, &mut runtime);
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
        let result = execute_polling_bash("exit 1", 1000, 10_000, 1000, &mut runtime);
        assert!(result.contains("Polling state: cancelled"), "{result}");
    }

    #[test]
    fn run_bash_poll_mode_requests_user_approval_in_ask_mode() {
        let store = FileShellJobStore::new(&tmp_memory_dir("poll_approval"));
        let result = execute_run_bash(
            "test -f /tmp/timem_poll_marker",
            false,
            5000,
            Some(1000),
            1000,
            BashApprovalMode::Ask,
            "等待外部状态完成",
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
                assert_eq!(pending.request.intent, "等待外部状态完成");
            }
            other => panic!("expected run_bash approval request, got {other:?}"),
        }
    }

    #[test]
    fn run_bash_polling_requires_loop_cmd_and_interval_pair() {
        let store = FileShellJobStore::new(&tmp_memory_dir("poll_pairing"));
        let cmd_with_interval = execute_run_bash(
            "test -f /tmp/timem_poll_marker",
            false,
            5000,
            Some(1000),
            1000,
            BashApprovalMode::Approve,
            "等待外部状态完成",
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
            false,
            5000,
            None,
            1000,
            BashApprovalMode::Approve,
            "等待外部状态完成",
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
    fn background_job_can_be_polled_until_finished() {
        let dir = tmp_memory_dir("background_job");
        let store = FileShellJobStore::new(&dir);
        let started = store.spawn("printf background_ok", "session_a", "turn_a");
        assert!(started.contains("background command has started"));
        let job_id = started
            .lines()
            .find_map(|line| line.strip_prefix("Job id for shell_job_status: "))
            .unwrap()
            .to_string();
        let status = store.status(&job_id, 1000);
        assert!(status.contains("State: finished"), "{status}");
        assert!(status.contains("background_ok"), "{status}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn background_status_requires_known_job_id() {
        let dir = tmp_memory_dir("missing_job");
        let store = FileShellJobStore::new(&dir);
        let missing = store.status("missing", 0);
        assert!(missing.contains("No background job with this id was found"));
        let empty = store.status("", 0);
        assert!(empty.contains("no job_id was provided"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn background_job_can_be_cancelled() {
        let dir = tmp_memory_dir("background_cancel");
        let store = FileShellJobStore::new(&dir);

        let started = store.spawn(
            "printf started; sleep 10; printf done",
            "session_a",
            "turn_a",
        );
        let job_id = started
            .lines()
            .find_map(|line| line.strip_prefix("Job id for shell_job_status: "))
            .expect("job id");
        let cancelled = store.cancel(job_id);

        assert!(
            cancelled.contains("Action result: shell_job_status"),
            "{cancelled}"
        );
        assert!(cancelled.contains("State: cancelled"), "{cancelled}");
        let status = store.status(job_id, 0);
        assert!(status.contains("State: cancelled"), "{status}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn background_jobs_can_be_cancelled_by_session_owner() {
        let dir = tmp_memory_dir("background_cancel_session");
        let store = FileShellJobStore::new(&dir);

        let owned = store.spawn("sleep 10", "session_owned", "turn_a");
        let other = store.spawn("sleep 10", "session_other", "turn_a");
        let owned_job = owned
            .lines()
            .find_map(|line| line.strip_prefix("Job id for shell_job_status: "))
            .expect("owned job");
        let other_job = other
            .lines()
            .find_map(|line| line.strip_prefix("Job id for shell_job_status: "))
            .expect("other job");

        let cancelled = store.cancel_unfinished_for_session("session_owned");
        assert_eq!(cancelled, vec![owned_job.to_string()]);
        assert!(store.status(owned_job, 0).contains("State: cancelled"));
        assert!(store.status(other_job, 0).contains("State: running"));
        let _ = store.cancel(other_job);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bash_validation_rejects_empty_and_huge_commands() {
        assert_eq!(
            validate_bash_request(""),
            Err("command_required".to_string())
        );
        let huge = "x".repeat(2001);
        assert_eq!(
            validate_bash_request(&huge),
            Err("command_too_long".to_string())
        );
        assert!(validate_bash_request("printf ok").is_ok());
    }
}

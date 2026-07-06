use crate::response_protocol::ParsedAction;
use crate::MemGuard;
use crate::{
    ActionExecution, AgentCore, ApprovalRequest, BashApprovalMode, PendingApproval,
    PendingApprovedAction,
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
            return "Action result: run_bash\nerror: command_required".to_string();
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
                return format!(
                    "Action result: run_bash\ncommand: {}\nerror: background_spawn_failed",
                    clean
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
            "Action result: run_bash\ncommand: {}\nstatus: background_started\njob_id: {}\npid: {}\noutput_file: {}\nstatus_file: {}\nnext_action: shell_job_status",
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
            return "Action result: shell_job_status\nerror: job_id_required".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: shell_job_status\njob_id: {}\nerror: job_not_found",
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
                        "Action result: shell_job_status\njob_id: {}\nstate: cancelled\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
                        record.id,
                        started.elapsed().as_millis(),
                        record.output_file,
                        compact_text(&output, 2000)
                    );
                }
                return format!(
                    "Action result: shell_job_status\njob_id: {}\nstate: finished\nexit_code: {}\nwaited_ms: {}\noutput_file: {}\noutput:\n{}",
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
                    "Action result: shell_job_status\njob_id: {}\nstate: running\npid: {}\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
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
            return "Action result: shell_job_status\nerror: job_id_required".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: shell_job_status\njob_id: {}\nerror: job_not_found",
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
                "Action result: shell_job_status\njob_id: {}\nstate: {}\nstatus: already_completed",
                record.id, state
            );
        }

        terminate_process(record.pid);
        let _ = fs::write(&record.status_file, "cancelled");
        let output = fs::read_to_string(&record.output_file).unwrap_or_default();
        format!(
            "Action result: shell_job_status\njob_id: {}\nstate: cancelled\npid: {}\noutput_file: {}\npartial_output:\n{}",
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
) -> ActionExecution {
    let loop_command = action.input_str("loop_cmd");
    if !loop_command.is_empty() && !action.input_str("cmd").is_empty() {
        return ActionExecution::Completed(
            "Action result: run_bash\nerror: cmd_and_loop_cmd_conflict".to_string(),
        );
    }
    let command_to_run = if loop_command.is_empty() {
        command_from_action(action)
    } else {
        loop_command
    };
    let interval_ms = action.input_u64("interval_ms");
    let timeout_ms = action.timeout_ms_i64(5000);
    let session_id = core.current_session_id();
    let turn_id = core.current_action_turn_id();
    execute_run_bash(
        &command_to_run,
        action.background(),
        timeout_ms,
        interval_ms,
        action.input_u64("check_timeout_ms").unwrap_or(5000),
        core.bash_approval_mode,
        &action.intent,
        &core.shell_jobs,
        &session_id,
        &turn_id,
        action.input_str("loop_cmd").is_empty(),
        &mut || false,
    )
}

pub(crate) fn execute_run_bash(
    command: &str,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    check_timeout_ms: u64,
    approval_mode: BashApprovalMode,
    intent: &str,
    shell_jobs: &FileShellJobStore,
    session_id: &str,
    turn_id: &str,
    is_regular_command: bool,
    should_cancel: &mut dyn FnMut() -> bool,
) -> ActionExecution {
    let command_to_run = command.trim();
    if command_to_run.is_empty() {
        return ActionExecution::Completed(
            "Action result: run_bash\nerror: command_required".to_string(),
        );
    }
    if let Err(reason) = validate_bash_request(command_to_run) {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: {}",
            command_to_run, reason
        ));
    }
    if !background
        && timeout_ms >= 0
        && is_regular_command
        && contains_long_foreground_sleep(command_to_run)
    {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: long_sleep_in_foreground_command\nmessage: Use run_bash with interval_ms for waiting on external status, or background=true for long local work.",
            command_to_run
        ));
    }
    if background && interval_ms.is_some() {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: poll_mode_cannot_be_background",
            command_to_run
        ));
    }
    if interval_ms.is_some() && is_regular_command {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: loop_cmd_required_for_polling",
            command_to_run
        ));
    }
    if interval_ms.is_none() && !is_regular_command {
        return ActionExecution::Completed(format!(
            "Action result: run_bash\ncommand: {}\nerror: interval_ms_required_for_loop_cmd",
            command_to_run
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
                check_timeout_ms,
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
            check_timeout_ms,
            should_cancel,
        ));
    }
    ActionExecution::Completed(execute_one_bash(command_to_run, timeout_ms, should_cancel))
}

pub(crate) fn execute_approved_bash(
    command: &str,
    background: bool,
    timeout_ms: i64,
    interval_ms: Option<u64>,
    check_timeout_ms: u64,
    session_id: &str,
    turn_id: &str,
    _is_regular_command: bool,
    request: &ApprovalRequest,
    shell_jobs: &FileShellJobStore,
    should_cancel: &mut dyn FnMut() -> bool,
) -> String {
    let mut result = if background {
        shell_jobs.spawn(command.trim(), session_id, turn_id)
    } else if let Some(interval_ms) = interval_ms {
        execute_polling_bash(
            command.trim(),
            interval_ms,
            timeout_ms,
            check_timeout_ms,
            should_cancel,
        )
    } else {
        execute_one_bash(command.trim(), timeout_ms, should_cancel)
    };
    result.push_str(&format!(
        "\napproval_id: {}\napproval_status: approved_by_user",
        request.approval_id
    ));
    result
}

pub fn execute_one_bash(
    command: &str,
    timeout_ms: i64,
    should_cancel: &mut dyn FnMut() -> bool,
) -> String {
    execute_one_bash_structured(command, timeout_ms, should_cancel).to_action_result("run_bash")
}

pub(crate) fn execute_polling_bash(
    command: &str,
    interval_ms: u64,
    timeout_ms: i64,
    check_timeout_ms: u64,
    mut cancelled: impl FnMut() -> bool,
) -> String {
    let interval = Duration::from_millis(interval_ms.clamp(1000, 60_000));
    let max_wait =
        (timeout_ms >= 0).then(|| Duration::from_millis((timeout_ms as u64).clamp(1000, 900_000)));
    let check_timeout_ms = check_timeout_ms.clamp(1000, 15_000);
    let started = Instant::now();
    let mut attempts = 0_u64;
    let mut last_status = None;
    let mut last_output = String::new();
    let mut last_error = None;

    loop {
        if cancelled() {
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
        let result = execute_one_bash_structured(command, check_timeout_ms as i64, &mut cancelled);
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
        sleep_cancelable(wait, &mut cancelled);
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
    let mut out = format!(
        "Action result: run_bash\nmode: poll\ncommand: {}\nstate: {}\nsuccess_exit_code: 0\nattempts: {}\nelapsed_ms: {}",
        command,
        state,
        attempts,
        elapsed.as_millis()
    );
    if let Some(status) = last_status {
        out.push_str(&format!("\nlast_status: {status}"));
    }
    if let Some(error) = error {
        out.push_str(&format!("\nlast_error: {error}"));
    }
    if !output.trim().is_empty() {
        out.push_str("\noutput:\n");
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

fn contains_long_foreground_sleep(command: &str) -> bool {
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
                "Action result: {}\ncommand: {}\nerror: {}",
                action_name, self.command, error
            );
        }
        format!(
            "Action result: {}\ncommand: {}\nstatus: {}\noutput:\n{}",
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
    should_cancel: &mut dyn FnMut() -> bool,
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
    loop {
        if should_cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return bash_error(command, "cancelled");
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

fn bash_error(command: &str, error: &str) -> BashCommandOutput {
    BashCommandOutput {
        command: command.to_string(),
        status: None,
        output: String::new(),
        error: Some(error.to_string()),
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
    fn foreground_bash_reports_status_and_output() {
        let result = execute_one_bash("printf shell_ok", 1000, &mut || false);
        assert!(result.contains("Action result: run_bash"));
        assert!(result.contains("status: 0"));
        assert!(result.contains("shell_ok"));
    }

    #[test]
    fn foreground_bash_timeout_is_bounded() {
        let result = execute_one_bash("sleep 2", 1000, &mut || false);
        assert!(result.contains("error: timeout"));
    }

    #[test]
    fn foreground_bash_timeout_minus_one_waits_without_runtime_timeout() {
        let result = execute_one_bash("sleep 1; printf no_timeout_ok", -1, &mut || false);
        assert!(result.contains("status: 0"), "{result}");
        assert!(result.contains("no_timeout_ok"), "{result}");
    }

    #[test]
    fn foreground_run_bash_rejects_long_sleep_commands() {
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
            &mut || false,
        );
        match result {
            ActionExecution::Completed(text) => {
                assert!(text.contains("long_sleep_in_foreground_command"));
                assert!(text.contains("interval_ms"));
            }
            ActionExecution::NeedsApproval(_) => {
                panic!("long sleep should be rejected before approval")
            }
        }
    }

    #[test]
    fn foreground_run_bash_allows_short_sleep_commands() {
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
            &mut || false,
        );
        match result {
            ActionExecution::Completed(text) => {
                assert!(text.contains("status: 0"));
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
        let result = execute_polling_bash(&command, 1000, 5000, 1000, || false);
        assert!(result.contains("Action result: run_bash"), "{result}");
        assert!(result.contains("mode: poll"), "{result}");
        assert!(result.contains("state: finished"), "{result}");
        assert!(result.contains("attempts: 2"), "{result}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn run_bash_poll_mode_times_out_when_command_stays_nonzero() {
        let result = execute_polling_bash("printf waiting; exit 7", 1000, 1100, 1000, || false);
        assert!(result.contains("mode: poll"), "{result}");
        assert!(result.contains("state: timeout"), "{result}");
        assert!(result.contains("last_status: 7"), "{result}");
        assert!(result.contains("waiting"), "{result}");
    }

    #[test]
    fn run_bash_poll_mode_can_be_cancelled_during_wait() {
        let cancelled = AtomicBool::new(false);
        let result = execute_polling_bash("exit 1", 1000, 10_000, 1000, || {
            let previous = cancelled.swap(true, Ordering::Relaxed);
            previous
        });
        assert!(result.contains("state: cancelled"), "{result}");
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
            &mut || false,
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
            &mut || false,
        );
        match cmd_with_interval {
            ActionExecution::Completed(text) => {
                assert!(text.contains("loop_cmd_required_for_polling"), "{text}");
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
            &mut || false,
        );
        match loop_without_interval {
            ActionExecution::Completed(text) => {
                assert!(text.contains("interval_ms_required_for_loop_cmd"), "{text}");
            }
            other => panic!("expected pairing error, got {other:?}"),
        }
    }

    #[test]
    fn background_job_can_be_polled_until_finished() {
        let dir = tmp_memory_dir("background_job");
        let store = FileShellJobStore::new(&dir);
        let started = store.spawn("printf background_ok", "session_a", "turn_a");
        assert!(started.contains("status: background_started"));
        let job_id = started
            .lines()
            .find_map(|line| line.strip_prefix("job_id: "))
            .unwrap()
            .to_string();
        let status = store.status(&job_id, 1000);
        assert!(status.contains("state: finished"), "{status}");
        assert!(status.contains("background_ok"), "{status}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn background_status_requires_known_job_id() {
        let dir = tmp_memory_dir("missing_job");
        let store = FileShellJobStore::new(&dir);
        let missing = store.status("missing", 0);
        assert!(missing.contains("error: job_not_found"));
        let empty = store.status("", 0);
        assert!(empty.contains("error: job_id_required"));
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
            .find_map(|line| line.strip_prefix("job_id: "))
            .expect("job id");
        let cancelled = store.cancel(job_id);

        assert!(
            cancelled.contains("Action result: shell_job_status"),
            "{cancelled}"
        );
        assert!(cancelled.contains("state: cancelled"), "{cancelled}");
        let status = store.status(job_id, 0);
        assert!(status.contains("state: cancelled"), "{status}");
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
            .find_map(|line| line.strip_prefix("job_id: "))
            .expect("owned job");
        let other_job = other
            .lines()
            .find_map(|line| line.strip_prefix("job_id: "))
            .expect("other job");

        let cancelled = store.cancel_unfinished_for_session("session_owned");
        assert_eq!(cancelled, vec![owned_job.to_string()]);
        assert!(store.status(owned_job, 0).contains("state: cancelled"));
        assert!(store.status(other_job, 0).contains("state: running"));
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

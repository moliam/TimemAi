use crate::response_protocol::ParsedAction;
use crate::MemGuard;
use crate::{
    ActionExecution, ActionOutcome, ActionRuntime, ActionStatus, AgentCore, ApprovalRequest,
    BashApprovalMode, LongRunningCommandStatus, PendingApproval, PendingApprovedAction,
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

#[cfg(unix)]
use std::os::unix::process::CommandExt;

static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
const BASH_EXECUTABLE: &str = "/bin/bash";
const MAX_BASH_OUTPUT_CHARS: usize = 32 * 1024;
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
    pub owner_id: Option<String>,
    pub command: String,
    #[serde(default)]
    pub cwd: String,
    pub output_file: String,
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
            jobs.insert(pid, WatchedShellChild { child, status_file });
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
        let status = match watched.child.try_wait() {
            Ok(Some(status)) => Some(exit_status_text(&status)),
            Ok(None) => None,
            Err(_) => Some("unknown".to_string()),
        };
        if let Some(status) = status {
            if let Some(watched) = jobs.remove(&pid) {
                write_status_if_empty(&watched.status_file, &status);
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
            match watched.child.try_wait() {
                Ok(Some(status)) => finished.push((*pid, exit_status_text(&status))),
                Ok(None) => {}
                Err(_) => finished.push((*pid, "unknown".to_string())),
            }
        }
        for (pid, status) in finished {
            if let Some(watched) = jobs.remove(&pid) {
                write_status_if_empty(&watched.status_file, &status);
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
            guard: MemGuard::for_memory_dir(memory_dir),
            watcher: ShellJobWatcher::new(),
            long_running_prompt_after: LONG_RUNNING_COMMAND_PROMPT_AFTER,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_long_running_prompt_after_for_tests(&mut self, duration: Duration) {
        self.long_running_prompt_after = duration.max(Duration::from_millis(1));
    }

    pub fn spawn_background(
        &self,
        command: &str,
        cwd: &Path,
        session_id: &str,
        turn_id: &str,
    ) -> String {
        self.spawn_background_outcome(command, cwd, session_id, turn_id, false)
            .text
    }

    pub(crate) fn spawn_background_outcome(
        &self,
        command: &str,
        cwd: &Path,
        session_id: &str,
        turn_id: &str,
        tail_out: bool,
    ) -> ActionOutcome {
        let clean = command.trim();
        if clean.is_empty() {
            return ActionOutcome::failed(bash_action_not_executed(
                None,
                "The background command was not started because no shell command was provided.",
            ));
        }
        let record =
            match self.spawn_record(clean, cwd, "background", session_id, turn_id, tail_out) {
                Ok(record) => record,
                Err(_) => {
                    return ActionOutcome::failed(bash_action_not_executed(
                        Some(clean),
                        "The background command could not be started by the local shell.",
                    ));
                }
            };
        let _ = self.append(&record);
        ActionOutcome::background_running(format!(
            "Action result: run_bash\npid={}, now keeps running in background\nCommand: {}",
            record.pid, clean
        ))
    }

    fn spawn_record(
        &self,
        clean: &str,
        cwd: &Path,
        kind: &str,
        session_id: &str,
        turn_id: &str,
        tail_out: bool,
    ) -> std::io::Result<ShellJobRecord> {
        fs::create_dir_all(&self.dir)?;
        let id = unique_shell_id("job");
        let output_file = self.dir.join(format!("{id}.out"));
        let status_file = self.dir.join(format!("{id}.status"));
        let output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&output_file)?;
        let stderr = output.try_clone()?;
        let mut command = Command::new(BASH_EXECUTABLE);
        configure_run_bash_environment(&mut command);
        command
            .arg("-c")
            .arg(clean)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let child = command.spawn()?;
        let pid = child.id();
        self.watcher.register(pid, child, status_file.clone());
        Ok(ShellJobRecord {
            id,
            created_at_ms: now_ms(),
            kind: kind.to_string(),
            session_id: session_id.trim().to_string(),
            turn_id: turn_id.trim().to_string(),
            pid,
            owner_id: Some(crate::runtime_process_owner_id().to_string()),
            command: clean.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            output_file: output_file.to_string_lossy().to_string(),
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
            command, cwd, timeout_ms, session_id, turn_id, false, runtime,
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
        tail_out: bool,
        runtime: &mut dyn ActionRuntime,
    ) -> ActionOutcome {
        self.run_with_timeout_structured(
            command, cwd, timeout_ms, session_id, turn_id, tail_out, runtime,
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
        tail_out: bool,
        runtime: &mut dyn ActionRuntime,
    ) -> BashCommandOutput {
        let clean = command.trim();
        if timeout_ms <= 0 {
            return bash_error(clean, "invalid_timeout");
        }
        let Ok(record) = self.spawn_record(clean, cwd, "timeout", session_id, turn_id, tail_out)
        else {
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
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: status.code,
                    signal: status.signal,
                    output: normalized_shell_output(&output),
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
                let partial = fs::read_to_string(&record.output_file).unwrap_or_default();
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: None,
                    signal: None,
                    output: partial,
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
                let partial = fs::read_to_string(&record.output_file).unwrap_or_default();
                return BashCommandOutput {
                    command: clean.to_string(),
                    status: None,
                    signal: None,
                    output: partial,
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
        let records = self
            .guard
            .with_read(|| self.records_unlocked())
            .unwrap_or_default();
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
            output: compact_text_with_tail(
                &normalized_shell_output(
                    &fs::read_to_string(&record.output_file).unwrap_or_default(),
                ),
                MAX_BASH_OUTPUT_CHARS,
                record.tail_out,
            ),
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
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            None,
            "The action provided both cmd and loop_cmd. Use cmd for a normal/background command, or loop_cmd with interval_ms for polling.",
        )));
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
    is_regular_command: bool,
    tail_out: bool,
    runtime: &mut dyn ActionRuntime,
) -> ActionExecution {
    let command_to_run = command.trim();
    if command_to_run.is_empty() {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            None,
            "The command was not executed because no shell command was provided.",
        )));
    }
    if let Err(reason) = validate_bash_request(command_to_run) {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            bash_validation_message(&reason),
        )));
    }
    if !background && is_regular_command && timeout_ms <= 0 {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            "timeout_ms must be a positive integer. Choose a wait budget that matches the command.",
        )));
    }
    if !background && !is_regular_command && timeout_ms <= 0 {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            "loop_timeout_ms must be a positive integer. Choose a total polling wait budget that matches the external state you are waiting for.",
        )));
    }
    if !background && is_regular_command && contains_long_normal_sleep(command_to_run) {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            "The command contains a long sleep in normal mode. Use loop_cmd with interval_ms to poll external status, or background=true for long local work that should continue across turns.",
        )));
    }
    if background && interval_ms.is_some() {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            "Polling mode and background mode cannot be combined. Use loop_cmd with interval_ms for polling, or background=true for a persistent background command.",
        )));
    }
    if interval_ms.is_some() && is_regular_command {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            "interval_ms is only valid with loop_cmd. Move the check command to loop_cmd, or remove interval_ms for a normal command.",
        )));
    }
    if interval_ms.is_none() && !is_regular_command {
        return ActionExecution::Completed(ActionOutcome::failed(bash_action_not_executed(
            Some(command_to_run),
            "loop_cmd needs interval_ms so the runtime knows how often to check the condition.",
        )));
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
                tail_out,
            },
            action_name: None,
            continuation: None,
        });
    }
    if background {
        return ActionExecution::Completed(shell_jobs.spawn_background_outcome(
            command_to_run,
            cwd,
            session_id,
            turn_id,
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
    _is_regular_command: bool,
    tail_out: bool,
    request: &ApprovalRequest,
    shell_jobs: &FileShellJobStore,
    runtime: &mut dyn ActionRuntime,
) -> ActionOutcome {
    let clean = command.trim();
    if let Err(reason) = validate_bash_request(clean) {
        let mut outcome = ActionOutcome::failed(bash_action_not_executed(
            Some(clean),
            bash_validation_message(&reason),
        ));
        outcome.text.push_str(&format!(
            "\napproval_id: {}\napproval_status: approved_by_user",
            request.approval_id
        ));
        return outcome;
    }
    let mut outcome = if background {
        shell_jobs.spawn_background_outcome(clean, cwd, session_id, turn_id, tail_out)
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
            clean, cwd, timeout_ms, session_id, turn_id, tail_out, runtime,
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
                tail_out,
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
    command: &str,
    state: &str,
    attempts: u64,
    elapsed: Duration,
    last_status: Option<i32>,
    output: &str,
    tail_out: bool,
    error: Option<&str>,
) -> ActionOutcome {
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
        out.push_str(&compact_text_with_tail(
            output,
            MAX_BASH_OUTPUT_CHARS,
            tail_out,
        ));
    }
    let status = match state {
        "finished" => ActionStatus::Completed,
        "timeout" => ActionStatus::Timeout,
        "cancelled" => ActionStatus::Cancelled,
        _ => ActionStatus::Failed,
    };
    ActionOutcome::new(status, out)
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
                _ if error.starts_with("timeout_still_running:") => ActionStatus::Timeout,
                _ if error.starts_with("long_running_still_running:") => {
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
        ActionOutcome::new(status, text)
    }

    fn render_action_result(&self, action_name: &str) -> String {
        if let Some(error) = &self.error {
            if let Some(details) = error.strip_prefix("long_running_still_running:") {
                let (pid, elapsed_ms) = details.split_once(':').unwrap_or((details, "unknown"));
                let mut out = format!(
                    "Action result: {}\nLONG_RUNNING_COMMAND_STATUS:\nCommand: {}\nPID: {}\nElapsed: {} ms\nStatus: still running\nThe command has not finished. Continue the task by deciding whether to wait, inspect, terminate, or take another appropriate action. Do not ask the user merely because this command is still running.",
                    action_name, self.command, pid, elapsed_ms
                );
                if !self.output.trim().is_empty() {
                    out.push_str("\nPartial output:\n");
                    out.push_str(&compact_text_with_tail(&self.output, 2000, self.tail_out));
                }
                return out;
            }
            if let Some(pid) = error.strip_prefix("timeout_still_running:") {
                let mut out = format!(
                    "Action result: {}\npid={}, timeout, but is still running\nTimeout means Timem stopped waiting; the process was not killed and there is no final exit code yet.\nCommand: {}",
                    action_name, pid, self.command
                );
                if !self.output.trim().is_empty() {
                    out.push_str("\nPartial output:\n");
                    out.push_str(&compact_text_with_tail(&self.output, 2000, self.tail_out));
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
        if let Some(signal) = self.signal {
            return format!(
                "Action result: {}\nThe command terminated because of a process signal.\nCommand: {}\nSignal: {}\nOutput:\n{}",
                action_name,
                self.command,
                signal,
                compact_text_with_tail(&self.output, MAX_BASH_OUTPUT_CHARS, self.tail_out)
            );
        }
        format!(
            "Action result: {}\nThe command finished.\nCommand: {}\nExit code: {}\nOutput:\n{}",
            action_name,
            self.command,
            self.status.unwrap_or(-1),
            compact_text_with_tail(&self.output, MAX_BASH_OUTPUT_CHARS, self.tail_out)
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
    let mut shell = Command::new(BASH_EXECUTABLE);
    configure_run_bash_environment(&mut shell);
    shell
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        shell.process_group(0);
    }
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
                status: output.status.code(),
                signal: exit_signal(&output.status),
                output: combined,
                error: None,
                tail_out: false,
            }
        }
        Err(_) => bash_error(command, "command_failed"),
    }
}

fn bash_error(command: &str, error: &str) -> BashCommandOutput {
    BashCommandOutput {
        command: command.to_string(),
        status: None,
        signal: None,
        output: String::new(),
        error: Some(error.to_string()),
        tail_out: false,
    }
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
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
        "dangerous_recursive_root_delete" => {
            "The shell command was blocked by Timem safety policy because it may recursively delete the filesystem root."
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
            if !output.status.success() {
                return false;
            }
            let stat = String::from_utf8_lossy(&output.stdout);
            let state = stat.trim();
            if state.starts_with('Z') || state.contains('Z') {
                return false;
            }
            return !state.is_empty();
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

#[cfg(test)]
fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn terminate_process(pid: u32) {
    #[cfg(unix)]
    {
        terminate_process_unix(pid);
    }
    #[cfg(not(unix))]
    {
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
}

#[cfg(unix)]
fn terminate_process_unix(pid: u32) {
    let pid = pid as libc::pid_t;
    let pgid = unsafe { libc::getpgid(pid) };
    if pgid < 0 {
        return;
    }
    if pgid == pid && pgid != unsafe { libc::getpgrp() } {
        signal_process_group(pgid, libc::SIGTERM);
        thread::sleep(Duration::from_millis(100));
        if process_group_running(pgid as u32) {
            signal_process_group(pgid, libc::SIGKILL);
        }
        return;
    }
    signal_process(pid, libc::SIGTERM);
    thread::sleep(Duration::from_millis(100));
    if process_running(pid as u32) {
        signal_process(pid, libc::SIGKILL);
    }
}

#[cfg(unix)]
fn signal_process(pid: libc::pid_t, signal: libc::c_int) {
    if pid > 1 && pid != unsafe { libc::getpid() } {
        let _ = unsafe { libc::kill(pid, signal) };
    }
}

#[cfg(unix)]
fn signal_process_group(pgid: libc::pid_t, signal: libc::c_int) {
    if pgid > 1 && pgid != unsafe { libc::getpgrp() } {
        let _ = unsafe { libc::kill(-pgid, signal) };
    }
}

#[cfg(unix)]
fn process_group_running(group_leader_pid: u32) -> bool {
    if group_leader_pid as libc::pid_t == unsafe { libc::getpgrp() } {
        return false;
    }
    let result = unsafe { libc::kill(-(group_leader_pid as libc::pid_t), 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub(crate) fn compact_text(text: &str, max_chars: usize) -> String {
    compact_text_with_tail(text, max_chars, false)
}

pub(crate) fn compact_text_with_tail(text: &str, max_chars: usize, tail_out: bool) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized;
    }

    if tail_out {
        let truncated = normalized
            .chars()
            .take(char_count.saturating_sub(max_chars))
            .collect::<String>();
        let truncated_words = truncated.split_whitespace().count();
        let retained = normalized
            .chars()
            .skip(char_count.saturating_sub(max_chars))
            .collect::<String>();
        format!(
            "!!!Too long, {truncated_words} words truncated before. Generate more actions if necessary !!!\n{retained}"
        )
    } else {
        let retained = normalized.chars().take(max_chars).collect::<String>();
        let truncated = normalized.chars().skip(max_chars).collect::<String>();
        let truncated_words = truncated.split_whitespace().count();
        format!(
            "{retained}\n!!!Too long, {truncated_words} words truncated after. Generate more actions if necessary !!!"
        )
    }
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

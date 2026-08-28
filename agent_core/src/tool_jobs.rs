use crate::{ActionOutcome, MemGuard};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TOOL_JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolJobRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub pid: u32,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub session_id: String,
    pub action: String,
    pub command_path: String,
    pub payload_file: String,
    pub output_file: String,
    pub status_file: String,
}

#[derive(Debug, Clone)]
pub struct FileToolJobStore {
    dir: PathBuf,
    index_file: PathBuf,
    guard: MemGuard,
}

impl FileToolJobStore {
    pub fn new(memory_dir: &Path) -> Self {
        let dir = memory_dir.join("tool_jobs");
        let _ = fs::create_dir_all(&dir);
        Self {
            index_file: dir.join("jobs.jsonl"),
            dir,
            guard: MemGuard::for_memory_domain(memory_dir, "tool-jobs"),
        }
    }

    pub fn spawn(&self, action: &str, path: &Path, payload: &Value) -> String {
        self.spawn_outcome("default", action, path, payload).text
    }

    pub(crate) fn spawn_outcome(
        &self,
        session_id: &str,
        action: &str,
        path: &Path,
        payload: &Value,
    ) -> ActionOutcome {
        let _ = fs::create_dir_all(&self.dir);
        let id = unique_job_id("tool_job");
        let payload_file = self.dir.join(format!("{id}.payload.json"));
        let output_file = self.dir.join(format!("{id}.out"));
        let status_file = self.dir.join(format!("{id}.status"));
        if let Err(err) = fs::write(&payload_file, payload.to_string()) {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: background_payload_write_failed\nreason: {}",
                compact_text(&err.to_string(), 1000)
            ));
        }

        let script = format!(
            "{} {} < {} > {} 2>&1; printf '%s' \"$?\" > {}",
            crate::os::POSIX_SHELL_EXECUTABLE,
            shell_quote_path(path),
            shell_quote_path(&payload_file),
            shell_quote_path(&output_file),
            shell_quote_path(&status_file)
        );
        let mut command = Command::new(crate::os::POSIX_SHELL_EXECUTABLE);
        command
            .arg("-lc")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::os::configure_child_process_group(&mut command);
        let spawn = command.spawn();
        let child = match spawn {
            Ok(child) => child,
            Err(err) => {
                return ActionOutcome::failed(format!(
                    "Action result: {action}\nerror: background_spawn_failed\nreason: {}",
                    compact_text(&err.to_string(), 1000)
                ))
            }
        };
        let record = ToolJobRecord {
            id: id.clone(),
            created_at_ms: now_ms(),
            pid: child.id(),
            owner_id: Some(crate::runtime_process_owner_id().to_string()),
            session_id: session_id.to_string(),
            action: action.to_string(),
            command_path: path.to_string_lossy().to_string(),
            payload_file: payload_file.to_string_lossy().to_string(),
            output_file: output_file.to_string_lossy().to_string(),
            status_file: status_file.to_string_lossy().to_string(),
        };
        let _ = self.append(&record);
        ActionOutcome::background_running(format!(
            "Action result: {action}\nstatus: background_started\njob_id: {}\npid: {}\noutput_file: {}\nstatus_file: {}\nnext_action: capmgr op=job_status",
            record.id, record.pid, record.output_file, record.status_file
        ))
    }

    pub fn status(&self, job_id: &str, wait_ms: u64) -> String {
        self.status_outcome(job_id, wait_ms).text
    }

    pub(crate) fn status_outcome(&self, job_id: &str, wait_ms: u64) -> ActionOutcome {
        let clean_id = job_id.trim();
        if clean_id.is_empty() {
            return ActionOutcome::failed(
                "Action result: capmgr\nop: job_status\nerror: job_id_required",
            );
        }
        let Some(record) = self.find(clean_id) else {
            return ActionOutcome::failed(format!(
                "Action result: capmgr\nop: job_status\njob_id: {}\nerror: job_not_found",
                clean_id
            ));
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
                    return ActionOutcome::cancelled(format!(
                        "Action result: capmgr\nop: job_status\njob_id: {}\naction: {}\nstate: cancelled\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
                        record.id,
                        record.action,
                        started.elapsed().as_millis(),
                        record.output_file,
                        compact_text(&output, 2000)
                    ));
                }
                return ActionOutcome::background_finished(format!(
                    "Action result: capmgr\nop: job_status\njob_id: {}\naction: {}\nstate: finished\nexit_code: {}\nwaited_ms: {}\noutput_file: {}\noutput:\n{}",
                    record.id,
                    record.action,
                    code,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 4000)
                ));
            }
            if started.elapsed() >= wait {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return ActionOutcome::background_running(format!(
                    "Action result: capmgr\nop: job_status\njob_id: {}\naction: {}\nstate: running\npid: {}\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
                    record.id,
                    record.action,
                    record.pid,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 2000)
                ));
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    /// Terminates background command jobs launched by this process.
    ///
    /// The process-unique owner identity prevents a restarted Host from
    /// signalling historical records even if an operating-system PID is reused.
    pub fn terminate_owned_running(&self) -> usize {
        let owner_id = crate::runtime_process_owner_id();
        let records = self.records_unlocked();
        let mut terminated = 0;
        for record in records {
            if record.owner_id.as_deref() != Some(owner_id)
                || completed_status(&record.status_file).is_some()
            {
                continue;
            }
            terminate_process(record.pid);
            let _ = fs::write(&record.status_file, "cancelled");
            terminated += 1;
        }
        terminated
    }

    /// Terminates unfinished command-tool jobs owned by this process and Session.
    pub fn cancel_unfinished_for_session(&self, session_id: &str) -> Vec<String> {
        let clean_session = session_id.trim();
        if clean_session.is_empty() {
            return Vec::new();
        }
        let owner_id = crate::runtime_process_owner_id();
        let mut cancelled = Vec::new();
        for record in self.records_unlocked() {
            if record.owner_id.as_deref() != Some(owner_id)
                || record.session_id != clean_session
                || completed_status(&record.status_file).is_some()
            {
                continue;
            }
            terminate_process(record.pid);
            let _ = fs::write(&record.status_file, "cancelled");
            cancelled.push(record.id);
        }
        cancelled
    }

    pub fn cancel(&self, job_id: &str) -> String {
        self.cancel_outcome(job_id).text
    }

    pub(crate) fn cancel_outcome(&self, job_id: &str) -> ActionOutcome {
        let clean_id = job_id.trim();
        if clean_id.is_empty() {
            return ActionOutcome::failed(
                "Action result: capmgr\nop: job_cancel\nerror: job_id_required",
            );
        }
        let Some(record) = self.find(clean_id) else {
            return ActionOutcome::failed(format!(
                "Action result: capmgr\nop: job_cancel\njob_id: {}\nerror: job_not_found",
                clean_id
            ));
        };
        if let Some(code) = fs::read_to_string(&record.status_file)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
        {
            if code == "cancelled" {
                return ActionOutcome::cancelled(format!(
                    "Action result: capmgr\nop: job_cancel\njob_id: {}\naction: {}\nstate: cancelled\nstatus: already_completed",
                    record.id, record.action
                ));
            }
            return ActionOutcome::background_finished(format!(
                "Action result: capmgr\nop: job_cancel\njob_id: {}\naction: {}\nstate: finished\nstatus: already_completed",
                record.id, record.action
            ));
        }

        terminate_process(record.pid);
        let _ = fs::write(&record.status_file, "cancelled");
        let output = fs::read_to_string(&record.output_file).unwrap_or_default();
        ActionOutcome::cancelled(format!(
            "Action result: capmgr\nop: job_cancel\njob_id: {}\naction: {}\nstate: cancelled\npid: {}\noutput_file: {}\npartial_output:\n{}",
            record.id,
            record.action,
            record.pid,
            record.output_file,
            compact_text(&output, 2000)
        ))
    }

    fn append(&self, record: &ToolJobRecord) -> std::io::Result<()> {
        self.guard
            .with_write(|| self.append_unlocked(record))
            .map_err(std::io::Error::other)?
    }

    fn append_unlocked(&self, record: &ToolJobRecord) -> std::io::Result<()> {
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

    fn find(&self, job_id: &str) -> Option<ToolJobRecord> {
        self.find_unlocked(job_id)
    }

    fn find_unlocked(&self, job_id: &str) -> Option<ToolJobRecord> {
        self.records_unlocked()
            .into_iter()
            .rev()
            .find(|record| record.id == job_id)
    }

    fn records_unlocked(&self) -> Vec<ToolJobRecord> {
        let Ok(file) = OpenOptions::new().read(true).open(&self.index_file) else {
            return Vec::new();
        };
        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<ToolJobRecord>(&line).ok())
            .collect()
    }
}

fn completed_status(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn unique_job_id(prefix: &str) -> String {
    let seq = TOOL_JOB_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{}", now_ms(), seq)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn terminate_process(pid: u32) {
    crate::os::terminate_process(pid);
}

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>();
        out.push('…');
    }
    out
}

#[cfg(test)]
#[path = "../tests/unit/tool_jobs_tests.rs"]
mod tests;

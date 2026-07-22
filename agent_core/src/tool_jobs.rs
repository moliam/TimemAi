use crate::MemGuard;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

static TOOL_JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolJobRecord {
    pub id: String,
    pub created_at_ms: i64,
    pub pid: u32,
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
            guard: MemGuard::for_memory_dir(memory_dir),
        }
    }

    pub fn spawn(&self, action: &str, path: &Path, payload: &Value) -> String {
        let _ = fs::create_dir_all(&self.dir);
        let id = unique_job_id("tool_job");
        let payload_file = self.dir.join(format!("{id}.payload.json"));
        let output_file = self.dir.join(format!("{id}.out"));
        let status_file = self.dir.join(format!("{id}.status"));
        if let Err(err) = fs::write(&payload_file, payload.to_string()) {
            return format!(
                "Action result: {action}\nerror: background_payload_write_failed\nreason: {}",
                compact_text(&err.to_string(), 1000)
            );
        }

        let script = format!(
            "/bin/sh {} < {} > {} 2>&1; printf '%s' \"$?\" > {}",
            shell_quote_path(path),
            shell_quote_path(&payload_file),
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
            Err(err) => {
                return format!(
                    "Action result: {action}\nerror: background_spawn_failed\nreason: {}",
                    compact_text(&err.to_string(), 1000)
                )
            }
        };
        let record = ToolJobRecord {
            id: id.clone(),
            created_at_ms: now_ms(),
            pid: child.id(),
            action: action.to_string(),
            command_path: path.to_string_lossy().to_string(),
            payload_file: payload_file.to_string_lossy().to_string(),
            output_file: output_file.to_string_lossy().to_string(),
            status_file: status_file.to_string_lossy().to_string(),
        };
        let _ = self.append(&record);
        format!(
            "Action result: {action}\nstatus: background_started\njob_id: {}\npid: {}\noutput_file: {}\nstatus_file: {}\nnext_action: capmgr op=job_status",
            record.id, record.pid, record.output_file, record.status_file
        )
    }

    pub fn status(&self, job_id: &str, wait_ms: u64) -> String {
        let clean_id = job_id.trim();
        if clean_id.is_empty() {
            return "Action result: capmgr\nop: job_status\nerror: job_id_required".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: capmgr\nop: job_status\njob_id: {}\nerror: job_not_found",
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
                        "Action result: capmgr\nop: job_status\njob_id: {}\naction: {}\nstate: cancelled\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
                        record.id,
                        record.action,
                        started.elapsed().as_millis(),
                        record.output_file,
                        compact_text(&output, 2000)
                    );
                }
                return format!(
                    "Action result: capmgr\nop: job_status\njob_id: {}\naction: {}\nstate: finished\nexit_code: {}\nwaited_ms: {}\noutput_file: {}\noutput:\n{}",
                    record.id,
                    record.action,
                    code,
                    started.elapsed().as_millis(),
                    record.output_file,
                    compact_text(&output, 4000)
                );
            }
            if started.elapsed() >= wait {
                let output = fs::read_to_string(&record.output_file).unwrap_or_default();
                return format!(
                    "Action result: capmgr\nop: job_status\njob_id: {}\naction: {}\nstate: running\npid: {}\nwaited_ms: {}\noutput_file: {}\npartial_output:\n{}",
                    record.id,
                    record.action,
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
            return "Action result: capmgr\nop: job_cancel\nerror: job_id_required".to_string();
        }
        let Some(record) = self.find(clean_id) else {
            return format!(
                "Action result: capmgr\nop: job_cancel\njob_id: {}\nerror: job_not_found",
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
                "Action result: capmgr\nop: job_cancel\njob_id: {}\naction: {}\nstate: {}\nstatus: already_completed",
                record.id, record.action, state
            );
        }

        terminate_process(record.pid);
        let _ = fs::write(&record.status_file, "cancelled");
        let output = fs::read_to_string(&record.output_file).unwrap_or_default();
        format!(
            "Action result: capmgr\nop: job_cancel\njob_id: {}\naction: {}\nstate: cancelled\npid: {}\noutput_file: {}\npartial_output:\n{}",
            record.id,
            record.action,
            record.pid,
            record.output_file,
            compact_text(&output, 2000)
        )
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
        self.guard.with_read(|| self.find_unlocked(job_id)).ok()?
    }

    fn find_unlocked(&self, job_id: &str) -> Option<ToolJobRecord> {
        let file = OpenOptions::new().read(true).open(&self.index_file).ok()?;
        let mut found = None;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<ToolJobRecord>(&line) else {
                continue;
            };
            if record.id == job_id {
                found = Some(record);
            }
        }
        found
    }
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
    #[cfg(unix)]
    {
        terminate_process_unix(pid);
    }
    #[cfg(not(unix))]
    {
        let _ = Command::new("/bin/kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
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
        return;
    }
    signal_process(pid, libc::SIGTERM);
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

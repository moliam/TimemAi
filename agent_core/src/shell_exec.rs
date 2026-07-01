use crate::MemGuard;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static SHELL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellJobRecord {
    pub id: String,
    pub created_at_ms: i64,
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

    pub fn spawn(&self, command: &str) -> String {
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
        let spawn = Command::new("/bin/sh")
            .arg("-lc")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
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
        let file = OpenOptions::new().read(true).open(&self.index_file).ok()?;
        let mut found = None;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(record) = serde_json::from_str::<ShellJobRecord>(&line) else {
                continue;
            };
            if record.id == job_id {
                found = Some(record);
            }
        }
        found
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

pub fn execute_one_bash(command: &str, timeout_ms: u64) -> String {
    let spawn = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(_) => {
            return format!(
                "Action result: run_bash\ncommand: {}\nerror: command_failed",
                command
            )
        }
    };
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms.max(1000).min(15000));
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return format!(
                    "Action result: run_bash\ncommand: {}\nerror: timeout",
                    command
                );
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                return format!(
                    "Action result: run_bash\ncommand: {}\nerror: command_failed",
                    command
                )
            }
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
            format!(
                "Action result: run_bash\ncommand: {}\nstatus: {}\noutput:\n{}",
                command,
                output.status.code().unwrap_or(-1),
                compact_text(&combined, 4000)
            )
        }
        Err(_) => format!(
            "Action result: run_bash\ncommand: {}\nerror: command_failed",
            command
        ),
    }
}

fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn compact_text(text: &str, max_chars: usize) -> String {
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
        let result = execute_one_bash("printf shell_ok", 1000);
        assert!(result.contains("Action result: run_bash"));
        assert!(result.contains("status: 0"));
        assert!(result.contains("shell_ok"));
    }

    #[test]
    fn foreground_bash_timeout_is_bounded() {
        let result = execute_one_bash("sleep 2", 1000);
        assert!(result.contains("error: timeout"));
    }

    #[test]
    fn background_job_can_be_polled_until_finished() {
        let dir = tmp_memory_dir("background_job");
        let store = FileShellJobStore::new(&dir);
        let started = store.spawn("printf background_ok");
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

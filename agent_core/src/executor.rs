use crate::capability::CapabilityRegistry;
use crate::ActionOutcome;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const COMMAND_OUTPUT_CAPTURE_BYTES: usize = 64 * 1024;
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorTarget {
    Builtin {
        binding_name: String,
    },
    Command {
        action: String,
        path: PathBuf,
    },
    Mcp {
        server_id: String,
        tool_name: String,
    },
}

pub fn resolve_action(
    capabilities: &CapabilityRegistry,
    action: &str,
) -> Result<ExecutorTarget, String> {
    let Some(binding) = capabilities.binding(action) else {
        return Err(format!("{action}:unsupported_action"));
    };
    match binding.binding_type.as_str() {
        "builtin" => Ok(ExecutorTarget::Builtin {
            binding_name: binding.name.clone(),
        }),
        "command" => {
            let Some(path) = binding.command_path.clone() else {
                return Err(format!("{action}:command_binding_missing_path"));
            };
            Ok(ExecutorTarget::Command {
                action: action.to_string(),
                path,
            })
        }
        "mcp" => {
            let Some((server_id, tool_name)) = binding.name.split_once("::") else {
                return Err(format!("{action}:mcp_binding_invalid"));
            };
            Ok(ExecutorTarget::Mcp {
                server_id: server_id.to_string(),
                tool_name: tool_name.to_string(),
            })
        }
        other => Err(format!("{action}:unsupported_binding_type:{other}")),
    }
}

pub fn execute_command_action(
    action: &str,
    path: &Path,
    payload: &Value,
    timeout_ms: u64,
) -> String {
    execute_command_action_outcome(action, path, payload, timeout_ms).text
}

pub(crate) fn execute_command_action_outcome(
    action: &str,
    path: &Path,
    payload: &Value,
    timeout_ms: u64,
) -> ActionOutcome {
    let mut command = Command::new("/bin/sh");
    command
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_spawn_failed\nreason: {}",
                compact_text(&err.to_string(), 1000)
            ))
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let stdout_reader = child
        .stdout
        .take()
        .map(|stdout| spawn_bounded_reader(stdout, COMMAND_OUTPUT_CAPTURE_BYTES));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stderr| spawn_bounded_reader(stderr, COMMAND_OUTPUT_CAPTURE_BYTES));
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms.clamp(1000, 15000));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_command_process(&mut child);
                let _ = join_bounded_reader(stdout_reader);
                let _ = join_bounded_reader(stderr_reader);
                return ActionOutcome::timeout(format!("Action result: {action}\nerror: timeout"));
            }
            Ok(None) => thread::sleep(COMMAND_POLL_INTERVAL),
            Err(err) => {
                terminate_command_process(&mut child);
                let _ = join_bounded_reader(stdout_reader);
                let _ = join_bounded_reader(stderr_reader);
                return ActionOutcome::failed(format!(
                    "Action result: {action}\nerror: command_wait_failed\nreason: {}",
                    compact_text(&err.to_string(), 1000)
                ));
            }
        }
    };
    let stdout = match join_bounded_reader(stdout_reader) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_output_failed\nreason: {error}"
            ))
        }
    };
    let stderr = match join_bounded_reader(stderr_reader) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_output_failed\nreason: {error}"
            ))
        }
    };
    render_command_output(action, status, &stdout, &stderr)
}

fn spawn_bounded_reader(
    mut reader: impl Read + Send + 'static,
    max_bytes: usize,
) -> thread::JoinHandle<std::io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut captured = Vec::with_capacity(max_bytes.min(8192));
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(captured);
            }
            let remaining = max_bytes.saturating_sub(captured.len());
            if remaining > 0 {
                captured.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
    })
}

fn join_bounded_reader(
    reader: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
) -> Result<Vec<u8>, String> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    reader
        .join()
        .map_err(|_| "command_output_reader_panicked".to_string())?
        .map_err(|error| compact_text(&error.to_string(), 1000))
}

fn render_command_output(
    action: &str,
    status: ExitStatus,
    stdout: &str,
    stderr: &str,
) -> ActionOutcome {
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
    if let Some(signal) = exit_signal(&status) {
        return ActionOutcome::failed(format!(
            "Action result: {action}\nerror: terminated_by_signal\nsignal: {signal}\noutput:\n{}",
            compact_text(&combined, 4000)
        ));
    }
    let code = status.code().unwrap_or(-1);
    let text = format!(
        "Action result: {action}\nstatus: {code}\noutput:\n{}",
        compact_text(&combined, 4000)
    );
    if code == 0 {
        ActionOutcome::completed(text)
    } else {
        ActionOutcome::failed(text)
    }
}

fn terminate_command_process(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        let pid = child.id() as libc::pid_t;
        if pid > 1 && pid != libc::getpgrp() {
            let _ = libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
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

fn compact_text(text: &str, max_chars: usize) -> String {
    let mut out = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>();
        out.push('…');
    }
    out
}

#[cfg(test)]
#[path = "../tests/unit/executor_tests.rs"]
mod tests;

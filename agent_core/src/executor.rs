use crate::capability::CapabilityRegistry;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorTarget {
    Builtin { binding_name: String },
    Command { action: String, path: PathBuf },
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
        other => Err(format!("{action}:unsupported_binding_type:{other}")),
    }
}

pub fn execute_command_action(
    action: &str,
    path: &Path,
    payload: &Value,
    timeout_ms: u64,
) -> String {
    let mut child = match Command::new("/bin/sh")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return format!(
                "Action result: {action}\nerror: command_spawn_failed\nreason: {}",
                compact_text(&err.to_string(), 1000)
            )
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms.clamp(1000, 15000));
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return format!("Action result: {action}\nerror: timeout");
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(err) => {
                return format!(
                    "Action result: {action}\nerror: command_wait_failed\nreason: {}",
                    compact_text(&err.to_string(), 1000)
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
            if let Some(signal) = exit_signal(&output.status) {
                format!(
                    "Action result: {action}\nerror: terminated_by_signal\nsignal: {signal}\noutput:\n{}",
                    compact_text(&combined, 4000)
                )
            } else {
                format!(
                    "Action result: {action}\nstatus: {}\noutput:\n{}",
                    output.status.code().unwrap_or(-1),
                    compact_text(&combined, 4000)
                )
            }
        }
        Err(err) => format!(
            "Action result: {action}\nerror: command_output_failed\nreason: {}",
            compact_text(&err.to_string(), 1000)
        ),
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

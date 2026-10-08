use crate::capability::CapabilityRegistry;
use crate::{ActionOutcome, BashResultEvidence};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    execute_command_action_outcome_with_process_job(
        action,
        path,
        payload,
        timeout_ms,
        crate::os::ManagedProcessJob::create(),
    )
}

fn configured_command_action(path: &Path) -> Result<std::process::Command, String> {
    let mut command = crate::os::command_for_script(path)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::os::configure_child_process_group(&mut command);
    Ok(command)
}

fn execute_command_action_outcome_with_process_job(
    action: &str,
    path: &Path,
    payload: &Value,
    timeout_ms: u64,
    process_job_result: std::io::Result<crate::os::ManagedProcessJob>,
) -> ActionOutcome {
    let mut command = match configured_command_action(path) {
        Ok(command) => command,
        Err(error) => {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_interpreter_unavailable\nreason: {error}"
            ))
        }
    };
    // Exact per-job ownership is optional. Unsupported, undelegated, or
    // transiently unavailable backends must never block command execution.
    let mut process_job = process_job_result.ok();
    if process_job
        .as_ref()
        .is_some_and(|job| job.configure_command(&mut command).is_err())
    {
        process_job = None;
        command = match configured_command_action(path) {
            Ok(command) => command,
            Err(error) => {
                return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_interpreter_unavailable\nreason: {error}"
            ))
            }
        };
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) if process_job.is_some() => {
            // A native backend may disappear between setup and pre-exec. The
            // failed pre-exec means user code did not run, so retry once with a
            // fresh command using the process-group fallback.
            process_job = None;
            match configured_command_action(path)
                .and_then(|mut command| command.spawn().map_err(|error| error.to_string()))
            {
                Ok(child) => child,
                Err(error) => {
                    return ActionOutcome::failed(format!(
                        "Action result: {action}\nerror: command_spawn_failed\nreason: {}",
                        compact_text(&error, 1000)
                    ))
                }
            }
        }
        Err(error) => {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_spawn_failed\nreason: {}",
                compact_text(&error.to_string(), 1000)
            ))
        }
    };
    let _child_registration = crate::os::register_managed_child(child.id());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let output =
        crate::command_output::CommandOutput::start(&mut child, COMMAND_OUTPUT_CAPTURE_BYTES);
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms.clamp(1000, 15000));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_command_process(&mut child, process_job.as_ref());
                let _ = output.finish();
                return ActionOutcome::timeout(format!("Action result: {action}\nerror: timeout"));
            }
            Ok(None) => thread::sleep(COMMAND_POLL_INTERVAL),
            Err(err) => {
                terminate_command_process(&mut child, process_job.as_ref());
                let _ = output.finish();
                return ActionOutcome::failed(format!(
                    "Action result: {action}\nerror: command_wait_failed\nreason: {}",
                    compact_text(&err.to_string(), 1000)
                ));
            }
        }
    };
    // Command capabilities are finite executions: descendants are not allowed
    // to outlive the registered tool process, even when the leader exits 0.
    // Kill residual members before joining pipe drains because a descendant may
    // still hold stdout/stderr open.
    terminate_command_descendants(child.id(), process_job.as_ref());
    let (stdout, stderr) = match output.finish() {
        Ok(output) => output,
        Err(error) => {
            return ActionOutcome::failed(format!(
                "Action result: {action}\nerror: command_output_failed\nreason: {error}"
            ))
        }
    };
    render_command_output(
        action,
        status,
        &String::from_utf8_lossy(&stdout),
        &String::from_utf8_lossy(&stderr),
    )
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
        ))
        .with_bash_result(BashResultEvidence {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            stdout_truncation: None,
            stderr_truncation: None,
            exit_code: None,
            signal: Some(signal),
            pid: None,
            timed_out: false,
            pid_kind: None,
            error_type: Some("TerminatedBySignal".to_string()),
        });
    }
    let code = status.code().unwrap_or(-1);
    let text = format!(
        "Action result: {action}\nstatus: {code}\noutput:\n{}",
        compact_text(&combined, 4000)
    );
    let outcome = if code == 0 {
        ActionOutcome::completed(text)
    } else {
        ActionOutcome::failed(text)
    };
    outcome.with_bash_result(BashResultEvidence {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        stdout_truncation: None,
        stderr_truncation: None,
        exit_code: Some(code),
        signal: None,
        pid: None,
        timed_out: false,
        pid_kind: None,
        error_type: None,
    })
}

fn terminate_command_process(
    child: &mut std::process::Child,
    process_job: Option<&crate::os::ManagedProcessJob>,
) {
    terminate_command_descendants(child.id(), process_job);
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_command_descendants(
    leader_pid: u32,
    process_job: Option<&crate::os::ManagedProcessJob>,
) {
    if process_job.is_none_or(|process_job| process_job.kill_all().is_err()) {
        // Explicit degraded mode, or best-effort fallback if the native Job backend
        // control file becomes unavailable after spawn. This reaches only
        // descendants that did not escape the process group.
        crate::os::kill_process_group(leader_pid);
    }
}

fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    crate::os::exit_signal(status)
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

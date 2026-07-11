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
    let timeout = Duration::from_millis(timeout_ms.max(1000).min(15000));
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
            format!(
                "Action result: {action}\nstatus: {}\noutput:\n{}",
                output.status.code().unwrap_or(-1),
                compact_text(&combined, 4000)
            )
        }
        Err(err) => format!(
            "Action result: {action}\nerror: command_output_failed\nreason: {}",
            compact_text(&err.to_string(), 1000)
        ),
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
mod tests {
    use super::*;
    use crate::capability::CapabilityRegistry;
    use serde_json::json;
    use std::fs;

    #[test]
    fn builtin_manifest_action_resolves_to_builtin_binding() {
        let registry = CapabilityRegistry::builtin();

        assert_eq!(
            resolve_action(&registry, "memmgr").unwrap(),
            ExecutorTarget::Builtin {
                binding_name: "memmgr".to_string()
            }
        );
        assert_eq!(
            resolve_action(&registry, "capmgr").unwrap(),
            ExecutorTarget::Builtin {
                binding_name: "capmgr".to_string()
            }
        );
        assert_eq!(
            resolve_action(&registry, "self_tool").unwrap(),
            ExecutorTarget::Builtin {
                binding_name: "self_tool".to_string()
            }
        );
    }

    #[test]
    fn action_outside_manifest_is_rejected() {
        let registry = CapabilityRegistry::builtin();

        assert_eq!(
            resolve_action(&registry, "query_memory").unwrap_err(),
            "query_memory:unsupported_action"
        );
    }

    #[test]
    fn overlay_command_manifest_resolves_to_command_path() {
        let dir =
            std::env::temp_dir().join(format!("timem_executor_overlay_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("tools")).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::write(dir.join("bin/local_echo.sh"), "#!/bin/sh\ncat\n").unwrap();
        fs::write(
            dir.join("tools/local_echo.yaml"),
            r#"kind: tool
id: local_echo
binding_type: command
binding_name: bin/local_echo.sh
summary: Local echo command.
description: |
  Echo local input for tests.
input_properties:
  message?: string
example_json: |
  {
    "action": "local_echo",
    "args": {
      "message": "hello"
    }
  }
"#,
        )
        .unwrap();

        let registry = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap();
        assert_eq!(
            resolve_action(&registry, "local_echo").unwrap(),
            ExecutorTarget::Command {
                action: "local_echo".to_string(),
                path: dir.join("bin/local_echo.sh")
            }
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_action_receives_json_payload_on_stdin() {
        let dir = temp_case_dir("command_payload");
        fs::write(
            dir.join("echo_payload.sh"),
            "#!/bin/sh\npython3 -c 'import sys,json; data=json.load(sys.stdin); print(data[\"args\"][\"message\"])'\n",
        )
        .unwrap();

        let result = execute_command_action(
            "local_echo",
            &dir.join("echo_payload.sh"),
            &json!({"args":{"message":"hello from payload"}}),
            1000,
        );

        assert!(result.contains("Action result: local_echo"));
        assert!(result.contains("status: 0"));
        assert!(result.contains("hello from payload"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_action_merges_stderr_with_output() {
        let dir = temp_case_dir("command_stderr");
        fs::write(
            dir.join("stderr.sh"),
            "#!/bin/sh\nprintf out\nprintf err >&2\nexit 3\n",
        )
        .unwrap();

        let result = execute_command_action("local_tool", &dir.join("stderr.sh"), &json!({}), 1000);

        assert!(result.contains("status: 3"));
        assert!(result.contains("out"));
        assert!(result.contains("stderr: err"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_action_timeout_is_bounded() {
        let dir = temp_case_dir("command_timeout");
        fs::write(dir.join("slow.sh"), "#!/bin/sh\nsleep 2\n").unwrap();

        let result = execute_command_action("slow_tool", &dir.join("slow.sh"), &json!({}), 1000);

        assert!(result.contains("error: timeout"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_case_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "timem_executor_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}

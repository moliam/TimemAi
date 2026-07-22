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
    let dir = std::env::temp_dir().join(format!("timem_executor_overlay_{}", std::process::id()));
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
            "#!/bin/sh\npayload=$(cat)\ncase \"$payload\" in\n  *'\"message\":\"hello from payload\"'*) printf '%s\\n' 'hello from payload' ;;\n  *) printf 'unexpected payload: %s\\n' \"$payload\"; exit 7 ;;\nesac\n",
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

#[cfg(unix)]
#[test]
fn command_action_contains_script_sigsegv_and_executor_remains_usable() {
    let dir = temp_case_dir("command_sigsegv");
    fs::write(dir.join("crash.sh"), "#!/bin/sh\nkill -SEGV $$\n").unwrap();
    fs::write(dir.join("ok.sh"), "#!/bin/sh\nprintf still_alive\n").unwrap();

    let crashed = execute_command_action("crash_tool", &dir.join("crash.sh"), &json!({}), 1000);
    assert!(crashed.contains("error: terminated_by_signal"), "{crashed}");
    assert!(crashed.contains("signal: 11"), "{crashed}");

    let follow_up = execute_command_action("ok_tool", &dir.join("ok.sh"), &json!({}), 1000);
    assert!(follow_up.contains("status: 0"), "{follow_up}");
    assert!(follow_up.contains("still_alive"), "{follow_up}");
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

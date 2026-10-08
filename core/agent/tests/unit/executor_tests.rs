use super::*;
use crate::capability::CapabilityRegistry;
use serde_json::json;
use std::fs;

#[test]
fn builtin_manifest_action_resolves_to_builtin_binding() {
    let registry = CapabilityRegistry::builtin_for_host(
        crate::capability::CapabilityHostProfile::with_local_command_execution(),
    );

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
    let registry = CapabilityRegistry::builtin_for_host(
        crate::capability::CapabilityHostProfile::with_local_command_execution(),
    );

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

#[cfg(unix)]
#[test]
fn unavailable_exact_process_backend_never_blocks_command_action() {
    let dir = temp_case_dir("command_backend_fallback");
    let script = dir.join("fallback.sh");
    fs::write(&script, "#!/bin/sh\nprintf fallback_ok\n").unwrap();

    for kind in [
        std::io::ErrorKind::Unsupported,
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::Other,
    ] {
        let outcome = execute_command_action_outcome_with_process_job(
            "fallback_tool",
            &script,
            &json!({}),
            1000,
            Err(std::io::Error::new(kind, "optional backend unavailable")),
        );
        assert_eq!(outcome.status, crate::ActionStatus::Completed, "{kind:?}");
        assert!(
            outcome.text.contains("fallback_ok"),
            "{kind:?}: {}",
            outcome.text
        );
        assert!(!outcome.text.contains("containment"), "{}", outcome.text);
        assert!(!outcome.text.contains("cgroup"), "{}", outcome.text);
        assert!(
            !outcome.text.contains("optional backend"),
            "{}",
            outcome.text
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
#[test]
fn command_action_drains_large_output_without_pipe_deadlock() {
    let dir = temp_case_dir("command_large_output");
    fs::write(
        dir.join("large_output.sh"),
        "#!/bin/sh\npython3 - <<'PY'\nprint('x' * 262144)\nPY\n",
    )
    .unwrap();

    let result = execute_command_action(
        "large_output_tool",
        &dir.join("large_output.sh"),
        &json!({}),
        2000,
    );

    assert!(result.contains("status: 0"), "{result}");
    assert!(result.contains("xxxx"), "{result}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn command_action_timeout_terminates_descendant_process_group() {
    let dir = temp_case_dir("command_timeout_descendants");
    let pid_file = dir.join("child.pid");
    fs::write(
        dir.join("descendant.sh"),
        format!(
            "#!/bin/sh\nsleep 30 &\nprintf '%s' \"$!\" > '{}'\nwait\n",
            pid_file.display()
        ),
    )
    .unwrap();

    let result = execute_command_action("slow_tree", &dir.join("descendant.sh"), &json!({}), 1000);
    assert!(result.contains("error: timeout"), "{result}");

    let child_pid: i32 = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // `kill(pid, 0)` also succeeds for zombies: the test process is the
    // installed subreaper, so the SIGKILLed descendant may linger as an
    // unreaped zombie child. A zombie state means the kill worked.
    //
    // Nested-runtime note (Timem developing Timem): when this test runs
    // under an outer Timem instance, the subreaper that adopts the orphaned
    // descendant can be the OUTER runtime process, not this test binary. The
    // zombie is then reparented outside our process tree and never reaped by
    // us, so `kill(pid, 0)` stays "alive" forever at every inner level.
    // Judging by /proc/<pid>/stat state (Z) instead of signal probing keeps
    // this assertion correct for any level of nesting.
    let zombie_or_gone = |pid: i32| match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .unwrap_or("")
            .trim_start()
            .starts_with('Z'),
        Err(_) => true,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while unsafe { libc::kill(child_pid, 0) } == 0
        && !zombie_or_gone(child_pid)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        unsafe { libc::kill(child_pid, 0) } != 0 || zombie_or_gone(child_pid),
        "descendant process {child_pid} survived command timeout"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
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

#[cfg(windows)]
#[test]
fn windows_command_action_merges_stderr_with_output() {
    let dir = temp_case_dir("windows_command_stderr");
    let script = dir.join("stderr.ps1");
    fs::write(
        &script,
        "[Console]::Out.Write('out')\n[Console]::Error.Write('err')\nexit 3\n",
    )
    .unwrap();

    let result = execute_command_action("local_tool", &script, &json!({}), 1000);

    assert!(result.contains("status: 3"), "{result}");
    assert!(result.contains("out"), "{result}");
    assert!(result.contains("stderr: err"), "{result}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_command_action_drains_large_output_without_pipe_deadlock() {
    let dir = temp_case_dir("windows_command_large_output");
    let script = dir.join("large_output.ps1");
    fs::write(&script, "[Console]::Out.Write(('x' * 262144))\n").unwrap();

    let result = execute_command_action("large_output_tool", &script, &json!({}), 5000);

    assert!(result.contains("status: 0"), "{result}");
    assert!(result.contains("xxxx"), "{result}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_command_action_timeout_is_bounded() {
    let dir = temp_case_dir("windows_command_timeout");
    let script = dir.join("slow.ps1");
    fs::write(&script, "Start-Sleep -Seconds 2\n").unwrap();

    let result = execute_command_action("slow_tool", &script, &json!({}), 1000);

    assert!(result.contains("error: timeout"), "{result}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_powershell_command_action_receives_json_payload() {
    let dir = temp_case_dir("windows_powershell_payload");
    let script = dir.join("echo_payload.ps1");
    fs::write(
        &script,
        "$data = ($input | Out-String) | ConvertFrom-Json\nWrite-Output $data.args.message\n",
    )
    .unwrap();

    let result = execute_command_action(
        "local_echo",
        &script,
        &json!({"args":{"message":"windows payload ok"}}),
        5000,
    );

    assert!(result.contains("status: 0"), "{result}");
    assert!(result.contains("windows payload ok"), "{result}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_command_action_rejects_unknown_script_extension() {
    let dir = temp_case_dir("windows_unknown_script");
    let script = dir.join("unknown.py");
    fs::write(&script, "print('not executed')\n").unwrap();

    let result = execute_command_action("unknown", &script, &json!({}), 1000);

    assert!(
        result.contains("error: command_interpreter_unavailable"),
        "{result}"
    );
    assert!(
        result.contains("unsupported_windows_command_extension:py"),
        "{result}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn command_action_timeout_kills_its_setsid_escapee_with_job_ownership() {
    let dir = temp_case_dir("command_timeout_setsid_owned");
    let pid_file = dir.join("escapee.pid");
    fs::write(
        dir.join("escape.sh"),
        format!(
            "#!/bin/sh\nsetsid --fork sh -c 'echo $$ > \"{}\"; sleep 60' >/dev/null 2>&1\nsleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();

    let result = execute_command_action("escape_tool", &dir.join("escape.sh"), &json!({}), 1000);
    assert!(result.contains("error: timeout"), "{result}");

    let escapee_pid: u32 = fs::read_to_string(&pid_file)
        .expect("escapee PID marker")
        .trim()
        .parse()
        .expect("numeric escapee PID");
    let still_executing = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| stat.rsplit_once(") ").map(|(_, tail)| tail.to_string()))
            .and_then(|tail| tail.split_whitespace().next().map(str::to_string))
            .is_some_and(|state| state != "Z" && state != "X")
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while still_executing(escapee_pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !still_executing(escapee_pid),
        "owned setsid escapee {escapee_pid} remained executable after command timeout"
    );
    let _ = fs::remove_dir_all(dir);
}

#[cfg(target_os = "macos")]
#[test]
fn command_action_escaped_pipe_reports_incomplete_without_unbounded_join() {
    let dir = temp_case_dir("escaped_pipe_capture");
    let script = dir.join("escape.sh");
    let fixture = crate::escaped_pipe_fixture::NativeEscapedPipeFixture::new(&dir);
    fs::write(&script, format!("#!/bin/sh\n{}\n", fixture.command(false))).unwrap();
    let started = Instant::now();
    let outcome = execute_command_action_outcome("escape_tool", &script, &json!({}), 1000);
    let elapsed = started.elapsed();
    let phase = fs::read_to_string(fixture.path().join("phase"));
    drop(fixture);
    let _ = fs::remove_dir_all(dir);
    assert!(elapsed < Duration::from_secs(2), "capture took {elapsed:?}");
    assert_eq!(
        outcome.status,
        crate::ActionStatus::Failed,
        "phase={phase:?}; {}",
        outcome.text
    );
    assert!(
        outcome.text.contains("output_capture_incomplete"),
        "{}",
        outcome.text
    );
}

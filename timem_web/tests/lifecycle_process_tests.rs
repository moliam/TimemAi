#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn temporary_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "timem-web-process-diagnostics-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ))
}
fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("condition did not become ready within {timeout:?}");
}
fn try_read_json(path: &Path) -> Option<serde_json::Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}
fn read_json(path: &Path) -> serde_json::Value {
    try_read_json(path).unwrap_or_else(|| panic!("missing or invalid JSON: {}", path.display()))
}
fn diagnostics_root(data: &Path) -> PathBuf {
    data.join("diagnostics/timem-web")
}
fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
fn spawn_running_web(root: &Path) -> (ChildGuard, PathBuf) {
    let data = root.join("data");
    let mem = root.join("mem");
    fs::create_dir_all(&mem).unwrap();
    let output = fs::File::create(root.join("output.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_timem-web"))
        .args([
            "--no-open",
            "--data-dir",
            data.to_str().unwrap(),
            "--space",
            mem.to_str().unwrap(),
        ])
        .stdout(Stdio::from(output.try_clone().unwrap()))
        .stderr(Stdio::from(output))
        .spawn()
        .unwrap();
    let child = ChildGuard(child);
    let diagnostics = diagnostics_root(&data);
    let current = diagnostics.join("current-run.json");
    wait_until(WAIT_TIMEOUT, || {
        try_read_json(&current)
            .and_then(|value| value["recent_lifecycle_events"].as_array().cloned())
            .is_some_and(|events| events.iter().any(|event| event["name"] == "listener_bound"))
    });
    (child, diagnostics)
}
fn assert_signal_exit(signal: i32, expected_reason: &str) {
    let root = temporary_root(expected_reason);
    let (mut child, diagnostics) = spawn_running_web(&root);
    let current = diagnostics.join("current-run.json");
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, signal) }, 0);
    wait_until(WAIT_TIMEOUT, || child.0.try_wait().ok().flatten().is_some());
    let exit = read_json(&diagnostics.join("last-exit.json"));
    assert_eq!(exit["exit_reason"], expected_reason);
    assert_eq!(exit["graceful"], true);
    assert!(!current.exists());
    let events = exit["recent_lifecycle_events"].as_array().unwrap();
    assert!(events
        .iter()
        .any(|event| event["name"] == "shutdown_trigger_received"
            && event["details"]["reason"] == expected_reason));
    assert_eq!(
        events.last().unwrap()["name"],
        "graceful_shutdown_completed"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_process_records_sigterm_sighup_and_sigint_exit_reasons() {
    assert_signal_exit(libc::SIGTERM, "sigterm");
    assert_signal_exit(libc::SIGHUP, "sighup");
    assert_signal_exit(libc::SIGINT, "ctrl_c");
}

#[test]
fn injected_rust_panic_writes_redacted_report_and_preserves_running_marker() {
    let root = temporary_root("panic");
    let data = root.join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_timem-web"))
        .args(["--help", "--data-dir", data.to_str().unwrap()])
        .env("TIMEM_WEB_TEST_FAULT", "panic_after_diagnostics_install")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostics = diagnostics_root(&data);
    let report = fs::read_to_string(diagnostics.join("last-panic.txt")).unwrap();
    assert!(report.contains("TIMEM WEB PANIC REPORT"));
    assert!(report.contains("test_fault_injected"));
    assert!(report.contains("BACKTRACE"));
    assert!(report.contains("[REDACTED]"));
    assert!(!report.contains("test-private-token"));
    assert!(diagnostics.join("current-run.json").is_file());
    assert!(!diagnostics.join("last-exit.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sigkill_residue_is_promoted_by_the_next_start_without_guessing_cause() {
    let root = temporary_root("sigkill");
    let data = root.join("data");
    let (mut child, diagnostics) = spawn_running_web(&root);
    let killed_pid = child.0.id();
    assert_eq!(unsafe { libc::kill(killed_pid as i32, libc::SIGKILL) }, 0);
    wait_until(WAIT_TIMEOUT, || child.0.try_wait().ok().flatten().is_some());
    assert!(diagnostics.join("current-run.json").is_file());
    assert!(!diagnostics.join("last-exit.json").exists());
    let restart = Command::new(env!("CARGO_BIN_EXE_timem-web"))
        .args(["--help", "--data-dir", data.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(restart.status.success(), "{}", output_text(&restart));
    let previous = read_json(&diagnostics.join("previous-abnormal-exit.json"));
    assert_eq!(previous["status"], "abnormal_exit_detected_on_next_start");
    assert_eq!(previous["exact_cause"], "unknown");
    assert_eq!(previous["pid"], killed_pid);
    assert!(!diagnostics.join("current-run.json").exists());
    assert_eq!(
        read_json(&diagnostics.join("last-exit.json"))["exit_reason"],
        "help_requested"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_configuration_failure_records_bounded_error_without_secret_values() {
    let root = temporary_root("startup-error");
    let data = root.join("data");
    let secret = "sk-process-test-private";
    let output = Command::new(env!("CARGO_BIN_EXE_timem-web"))
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "--api-key",
            secret,
            "--port",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let diagnostics = diagnostics_root(&data);
    let exit_text = fs::read_to_string(diagnostics.join("last-exit.json")).unwrap();
    let exit: serde_json::Value = serde_json::from_str(&exit_text).unwrap();
    assert_eq!(exit["exit_reason"], "startup_or_runtime_error");
    assert_eq!(exit["graceful"], false);
    assert!(exit["error"]
        .as_str()
        .unwrap()
        .contains("port_out_of_range"));
    assert!(!exit_text.contains(secret));
    assert!(!output_text(&output).contains(secret));
    assert!(!diagnostics.join("current-run.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_diagnostics_degrades_without_blocking_help() {
    let root = temporary_root("diagnostics-unavailable");
    fs::create_dir_all(&root).unwrap();
    let unusable_data_root = root.join("data-is-a-file");
    fs::write(&unusable_data_root, b"not a directory").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_timem-web"))
        .args(["--help", "--data-dir", unusable_data_root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Timem Web"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("[timem_web_diagnostics_unavailable]"));
    assert!(unusable_data_root.is_file());
    fs::remove_dir_all(root).unwrap();
}

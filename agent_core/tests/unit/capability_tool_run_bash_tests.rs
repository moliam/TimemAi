use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct NeverCancelRuntime;

impl ActionRuntime for NeverCancelRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }
}

struct ToggleCancelRuntime<'a> {
    cancelled: &'a AtomicBool,
}

impl ActionRuntime for ToggleCancelRuntime<'_> {
    fn should_cancel(&mut self) -> bool {
        let previous = self.cancelled.swap(true, Ordering::Relaxed);
        previous
    }
}

#[derive(Default)]
struct CancelAfterLongRunningPromptRuntime {
    prompts: Vec<LongRunningCommandStatus>,
}

impl ActionRuntime for CancelAfterLongRunningPromptRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }

    fn on_long_running_command(
        &mut self,
        status: &LongRunningCommandStatus,
    ) -> LongRunningCommandDecision {
        self.prompts.push(status.clone());
        LongRunningCommandDecision::Cancel
    }
}

fn tmp_memory_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "timem_shell_exec_test_{}_{}",
        name,
        unique_shell_id("case")
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn tmp_cwd(name: &str) -> PathBuf {
    tmp_memory_dir(&format!("cwd_{name}"))
}

#[test]
fn normal_bash_reports_status_and_output() {
    let mut runtime = NeverCancelRuntime;
    let result = execute_one_bash("printf shell_ok", 1000, &mut runtime);
    assert!(result.contains("Action result: run_bash"));
    assert!(result.contains("Exit code: 0"));
    assert!(result.contains("shell_ok"));
}

#[cfg(unix)]
#[test]
fn normal_bash_contains_child_sigsegv_and_accepts_follow_up_command() {
    let mut runtime = NeverCancelRuntime;
    let crashed = execute_one_bash("kill -SEGV $$", 1000, &mut runtime);
    assert!(crashed.contains("process signal"), "{crashed}");
    assert!(crashed.contains("Signal: 11"), "{crashed}");

    let follow_up = execute_one_bash("printf still_alive", 1000, &mut runtime);
    assert!(follow_up.contains("Exit code: 0"), "{follow_up}");
    assert!(follow_up.contains("still_alive"), "{follow_up}");
}

#[test]
fn normal_bash_timeout_is_bounded() {
    let mut runtime = NeverCancelRuntime;
    let result = execute_one_bash("sleep 2", 1000, &mut runtime);
    assert!(result.contains("Timem stopped waiting"), "{result}");
    assert!(
        result.contains("does not by itself mean the process was killed"),
        "{result}"
    );
}

#[test]
fn normal_bash_rejects_non_positive_timeout() {
    let mut runtime = NeverCancelRuntime;
    let marker = tmp_memory_dir("invalid_timeout").join("marker.txt");
    let command = format!("printf should_not_run > {}", marker.display());
    let result = execute_one_bash(&command, -1, &mut runtime);
    assert!(
        result.contains("timeout_ms must be a positive integer"),
        "{result}"
    );
    assert!(!result.contains("Exit code: 0"), "{result}");
    assert!(!marker.exists(), "{result}");
}

#[test]
fn normal_bash_positive_timeout_reports_long_running_status_to_runtime() {
    let _guard = set_long_running_command_prompt_after_for_tests(Duration::from_millis(50));
    let mut runtime = CancelAfterLongRunningPromptRuntime::default();
    let result = execute_one_bash("sleep 2; printf should_not_finish", 5000, &mut runtime);

    assert!(result.contains("cancelled before it completed"), "{result}");
    assert_eq!(runtime.prompts.len(), 1);
    assert_eq!(runtime.prompts[0].action, "run_bash");
    assert_eq!(
        runtime.prompts[0].command,
        "sleep 2; printf should_not_finish"
    );
    assert_eq!(runtime.prompts[0].timeout_ms, Some(5000));
    assert!(runtime.prompts[0].elapsed >= Duration::from_millis(50));
}

#[test]
fn normal_run_bash_rejects_long_sleep_commands() {
    let store = FileShellJobStore::new(&tmp_memory_dir("long_sleep_guard"));
    let cwd = tmp_cwd("long_sleep_guard");
    let result = execute_run_bash(
        "sleep 90 && printf done",
        &cwd,
        false,
        5000,
        None,
        5000,
        BashApprovalMode::Approve,
        &store,
        "session_a",
        "turn_a",
        true,
        &mut NeverCancelRuntime,
    );
    match result {
        ActionExecution::Completed(text) => {
            assert!(text.contains("long sleep in normal mode"), "{text}");
            assert!(text.contains("interval_ms"));
        }
        ActionExecution::NeedsApproval(_) => {
            panic!("long sleep should be rejected before approval")
        }
    }
}

#[test]
fn normal_run_bash_allows_short_sleep_commands() {
    let store = FileShellJobStore::new(&tmp_memory_dir("short_sleep_guard"));
    let cwd = tmp_cwd("short_sleep_guard");
    let result = execute_run_bash(
        "sleep 1; printf done",
        &cwd,
        false,
        3000,
        None,
        5000,
        BashApprovalMode::Approve,
        &store,
        "session_a",
        "turn_a",
        true,
        &mut NeverCancelRuntime,
    );
    match result {
        ActionExecution::Completed(text) => {
            assert!(text.contains("Exit code: 0"));
            assert!(text.contains("done"));
        }
        ActionExecution::NeedsApproval(_) => panic!("approve mode should not request approval"),
    }
}

#[test]
fn run_bash_poll_mode_finishes_when_command_exits_zero() {
    let dir = tmp_memory_dir("poll_success");
    let marker = dir.join("ready.flag");
    let command = format!(
        "test -f {} || (touch {}; exit 1)",
        shell_quote_path(&marker),
        shell_quote_path(&marker)
    );
    let mut runtime = NeverCancelRuntime;
    let result = execute_polling_bash(&command, &dir, 1000, 5000, 1000, &mut runtime);
    assert!(result.contains("Action result: run_bash"), "{result}");
    assert!(result.contains("Polling state: finished"), "{result}");
    assert!(result.contains("Attempts: 2"), "{result}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn run_bash_poll_mode_times_out_when_command_stays_nonzero() {
    let mut runtime = NeverCancelRuntime;
    let cwd = tmp_cwd("poll_timeout");
    let result = execute_polling_bash(
        "printf waiting; exit 7",
        &cwd,
        1000,
        1100,
        1000,
        &mut runtime,
    );
    assert!(result.contains("Polling state: timeout"), "{result}");
    assert!(result.contains("Last observed exit code: 7"), "{result}");
    assert!(result.contains("waiting"), "{result}");
}

#[test]
fn run_bash_poll_mode_can_be_cancelled_during_wait() {
    let cancelled = AtomicBool::new(false);
    let mut runtime = ToggleCancelRuntime {
        cancelled: &cancelled,
    };
    let cwd = tmp_cwd("poll_cancel");
    let result = execute_polling_bash("exit 1", &cwd, 1000, 10_000, 1000, &mut runtime);
    assert!(result.contains("Polling state: cancelled"), "{result}");
}

#[test]
fn run_bash_poll_mode_requests_user_approval_in_ask_mode() {
    let store = FileShellJobStore::new(&tmp_memory_dir("poll_approval"));
    let cwd = tmp_cwd("poll_approval");
    let result = execute_run_bash(
        "test -f /tmp/timem_poll_marker",
        &cwd,
        false,
        5000,
        Some(1000),
        1000,
        BashApprovalMode::Ask,
        &store,
        "session_a",
        "turn_a",
        false,
        &mut NeverCancelRuntime,
    );
    match result {
        ActionExecution::NeedsApproval(pending) => {
            assert_eq!(pending.request.action, "run_bash");
            assert_eq!(pending.request.risk, "local_command_execution");
        }
        other => panic!("expected run_bash approval request, got {other:?}"),
    }
}

#[test]
fn run_bash_polling_requires_loop_cmd_and_interval_pair() {
    let store = FileShellJobStore::new(&tmp_memory_dir("poll_pairing"));
    let cwd = tmp_cwd("poll_pairing");
    let cmd_with_interval = execute_run_bash(
        "test -f /tmp/timem_poll_marker",
        &cwd,
        false,
        5000,
        Some(1000),
        1000,
        BashApprovalMode::Approve,
        &store,
        "session_a",
        "turn_a",
        true,
        &mut NeverCancelRuntime,
    );
    match cmd_with_interval {
        ActionExecution::Completed(text) => {
            assert!(
                text.contains("interval_ms is only valid with loop_cmd"),
                "{text}"
            );
        }
        other => panic!("expected pairing error, got {other:?}"),
    }

    let loop_without_interval = execute_run_bash(
        "test -f /tmp/timem_poll_marker",
        &cwd,
        false,
        5000,
        None,
        1000,
        BashApprovalMode::Approve,
        &store,
        "session_a",
        "turn_a",
        false,
        &mut NeverCancelRuntime,
    );
    match loop_without_interval {
        ActionExecution::Completed(text) => {
            assert!(text.contains("loop_cmd needs interval_ms"), "{text}");
        }
        other => panic!("expected pairing error, got {other:?}"),
    }
}

#[test]
fn polling_bash_waits_until_async_file_appears() {
    let dir = tmp_memory_dir("poll_async_file");
    let flag = dir.join("done.flag");
    let flag_path = shell_quote_path(&flag);
    let mut runtime = NeverCancelRuntime;
    let _ = fs::remove_file(&flag);
    let mut child = Command::new(BASH_EXECUTABLE)
        .arg("-lc")
        .arg(format!("sleep 0.3; touch {flag_path}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn delayed flag creator");

    let started = Instant::now();
    let result = execute_polling_bash(
        &format!("test -f {flag_path}"),
        &dir,
        100,
        2000,
        1000,
        &mut runtime,
    );
    let elapsed = started.elapsed();

    assert!(result.contains("Polling state: finished"), "{result}");
    assert!(
        result.contains("Success condition: exit code 0"),
        "{result}"
    );
    assert!(
        elapsed >= Duration::from_millis(200),
        "poll should wait for asynchronous file creation, elapsed={elapsed:?}\n{result}"
    );
    assert!(
        elapsed < Duration::from_millis(1500),
        "poll should return soon after condition succeeds, elapsed={elapsed:?}\n{result}"
    );
    let _ = child.wait();
}

#[test]
fn background_job_reports_pid_and_running_list_until_exit() {
    let dir = tmp_memory_dir("background_job");
    let store = FileShellJobStore::new(&dir);
    let started =
        store.spawn_background("sleep 1; printf background_ok", &dir, "session_a", "turn_a");
    assert!(
        started.contains("now keeps running in background"),
        "{started}"
    );
    let pid = started
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|rest| rest.split(',').next())
        .and_then(|pid| pid.parse::<u32>().ok())
        .unwrap();
    let running = store.running_for_session("session_a");
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].pid, pid);
    assert_eq!(running[0].kind, "background");

    let mut running = Vec::new();
    let mut updates = Vec::new();
    let wait_started = Instant::now();
    while wait_started.elapsed() < Duration::from_secs(5) {
        (running, updates) = store.refresh_for_session("session_a");
        if running.is_empty() && !updates.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(running.is_empty(), "background job should have exited");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].pid, pid);
    assert_eq!(updates[0].description(), "background job");
    assert_eq!(updates[0].status, "0");
    assert_eq!(updates[0].output, "background_ok");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn timeout_job_reports_pid_and_later_exit_update() {
    let dir = tmp_memory_dir("timeout_job");
    let store = FileShellJobStore::new(&dir);
    let mut runtime = NeverCancelRuntime;
    let result = store.run_with_timeout(
        "printf started; sleep 1; printf done",
        &dir,
        100,
        "session_a",
        "turn_a",
        &mut runtime,
    );
    assert!(result.contains("timeout, but is still running"), "{result}");
    assert!(result.contains("process was not killed"), "{result}");
    assert!(result.contains("no final exit code yet"), "{result}");
    let pid = result
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|rest| rest.split(',').next())
        .and_then(|pid| pid.parse::<u32>().ok())
        .expect("pid");

    let running = store.running_for_session("session_a");
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].pid, pid);
    assert_eq!(running[0].kind, "timeout");

    let mut running = Vec::new();
    let mut updates = Vec::new();
    let wait_started = Instant::now();
    while wait_started.elapsed() < Duration::from_secs(3) {
        (running, updates) = store.refresh_for_session("session_a");
        if running.is_empty() && !updates.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(running.is_empty(), "timed-out job should eventually exit");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].pid, pid);
    assert_eq!(updates[0].description(), "old timeout job");
    assert_eq!(updates[0].status, "0");
    assert_eq!(updates[0].output, "starteddone");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn timeout_job_supports_heredoc_with_backticks() {
    let dir = tmp_memory_dir("timeout_heredoc_backticks");
    let store = FileShellJobStore::new(&dir);
    let mut runtime = NeverCancelRuntime;
    let result = store.run_with_timeout(
        "cat <<'EOF'\nline with `ShellJobWatcher` backticks\nEOF",
        &dir,
        5000,
        "session_a",
        "turn_a",
        &mut runtime,
    );
    assert!(result.contains("Exit code: 0"), "{result}");
    assert!(
        result.contains("line with `ShellJobWatcher` backticks"),
        "{result}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn background_job_supports_heredoc_with_backticks() {
    let dir = tmp_memory_dir("background_heredoc_backticks");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background(
        "cat <<'EOF'\nbackground `ShellJobWatcher` output\nEOF",
        &dir,
        "session_a",
        "turn_a",
    );
    assert!(
        started.contains("now keeps running in background"),
        "{started}"
    );

    let mut updates = Vec::new();
    let wait_started = Instant::now();
    while wait_started.elapsed() < Duration::from_secs(3) {
        let (running_now, updates_now) = store.refresh_for_session("session_a");
        updates = updates_now;
        if running_now.is_empty() && !updates.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].status, "0");
    assert_eq!(updates[0].output, "background `ShellJobWatcher` output");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn watcher_reaps_sigsegv_background_job_and_reports_signal_transition() {
    let dir = tmp_memory_dir("background_sigsegv");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background("kill -SEGV $$", &dir, "session_signal", "turn_signal");
    assert!(
        started.contains("now keeps running in background"),
        "{started}"
    );

    let mut updates = Vec::new();
    let wait_started = Instant::now();
    while wait_started.elapsed() < Duration::from_secs(3) {
        let (running, current_updates) = store.refresh_for_session("session_signal");
        updates = current_updates;
        if running.is_empty() && !updates.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(updates.len(), 1, "signal exit must produce one update");
    assert_eq!(updates[0].status, "signal:11");
    assert!(store.running_for_session("session_signal").is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tracked_job_preserves_complex_shell_syntax_without_runtime_wrapper() {
    let dir = tmp_memory_dir("tracked_complex_shell_syntax");
    let store = FileShellJobStore::new(&dir);
    let mut runtime = NeverCancelRuntime;
    let result = store.run_with_timeout(
            "x='brace ok'; (printf '%s\\n' \"$x\"); { printf '%s\\n' group; }; cat <<'EOF'\nliteral `backticks` and $(not expanded)\nEOF",
            &dir,
            5000,
            "session_a",
            "turn_a",
            &mut runtime,
        );
    assert!(result.contains("Exit code: 0"), "{result}");
    assert!(result.contains("brace ok"), "{result}");
    assert!(result.contains("group"), "{result}");
    assert!(
        result.contains("literal `backticks` and $(not expanded)"),
        "{result}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tracked_job_runs_real_bash_syntax() {
    let dir = tmp_memory_dir("tracked_real_bash_syntax");
    let store = FileShellJobStore::new(&dir);
    let mut runtime = NeverCancelRuntime;
    let result = store.run_with_timeout(
        "arr=(alpha beta); [[ ${arr[1]} == beta ]] && printf '%s\\n' \"${arr[1]}\"",
        &dir,
        5000,
        "session_a",
        "turn_a",
        &mut runtime,
    );
    assert!(result.contains("Exit code: 0"), "{result}");
    assert!(result.contains("beta"), "{result}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn watcher_reaps_background_job_without_refresh_polling() {
    let dir = tmp_memory_dir("watcher_reaps_background");
    let store = FileShellJobStore::new(&dir);
    let started =
        store.spawn_background("printf watcher_reaped", &dir, "session_watch", "turn_watch");
    assert!(
        started.contains("now keeps running in background"),
        "{started}"
    );
    let record = store
        .guard
        .with_read(|| store.records_unlocked().into_iter().next())
        .unwrap()
        .expect("job record");

    let started_wait = Instant::now();
    while started_wait.elapsed() < Duration::from_secs(3) && !store.record_finished(&record) {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        store.record_finished(&record),
        "shared watcher should write the status file without refresh polling"
    );

    let (running, updates) = store.refresh_for_session("session_watch");
    assert!(running.is_empty());
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].status, "0");
    assert_eq!(updates[0].output, "watcher_reaped");
    assert!(
        !process_running(record.pid),
        "reaped child should not be reported as running"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn process_running_treats_zombie_as_not_running() {
    let mut child = Command::new(BASH_EXECUTABLE)
        .arg("-lc")
        .arg("exit 0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn short child");
    let pid = child.id();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) && process_running(pid) {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !process_running(pid),
        "exited child pid {pid} should not be reported as running"
    );
    let _ = child.wait();
}

#[test]
fn running_job_list_context_uses_pid_kind_and_command() {
    let dir = tmp_memory_dir("running_context");
    let store = FileShellJobStore::new(&dir);

    let _ = store.spawn_background("sleep 10", &dir, "session_owned", "turn_a");
    let _ = store.spawn_background("sleep 10", &dir, "session_other", "turn_a");
    let context = store
        .running_job_list_context("session_owned")
        .expect("running context");

    assert!(context.starts_with("RUNNING JOB LIST:"), "{context}");
    assert!(context.contains("background job"), "{context}");
    assert!(context.contains("cmd=sleep 10"), "{context}");
    assert!(!context.contains("session_other"), "{context}");
    for job in store.running_for_session("session_owned") {
        terminate_process(job.pid);
    }
    for job in store.running_for_session("session_other") {
        terminate_process(job.pid);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bash_validation_rejects_empty_and_allows_long_commands() {
    assert_eq!(
        validate_bash_request(""),
        Err("command_required".to_string())
    );
    let huge = "x".repeat(2001);
    assert!(validate_bash_request(&huge).is_ok());
    assert!(validate_bash_request("printf ok").is_ok());
}

#[test]
fn bash_validation_blocks_recursive_force_root_delete_variants() {
    for command in [
        "rm -rf /",
        "rm -fr -- /",
        "rm -rf /*",
        "echo ok; rm -rf /; echo done",
        "EMPTY=; rm -rf \"$EMPTY\"/",
        "EMPTY=; rm -rf ${EMPTY}/*",
        "rm -rf $(printf '')/",
        "rm -rf $(printf '')/*",
        "if true; then rm -rf /; fi",
    ] {
        assert_eq!(
            validate_bash_request(command),
            Err("dangerous_recursive_root_delete".to_string()),
            "{command}"
        );
    }
}

#[test]
fn bash_validation_allows_non_root_delete_variants() {
    for command in [
        "rm -rf ./target",
        "rm -rf target",
        "rm -rf /tmp/timem-test-dir",
        "rm -r /",
        "rm -f /",
        "printf 'rm -rf / is only text'",
    ] {
        assert!(validate_bash_request(command).is_ok(), "{command}");
    }
}

#[test]
fn run_bash_blocks_dangerous_delete_before_spawning_or_approval() {
    let store = FileShellJobStore::new(&tmp_memory_dir("dangerous_delete_guard"));
    let cwd = tmp_cwd("dangerous_delete_guard");
    let marker = cwd.join("marker.txt");
    let command = format!("rm -rf /; printf should_not_run > {}", marker.display());
    let result = execute_run_bash(
        &command,
        &cwd,
        false,
        5000,
        None,
        5000,
        BashApprovalMode::Ask,
        &store,
        "session_a",
        "turn_a",
        true,
        &mut NeverCancelRuntime,
    );
    match result {
        ActionExecution::Completed(text) => {
            assert!(text.contains("blocked by Timem safety policy"), "{text}");
            assert!(
                !marker.exists(),
                "blocked command must not execute follow-up"
            );
        }
        ActionExecution::NeedsApproval(_) => {
            panic!("dangerous command should be blocked before approval")
        }
    }
}

#[test]
fn run_bash_blocks_dangerous_polling_loop_command() {
    let store = FileShellJobStore::new(&tmp_memory_dir("dangerous_poll_guard"));
    let cwd = tmp_cwd("dangerous_poll_guard");
    let result = execute_run_bash(
        "rm -rf $(printf '')/*",
        &cwd,
        false,
        5000,
        Some(1000),
        1000,
        BashApprovalMode::Approve,
        &store,
        "session_a",
        "turn_a",
        false,
        &mut NeverCancelRuntime,
    );
    match result {
        ActionExecution::Completed(text) => {
            assert!(text.contains("blocked by Timem safety policy"), "{text}");
        }
        other => panic!("expected safety block, got {other:?}"),
    }
}

#[test]
fn approved_bash_rechecks_safety_before_execution() {
    let store = FileShellJobStore::new(&tmp_memory_dir("approved_dangerous_guard"));
    let cwd = tmp_cwd("approved_dangerous_guard");
    let marker = cwd.join("marker.txt");
    let request = ApprovalRequest {
        approval_id: "approval_test".to_string(),
        action: "run_bash".to_string(),
        command: "rm -rf /".to_string(),
        reason: "test".to_string(),
        risk: "local_command_execution".to_string(),
    };
    let command = format!("rm -rf /; printf should_not_run > {}", marker.display());
    let result = execute_approved_bash(
        &command,
        &cwd,
        false,
        5000,
        None,
        5000,
        "session_a",
        "turn_a",
        true,
        &request,
        &store,
        &mut NeverCancelRuntime,
    );
    assert!(
        result.contains("blocked by Timem safety policy"),
        "{result}"
    );
    assert!(
        result.contains("approval_status: approved_by_user"),
        "{result}"
    );
    assert!(
        !marker.exists(),
        "blocked approved command must not execute"
    );
}

#[test]
fn run_bash_allows_safe_tmp_delete() {
    let store = FileShellJobStore::new(&tmp_memory_dir("safe_tmp_delete"));
    let cwd = tmp_cwd("safe_tmp_delete");
    let target = cwd.join("safe-delete");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("file.txt"), "ok").unwrap();
    let result = execute_run_bash(
        &format!("rm -rf {}", shell_quote_path(&target)),
        &cwd,
        false,
        5000,
        None,
        5000,
        BashApprovalMode::Approve,
        &store,
        "session_a",
        "turn_a",
        true,
        &mut NeverCancelRuntime,
    );
    match result {
        ActionExecution::Completed(text) => {
            assert!(text.contains("Exit code: 0"), "{text}");
            assert!(!target.exists(), "safe temp dir should be removable");
        }
        other => panic!("expected safe command to run, got {other:?}"),
    }
}

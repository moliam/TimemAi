use super::*;
use crate::LongRunningCommandDecision;
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
        self.cancelled.swap(true, Ordering::Relaxed)
    }
}

struct CancelAfterFileRuntime {
    path: PathBuf,
}

impl ActionRuntime for CancelAfterFileRuntime {
    fn should_cancel(&mut self) -> bool {
        self.path.exists()
    }
}

#[derive(Default)]
struct CaptureLongRunningStatusRuntime {
    statuses: Vec<LongRunningCommandStatus>,
}

impl ActionRuntime for CaptureLongRunningStatusRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }

    fn on_long_running_command(
        &mut self,
        status: &LongRunningCommandStatus,
    ) -> LongRunningCommandDecision {
        self.statuses.push(status.clone());
        LongRunningCommandDecision::Continue
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
fn normal_bash_sets_noninteractive_pager_environment() {
    let mut runtime = NeverCancelRuntime;
    let result = execute_one_bash(
        "printf 'GIT_PAGER=%s\\nPAGER=%s\\nTERM=%s\\n' \"$GIT_PAGER\" \"$PAGER\" \"$TERM\"",
        1000,
        &mut runtime,
    );

    assert!(result.contains("GIT_PAGER=cat"), "{result}");
    assert!(result.contains("PAGER=cat"), "{result}");
    assert!(result.contains("TERM=dumb"), "{result}");
}

#[test]
fn normal_bash_reports_status_and_output() {
    let mut runtime = NeverCancelRuntime;
    let result = execute_one_bash("printf shell_ok", 1000, &mut runtime);
    assert!(result.contains("Action result: run_bash"));
    assert!(result.contains("Exit code: 0"));
    assert!(result.contains("shell_ok"));
}

#[test]
fn foreground_bash_preserves_stdout_and_stderr_independently() {
    let mut runtime = NeverCancelRuntime;
    let result = execute_one_bash_structured(
        "printf 'stdout-value'; printf 'stderr-value' >&2; exit 7",
        Path::new("."),
        1000,
        &mut runtime,
    );

    assert_eq!(result.status, Some(7));
    assert_eq!(result.stdout, "stdout-value");
    assert_eq!(result.stderr, "stderr-value");
    assert!(result.output.contains("stdout-value"), "{}", result.output);
    assert!(
        result.output.contains("stderr: stderr-value"),
        "{}",
        result.output
    );

    let outcome = result.to_action_outcome("run_bash");
    let evidence = outcome.bash_result.expect("structured Bash evidence");
    assert_eq!(evidence.stdout, "stdout-value");
    assert_eq!(evidence.stderr, "stderr-value");
    assert_eq!(evidence.exit_code, Some(7));
    assert_eq!(evidence.signal, None);
    assert_eq!(evidence.pid, None);
    assert_eq!(evidence.error_type, None);
}

#[test]
fn bash_command_outcomes_keep_lifecycle_separate_from_result_metadata() {
    let nonzero = BashCommandOutput {
        command: "exit 7".to_string(),
        status: Some(7),
        signal: None,
        stdout: String::new(),
        stderr: "diagnostic".to_string(),
        output: "stderr: diagnostic".to_string(),
        error: None,
        tail_out: false,
    }
    .to_action_outcome("run_bash");
    assert_eq!(nonzero.status, ActionStatus::Failed);
    let evidence = nonzero.bash_result.expect("nonzero Bash evidence");
    assert_eq!(evidence.exit_code, Some(7));
    assert_eq!(evidence.signal, None);
    assert_eq!(evidence.error_type, None);

    let signalled = BashCommandOutput {
        command: "kill -SEGV $$".to_string(),
        status: None,
        signal: Some(11),
        stdout: String::new(),
        stderr: String::new(),
        output: "<no output>".to_string(),
        error: None,
        tail_out: false,
    }
    .to_action_outcome("run_bash");
    assert_eq!(signalled.status, ActionStatus::Failed);
    let evidence = signalled.bash_result.expect("signal Bash evidence");
    assert_eq!(evidence.exit_code, None);
    assert_eq!(evidence.signal, Some(11));
    assert_eq!(evidence.error_type, None);

    let cancelled = bash_error("cancel", "cancelled").to_action_outcome("run_bash");
    assert_eq!(cancelled.status, ActionStatus::Cancelled);
    let evidence = cancelled.bash_result.expect("cancelled Bash evidence");
    assert_eq!(evidence.error_type.as_deref(), Some("Cancelled"));
    assert_eq!(evidence.pid, None);

    let invalid = bash_error("true", "invalid_timeout").to_action_outcome("run_bash");
    assert_eq!(invalid.status, ActionStatus::Failed);
    let evidence = invalid.bash_result.expect("invalid input Bash evidence");
    assert_eq!(evidence.error_type.as_deref(), Some("InvalidInput"));

    let spawn_failed = bash_error("true", "command_failed").to_action_outcome("run_bash");
    assert_eq!(spawn_failed.status, ActionStatus::Failed);
    let evidence = spawn_failed
        .bash_result
        .expect("spawn failure Bash evidence");
    assert_eq!(evidence.error_type.as_deref(), Some("SpawnFailed"));

    let timeout = BashCommandOutput {
        command: "sleep 10".to_string(),
        status: None,
        signal: None,
        stdout: "partial".to_string(),
        stderr: String::new(),
        output: "partial".to_string(),
        error: Some("timeout_still_running:4321".to_string()),
        tail_out: false,
    }
    .to_action_outcome("run_bash");
    assert_eq!(timeout.status, ActionStatus::BackgroundRunning);
    let evidence = timeout.bash_result.expect("timeout Bash evidence");
    assert_eq!(evidence.pid, Some(4321));
    assert!(evidence.timed_out);
    assert_eq!(evidence.pid_kind.as_deref(), Some(runtime_child_pid_kind()));
    assert_eq!(evidence.error_type, None);

    let ended_timeout = bash_error("sleep 10", "timeout").to_action_outcome("run_bash");
    assert_eq!(ended_timeout.status, ActionStatus::Timeout);
    let evidence = ended_timeout
        .bash_result
        .expect("ended timeout Bash evidence");
    assert_eq!(evidence.pid, None);
    assert!(!evidence.timed_out);
    assert_eq!(evidence.pid_kind, None);

    let running = BashCommandOutput {
        command: "build".to_string(),
        status: None,
        signal: None,
        stdout: String::new(),
        stderr: String::new(),
        output: String::new(),
        error: Some("long_running_still_running:9876:5000".to_string()),
        tail_out: false,
    }
    .to_action_outcome("run_bash");
    assert_eq!(running.status, ActionStatus::BackgroundRunning);
    let evidence = running.bash_result.expect("running Bash evidence");
    assert_eq!(evidence.pid, Some(9876));
    assert!(!evidence.timed_out);
    assert_eq!(evidence.pid_kind.as_deref(), Some(runtime_child_pid_kind()));
    assert_eq!(evidence.error_type, None);
}

#[test]
fn foreground_bash_preserves_stderr_only_and_empty_streams() {
    let mut runtime = NeverCancelRuntime;
    let stderr_only = execute_one_bash_structured(
        "printf 'stderr-only' >&2; exit 9",
        Path::new("."),
        1000,
        &mut runtime,
    );
    assert_eq!(stderr_only.status, Some(9));
    assert_eq!(stderr_only.stdout, "");
    assert_eq!(stderr_only.stderr, "stderr-only");
    assert_eq!(stderr_only.output, "stderr: stderr-only");

    let empty = execute_one_bash_structured("exit 0", Path::new("."), 1000, &mut runtime);
    assert_eq!(empty.status, Some(0));
    assert_eq!(empty.stdout, "");
    assert_eq!(empty.stderr, "");
    assert_eq!(empty.output, "<no output>");
}

#[test]
fn polling_bash_preserves_last_stdout_and_stderr_evidence() {
    let cwd = tmp_cwd("polling_split_streams");
    let mut runtime = NeverCancelRuntime;
    let outcome = execute_polling_bash_outcome(
        "printf poll-out; printf poll-err >&2; exit 0",
        &cwd,
        10,
        1000,
        1000,
        &mut runtime,
    );

    assert_eq!(outcome.status, ActionStatus::Completed);
    let evidence = outcome.bash_result.expect("polling Bash evidence");
    assert_eq!(evidence.stdout, "poll-out");
    assert_eq!(evidence.stderr, "poll-err");
    assert_eq!(evidence.exit_code, Some(0));
    assert_eq!(evidence.signal, None);
}

#[test]
fn historical_shell_job_record_without_stderr_file_is_treated_as_stdout_only() {
    let dir = tmp_memory_dir("legacy_merged_output");
    let output_file = dir.join("legacy.out");
    fs::write(&output_file, "legacy merged bytes").unwrap();

    let record = ShellJobRecord {
        id: "legacy-job".to_string(),
        created_at_ms: now_ms(),
        kind: "background".to_string(),
        session_id: "legacy-session".to_string(),
        turn_id: "legacy-turn".to_string(),
        pid: 1,
        process_identity: None,
        tool_call_id: "test_call".to_string(),
        owner_id: None,
        command: "legacy".to_string(),
        cwd: dir.display().to_string(),
        output_file: output_file.display().to_string(),
        stderr_file: String::new(),
        status_file: dir.join("legacy.status").display().to_string(),
        tail_out: false,
    };

    let (stdout, stderr) = read_shell_job_streams(&record);
    assert_eq!(stdout, "legacy merged bytes");
    assert_eq!(stderr, "");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn shell_job_record_deserializes_without_legacy_stderr_file_field() {
    let record: ShellJobRecord = serde_json::from_value(serde_json::json!({
        "id": "legacy-json",
        "created_at_ms": 1,
        "pid": 2,
        "command": "printf legacy",
        "output_file": "legacy.out",
        "status_file": "legacy.status"
    }))
    .unwrap();

    assert_eq!(record.stderr_file, "");
    assert_eq!(record.kind, "background");
}

#[test]
fn run_bash_action_results_do_not_repeat_command_text() {
    let command = "printf unique_command_marker";
    let completed = BashCommandOutput {
        command: command.to_string(),
        status: Some(0),
        signal: None,
        stdout: String::new(),
        stderr: String::new(),
        output: "unique_command_marker".to_string(),
        error: None,
        tail_out: false,
    }
    .to_action_result("run_bash");

    assert!(!completed.contains("Command:"), "{completed}");
    assert_eq!(completed.matches(command).count(), 0, "{completed}");
    assert!(completed.contains("unique_command_marker"), "{completed}");

    let failed = BashCommandOutput {
        command: "false unique_failure_command".to_string(),
        status: Some(1),
        signal: None,
        stdout: String::new(),
        stderr: String::new(),
        output: "<no output>".to_string(),
        error: None,
        tail_out: false,
    }
    .to_action_result("run_bash");
    assert!(!failed.contains("Command:"), "{failed}");
    assert!(!failed.contains("false unique_failure_command"), "{failed}");

    let timed_out = BashCommandOutput {
        command: "sleep 123 unique_timeout_command".to_string(),
        status: None,
        signal: None,
        stdout: String::new(),
        stderr: String::new(),
        output: String::new(),
        error: Some("timeout_still_running:12345".to_string()),
        tail_out: false,
    }
    .to_action_result("run_bash");
    assert!(!timed_out.contains("Command:"), "{timed_out}");
    assert!(
        !timed_out.contains("sleep 123 unique_timeout_command"),
        "{timed_out}"
    );
}

#[test]
fn bash_result_builder_preserves_raw_output_for_the_model_result_gate() {
    let output = format!("{} alpha beta gamma", "x".repeat(33_000));
    let result = BashCommandOutput {
        command: "printf long-output".to_string(),
        status: Some(0),
        signal: None,
        stdout: String::new(),
        stderr: String::new(),
        output,
        error: None,
        tail_out: false,
    }
    .to_action_result("run_bash");

    let rendered_output = result.split_once("Return:\n").unwrap().1;
    assert_eq!(
        rendered_output,
        format!("{} alpha beta gamma", "x".repeat(33_000))
    );
    assert!(!rendered_output.contains("!!!Too long,"));
}

#[cfg(unix)]
#[test]
fn normal_bash_contains_child_sigsegv_and_accepts_follow_up_command() {
    let mut runtime = NeverCancelRuntime;
    // Linux may synchronously hand a crashing process to its core-dump
    // collector before wait(2) reports the signal. Keep this test focused on
    // signal containment instead of relying on a macOS-sized timeout.
    let crashed = execute_one_bash("kill -SEGV $$", 10_000, &mut runtime);
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
    let mut runtime = CaptureLongRunningStatusRuntime::default();
    let result = execute_one_bash_structured_with_prompt_after(
        "sleep 1; printf finished",
        Path::new("."),
        5000,
        &mut runtime,
        Duration::from_millis(50),
    )
    .to_action_result("run_bash");

    assert!(result.contains("Exit code: 0"), "{result}");
    assert!(result.contains("finished"), "{result}");
    assert!(!runtime.statuses.is_empty());
    let status = &runtime.statuses[0];
    assert_eq!(status.action, "run_bash");
    assert_eq!(status.command, "sleep 1; printf finished");
    assert_ne!(status.pid, 0);
    assert_eq!(status.timeout_ms, Some(5000));
    assert!(status.elapsed >= Duration::from_millis(50));
}

#[cfg(unix)]
#[test]
fn normal_bash_cancel_terminates_the_entire_process_group() {
    let cwd = tmp_cwd("cancel_process_group");
    let child_pid_file = cwd.join("child.pid");
    let command = format!(
        "bash -c 'trap \"\" TERM; tail -f /dev/null' & echo $! > {}; wait",
        shell_quote_path(&child_pid_file)
    );
    let mut runtime = CancelAfterFileRuntime {
        path: child_pid_file.clone(),
    };

    let started = Instant::now();
    let result = execute_one_bash_structured(&command, &cwd, 60_000, &mut runtime)
        .to_action_result("run_bash");

    assert!(result.contains("cancelled before it completed"), "{result}");
    assert!(started.elapsed() < Duration::from_secs(3));
    let child_pid = fs::read_to_string(&child_pid_file)
        .expect("child pid should be recorded before cancellation")
        .trim()
        .parse::<u32>()
        .expect("child pid should be numeric");
    thread::sleep(Duration::from_millis(100));
    assert!(
        !process_running(child_pid),
        "descendant process {child_pid} survived run_bash cancellation"
    );
}

#[test]
fn successful_run_bash_status_is_independent_of_output_words() {
    let store = FileShellJobStore::new(&tmp_memory_dir("status_words"));
    let cwd = tmp_cwd("status_words");
    let result = execute_run_bash(
        "printf 'timeout documentation\\nerror: example\\ncancelled text\\n'",
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

    let ActionExecution::Completed(outcome) = result else {
        panic!("approve mode should execute directly");
    };
    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(
        outcome.text.contains("timeout documentation"),
        "{}",
        outcome.text
    );
    assert!(outcome.text.contains("error: example"), "{}", outcome.text);
    assert!(outcome.text.contains("cancelled text"), "{}", outcome.text);
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Failed);
            assert!(
                outcome.text.contains("long sleep in normal mode"),
                "{}",
                outcome.text
            );
            assert!(outcome.text.contains("interval_ms"));
            let evidence = outcome
                .bash_result
                .expect("rejected run_bash must retain structured evidence");
            assert_eq!(evidence.error_type.as_deref(), Some("InvalidInput"));
            assert_eq!(evidence.exit_code, None);
            assert_eq!(evidence.signal, None);
            assert_eq!(evidence.pid, None);
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Completed);
            assert!(outcome.text.contains("Exit code: 0"));
            assert!(outcome.text.contains("done"));
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
    let outcome = execute_polling_bash_outcome(&command, &dir, 1000, 5000, 1000, &mut runtime);
    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(
        outcome.text.contains("Action result: run_bash"),
        "{}",
        outcome.text
    );
    assert!(
        outcome.text.contains("Polling state: finished"),
        "{}",
        outcome.text
    );
    assert!(outcome.text.contains("Attempts: 2"), "{}", outcome.text);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn run_bash_poll_mode_times_out_when_command_stays_nonzero() {
    let mut runtime = NeverCancelRuntime;
    let cwd = tmp_cwd("poll_timeout");
    let outcome = execute_polling_bash_outcome(
        "printf waiting; exit 7",
        &cwd,
        1000,
        1100,
        1000,
        &mut runtime,
    );
    assert_eq!(outcome.status, ActionStatus::Timeout);
    assert!(
        outcome.text.contains("Polling state: timeout"),
        "{}",
        outcome.text
    );
    assert!(
        outcome.text.contains("Last loop_cmd exit code: 7"),
        "{}",
        outcome.text
    );
    assert!(outcome.text.contains("waiting"), "{}", outcome.text);
    assert!(
        outcome.text.contains(
            "exit_code is from the last loop_cmd execution; polling does not know the waited task's own exit code"
        ),
        "{}",
        outcome.text
    );
    let evidence = outcome
        .bash_result
        .expect("timed-out polling must retain structured evidence");
    assert_eq!(evidence.exit_code, Some(7));
}

#[test]
fn run_bash_poll_mode_can_be_cancelled_during_wait() {
    let cancelled = AtomicBool::new(false);
    let mut runtime = ToggleCancelRuntime {
        cancelled: &cancelled,
    };
    let cwd = tmp_cwd("poll_cancel");
    let outcome = execute_polling_bash_outcome("exit 1", &cwd, 1000, 10_000, 1000, &mut runtime);
    assert_eq!(outcome.status, ActionStatus::Cancelled);
    assert!(
        outcome.text.contains("Polling state: cancelled"),
        "{}",
        outcome.text
    );
    let evidence = outcome
        .bash_result
        .expect("cancelled polling must retain structured evidence");
    assert_eq!(evidence.error_type.as_deref(), Some("Cancelled"));
    assert_eq!(evidence.pid, None);
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Failed);
            assert!(
                outcome
                    .text
                    .contains("interval_ms is only valid with loop_cmd"),
                "{}",
                outcome.text
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Failed);
            assert!(
                outcome.text.contains("loop_cmd needs interval_ms"),
                "{}",
                outcome.text
            );
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
    let mut child = Command::new(crate::os::BASH_EXECUTABLE)
        .arg("-lc")
        .arg(format!("sleep 0.3; touch {flag_path}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn delayed flag creator");

    let started = Instant::now();
    let outcome = execute_polling_bash_outcome(
        &format!("test -f {flag_path}"),
        &dir,
        100,
        2000,
        1000,
        &mut runtime,
    );
    let elapsed = started.elapsed();

    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(
        outcome.text.contains("Polling state: finished"),
        "{}",
        outcome.text
    );
    assert!(
        outcome.text.contains("Success condition: exit code 0"),
        "{}",
        outcome.text
    );
    assert!(
        elapsed >= Duration::from_millis(200),
        "poll should wait for asynchronous file creation, elapsed={elapsed:?}\n{}",
        outcome.text
    );
    assert!(
        elapsed < Duration::from_millis(1500),
        "poll should return soon after condition succeeds, elapsed={elapsed:?}\n{}",
        outcome.text
    );
    let _ = child.wait();
}

#[test]
fn refresh_rejects_a_live_pid_when_the_recorded_process_identity_does_not_match() {
    let dir = tmp_memory_dir("pid_identity_mismatch");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background("sleep 30", &dir, "identity_session", "identity_turn");
    let pid = started
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|rest| rest.split(',').next())
        .and_then(|pid| pid.parse::<u32>().ok())
        .expect("pid");
    store.forget_watched_job_for_tests(pid);

    let index = dir.join("shell_jobs/jobs.jsonl");
    let text = fs::read_to_string(&index).unwrap();
    let mut record: ShellJobRecord = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    record.process_identity = Some("definitely-not-this-process".to_string());
    fs::write(
        &index,
        format!("{}\n", serde_json::to_string(&record).unwrap()),
    )
    .unwrap();

    let (running, updates) = store.refresh_for_session("identity_session");
    assert!(running.is_empty());
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].pid, pid);
    assert_eq!(updates[0].status, "pid_identity_changed");

    crate::os::terminate_process(pid);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn background_job_reports_pid_and_running_list_until_exit() {
    let dir = tmp_memory_dir("background_job");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background(
        "sleep 1; printf background_ok; printf background_err >&2",
        &dir,
        "session_a",
        "turn_a",
    );
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
    assert_eq!(updates[0].stdout, "background_ok");
    assert_eq!(updates[0].stderr, "background_err");
    assert_eq!(updates[0].output, "background_ok\nstderr: background_err");
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
    assert_eq!(updates[0].stdout, "starteddone");
    assert_eq!(updates[0].stderr, "");
    assert_eq!(updates[0].output, "starteddone");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn timed_out_job_remains_cancellable_after_launcher_exits() {
    let dir = tmp_memory_dir("timeout_group_cancel_after_launcher");
    let store = FileShellJobStore::new(&dir);
    let descendant_pid_file = dir.join("descendant.pid");
    let command = format!(
        r#"tail -f /dev/null & child=$!; printf '%s' "$child" > {}; exit 0"#,
        shell_quote_path(&descendant_pid_file)
    );
    let mut runtime = NeverCancelRuntime;
    let result = store.run_with_timeout(
        &command,
        &dir,
        100,
        "timeout-group-session",
        "timeout-group-turn",
        &mut runtime,
    );
    assert!(result.contains("timeout, but is still running"), "{result}");
    let leader_pid = result
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|rest| rest.split(',').next())
        .and_then(|pid| pid.parse::<u32>().ok())
        .expect("managed group leader pid");
    let descendant_pid = fs::read_to_string(&descendant_pid_file)
        .expect("descendant pid file")
        .trim()
        .parse::<u32>()
        .expect("numeric descendant pid");

    assert!(crate::os::process_group_running(leader_pid));
    assert!(process_running(descendant_pid));
    assert_eq!(
        store
            .cancel_unfinished_for_session("timeout-group-session")
            .len(),
        1
    );

    let deadline = Instant::now() + Duration::from_secs(3);
    while process_running(descendant_pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !process_running(descendant_pid),
        "descendant {descendant_pid} survived timeout-job cancellation"
    );
    assert!(!crate::os::process_group_running(leader_pid));
    assert!(store
        .running_for_session("timeout-group-session")
        .is_empty());
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
fn background_bash_sets_noninteractive_pager_environment() {
    let dir = tmp_memory_dir("background_pager_environment");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background(
        "printf 'GIT_PAGER=%s\\nPAGER=%s\\nTERM=%s\\n' \"$GIT_PAGER\" \"$PAGER\" \"$TERM\"",
        &dir,
        "session_env",
        "turn_env",
    );
    assert!(
        started.contains("now keeps running in background"),
        "{started}"
    );

    let started_wait = Instant::now();
    let update = loop {
        let (_, updates) = store.refresh_for_session("session_env");
        if let Some(update) = updates.into_iter().next() {
            break update;
        }
        assert!(
            started_wait.elapsed() < Duration::from_secs(3),
            "background environment command did not finish"
        );
        thread::sleep(Duration::from_millis(20));
    };

    assert_eq!(update.status, "0");
    assert!(update.output.contains("GIT_PAGER=cat"), "{}", update.output);
    assert!(update.output.contains("PAGER=cat"), "{}", update.output);
    assert!(update.output.contains("TERM=dumb"), "{}", update.output);
    let _ = fs::remove_dir_all(dir);
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
    let mut child = Command::new(crate::os::BASH_EXECUTABLE)
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
fn shell_job_retention_removes_only_old_finished_jobs_and_artifacts() {
    let dir = tmp_memory_dir("rolling_retention");
    let store = FileShellJobStore::new(&dir);
    let shell_dir = dir.join("shell_jobs");
    let cutoff_ms = now_ms();
    let make_record = |id: &str, created_at_ms: i64| ShellJobRecord {
        id: id.to_string(),
        created_at_ms,
        kind: "background".to_string(),
        session_id: "retention-session".to_string(),
        turn_id: id.to_string(),
        pid: std::process::id(),
        process_identity: None,
        tool_call_id: format!("call-{id}"),
        owner_id: Some("retention-owner".to_string()),
        command: format!("command-{id}"),
        cwd: dir.display().to_string(),
        output_file: shell_dir.join(format!("{id}.out")).display().to_string(),
        stderr_file: shell_dir.join(format!("{id}.err")).display().to_string(),
        status_file: shell_dir.join(format!("{id}.status")).display().to_string(),
        tail_out: false,
    };
    let old_finished = make_record("old-finished", cutoff_ms - 1);
    let boundary_finished = make_record("boundary-finished", cutoff_ms);
    let old_running = make_record("old-running", cutoff_ms - 1);
    for record in [&old_finished, &boundary_finished, &old_running] {
        fs::write(&record.output_file, format!("stdout-{}", record.id)).unwrap();
        fs::write(&record.stderr_file, format!("stderr-{}", record.id)).unwrap();
        store.append(record).unwrap();
    }
    fs::write(&old_finished.status_file, "exit:0").unwrap();
    fs::write(format!("{}.notified", old_finished.status_file), "1").unwrap();
    fs::write(&boundary_finished.status_file, "exit:0").unwrap();

    assert_eq!(store.prune_finished_before(cutoff_ms).unwrap(), 1);
    assert_eq!(store.prune_finished_before(cutoff_ms).unwrap(), 0);
    for path in [
        old_finished.output_file.as_str(),
        old_finished.stderr_file.as_str(),
        old_finished.status_file.as_str(),
    ] {
        assert!(!Path::new(path).exists());
    }
    assert!(!Path::new(&format!("{}.notified", old_finished.status_file)).exists());
    for record in [&boundary_finished, &old_running] {
        assert!(Path::new(&record.output_file).exists());
        assert!(Path::new(&record.stderr_file).exists());
    }
    let retained = store.records_unlocked();
    assert_eq!(retained.len(), 2);
    assert!(retained
        .iter()
        .any(|record| record.id == boundary_finished.id));
    assert!(retained.iter().any(|record| record.id == old_running.id));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn shell_job_retention_rejects_artifact_paths_outside_its_store() {
    let dir = tmp_memory_dir("retention_path_escape");
    let store = FileShellJobStore::new(&dir);
    let outside = dir.join("must-remain.out");
    let status = dir.join("shell_jobs/escaped.status");
    fs::write(&outside, "do not delete").unwrap();
    fs::write(&status, "exit:0").unwrap();
    let record = ShellJobRecord {
        id: "escaped".to_string(),
        created_at_ms: 1,
        kind: "background".to_string(),
        session_id: "session".to_string(),
        turn_id: "turn".to_string(),
        pid: std::process::id(),
        process_identity: None,
        tool_call_id: "call".to_string(),
        owner_id: Some("owner".to_string()),
        command: "command".to_string(),
        cwd: dir.display().to_string(),
        output_file: outside.display().to_string(),
        stderr_file: dir.join("shell_jobs/escaped.err").display().to_string(),
        status_file: status.display().to_string(),
        tail_out: false,
    };
    store.append(&record).unwrap();

    assert_eq!(store.prune_finished_before(2).unwrap(), 0);
    assert!(
        outside.exists(),
        "a corrupted index must not delete files outside shell_jobs"
    );
    assert_eq!(store.records_unlocked(), vec![record]);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn shutdown_terminates_only_shell_jobs_owned_by_this_process() {
    let dir = tmp_memory_dir("owned_shutdown");
    let store = FileShellJobStore::new(&dir);
    let _ = store.spawn_background("sleep 30", &dir, "session_owned", "turn_a");

    assert_eq!(store.terminate_owned_running(), 1);
    assert!(store.running_for_session("session_owned").is_empty());
    assert_eq!(store.terminate_owned_running(), 0);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn shutdown_ignores_shell_job_record_owned_by_another_process() {
    let dir = tmp_memory_dir("foreign_shutdown");
    let store = FileShellJobStore::new(&dir);
    let status_file = dir.join("foreign.status");
    let record = ShellJobRecord {
        id: "foreign-job".to_string(),
        created_at_ms: now_ms(),
        kind: "background".to_string(),
        session_id: "foreign-session".to_string(),
        turn_id: "foreign-turn".to_string(),
        pid: std::process::id(),
        process_identity: None,
        tool_call_id: "test_call".to_string(),
        owner_id: Some("foreign-runtime-owner".to_string()),
        command: "foreign".to_string(),
        cwd: dir.display().to_string(),
        output_file: dir.join("foreign.out").display().to_string(),
        stderr_file: dir.join("foreign.err").display().to_string(),
        status_file: status_file.display().to_string(),
        tail_out: false,
    };
    store.append(&record).unwrap();

    assert_eq!(store.terminate_owned_running(), 0);
    assert!(!status_file.exists());
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn shutdown_and_session_cancel_refuse_pid_identity_mismatch() {
    let dir = tmp_memory_dir("owned_identity_mismatch");
    let store = FileShellJobStore::new(&dir);
    let status_file = dir.join("identity-mismatch.status");
    let record = ShellJobRecord {
        id: "identity-mismatch-job".to_string(),
        created_at_ms: now_ms(),
        kind: "timeout".to_string(),
        session_id: "identity-session".to_string(),
        turn_id: "identity-turn".to_string(),
        pid: std::process::id(),
        process_identity: Some("not-the-current-process".to_string()),
        tool_call_id: "identity-call".to_string(),
        owner_id: Some(crate::runtime_process_owner_id().to_string()),
        command: "must-not-be-signalled".to_string(),
        cwd: dir.display().to_string(),
        output_file: dir.join("identity-mismatch.out").display().to_string(),
        stderr_file: dir.join("identity-mismatch.err").display().to_string(),
        status_file: status_file.display().to_string(),
        tail_out: false,
    };
    store.append(&record).unwrap();

    assert_eq!(store.terminate_owned_running(), 0);
    assert_eq!(
        fs::read_to_string(&status_file).unwrap(),
        "pid_identity_changed"
    );
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);

    fs::remove_file(&status_file).unwrap();
    assert!(store
        .cancel_unfinished_for_session("identity-session")
        .is_empty());
    assert_eq!(
        fs::read_to_string(&status_file).unwrap(),
        "pid_identity_changed"
    );
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn session_cancel_and_running_list_ignore_foreign_runtime_records() {
    let dir = tmp_memory_dir("foreign_session_record");
    let store = FileShellJobStore::new(&dir);
    let status_file = dir.join("foreign-session.status");
    let record = ShellJobRecord {
        id: "foreign-session-job".to_string(),
        created_at_ms: now_ms(),
        kind: "timeout".to_string(),
        session_id: "shared-session".to_string(),
        turn_id: "foreign-turn".to_string(),
        pid: std::process::id(),
        process_identity: None,
        tool_call_id: "test_call".to_string(),
        owner_id: Some("foreign-runtime-owner".to_string()),
        command: "must-not-be-exposed".to_string(),
        cwd: dir.display().to_string(),
        output_file: dir.join("foreign-session.out").display().to_string(),
        stderr_file: dir.join("foreign-session.err").display().to_string(),
        status_file: status_file.display().to_string(),
        tail_out: false,
    };
    store.append(&record).unwrap();

    assert!(store.running_for_session("shared-session").is_empty());
    assert!(store.running_job_list_context("shared-session").is_none());
    assert!(store
        .cancel_unfinished_for_session("shared-session")
        .is_empty());
    assert!(!status_file.exists());
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn managed_shell_job_pid_is_a_distinct_runtime_child_process_group() {
    let dir = tmp_memory_dir("managed_child_group");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background_outcome(
        "sleep 30",
        &dir,
        "managed-session",
        "managed-turn",
        "test_call",
        false,
    );
    let evidence = started.bash_result.expect("managed child evidence");
    let pid = evidence.pid.expect("managed child pid");

    assert_ne!(pid, std::process::id());
    assert!(is_runtime_child_pid(pid));
    assert_eq!(
        evidence.pid_kind.as_deref(),
        Some("runtime_child_process_group")
    );

    assert_eq!(
        store.cancel_unfinished_for_session("managed-session").len(),
        1
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn terminate_process_ignores_missing_pid_without_signalling_broadly() {
    let missing_pid = i32::MAX as u32;
    terminate_process(missing_pid);
    assert_eq!(unsafe { libc::kill(libc::getpid(), 0) }, 0);
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
fn shell_lifecycle_validation_rejects_unmanaged_background_without_wait() {
    for command in [
        "sleep 30 &",
        "nohup sleep 30 >/tmp/timem-nohup.log 2>&1 &",
        "sleep 30 & echo started",
    ] {
        assert_eq!(
            validate_bash_lifecycle(command, false),
            Err("unmanaged_background_process".to_string()),
            "{command}"
        );
        assert!(validate_bash_lifecycle(command, true).is_ok(), "{command}");
    }
    assert!(
        validate_bash_lifecycle(r#"tail -f /dev/null & child=$!; wait "$child""#, false).is_ok()
    );
    assert_eq!(
        validate_bash_lifecycle("bash -c 'sleep 30 &'", false),
        Err("unmanaged_background_process".to_string())
    );
    assert!(
        validate_bash_lifecycle(r#"bash -c 'sleep 0.1 & child=$!; wait "$child"'"#, false).is_ok()
    );
}

#[test]
fn shell_lifecycle_validation_rejects_explicit_detach() {
    for command in [
        "setsid sleep 30",
        "command setsid sleep 30",
        "nohup setsid sleep 30",
        "env FOO=bar setsid sleep 30",
        "sudo -n -- setsid sleep 30",
        "disown",
        "daemon server",
        "/usr/bin/setsid sleep 30",
        "bash -c 'setsid sleep 30'",
        "/bin/sh -c '/usr/bin/setsid sleep 30'",
        "env FOO=bar bash -lc 'nohup setsid sleep 30'",
    ] {
        assert_eq!(
            validate_bash_lifecycle(command, true),
            Err("explicit_process_detach".to_string()),
            "{command}"
        );
    }
}

#[test]
fn shell_lifecycle_validation_allows_managed_background_and_ampersand_syntax() {
    for command in [
        "sleep 30 &",
        "nohup sleep 30 >/tmp/timem-nohup.log 2>&1 &",
        "printf 'literal & text'",
        "printf ok && printf done",
        "printf err >&2",
        "printf both &>/tmp/timem-output",
        "printf ok |& cat",
        r#"tail -f /dev/null & child=$!; printf '%s' "$child"; wait "$child""#,
    ] {
        assert!(validate_bash_lifecycle(command, true).is_ok(), "{command}");
    }
    for command in [
        "printf 'literal & text'",
        "printf ok && printf done",
        "printf err >&2",
        "printf both &>/tmp/timem-output",
        "printf ok |& cat",
    ] {
        assert!(validate_bash_lifecycle(command, false).is_ok(), "{command}");
    }
}

#[cfg(unix)]
#[test]
fn watcher_waits_for_managed_process_group_after_launcher_exits() {
    let dir = tmp_memory_dir("managed_process_group_lifecycle");
    let store = FileShellJobStore::new(&dir);
    let started = store.spawn_background_outcome(
        "sleep 0.8 &",
        &dir,
        "group-session",
        "group-turn",
        "group-call",
        false,
    );
    let pid = started
        .bash_result
        .and_then(|evidence| evidence.pid)
        .expect("managed launcher pid");

    thread::sleep(Duration::from_millis(200));
    assert!(
        crate::os::process_identity(pid).is_none(),
        "launcher should exit before its background child"
    );
    assert!(crate::os::process_group_running(pid));
    let (running, updates) = store.refresh_for_session("group-session");
    assert_eq!(running.len(), 1, "the process group must remain tracked");
    assert!(updates.is_empty());

    let group_deadline = Instant::now() + Duration::from_secs(3);
    let update = loop {
        let (_, updates) = store.refresh_for_session("group-session");
        if let Some(update) = updates.into_iter().next() {
            break update;
        }
        assert!(
            Instant::now() < group_deadline,
            "managed process group did not finish"
        );
        thread::sleep(Duration::from_millis(30));
    };
    assert_eq!(update.status, "0");
    assert!(!crate::os::process_group_running(pid));
    let _ = fs::remove_dir_all(&dir);
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
        "sudo rm -rf /",
        "sudo -n -- rm -rf /",
        "command rm -rf /",
        "builtin rm -rf /",
        "exec rm -rf /",
        "nohup rm -rf /",
        "env EMPTY= rm -rf \"$EMPTY\"/",
        "env -u EMPTY rm -rf ${EMPTY}/*",
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Failed);
            assert!(
                outcome.text.contains("blocked by Timem safety policy"),
                "{}",
                outcome.text
            );
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Failed);
            assert!(
                outcome.text.contains("blocked by Timem safety policy"),
                "{}",
                outcome.text
            );
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
    assert_eq!(result.status, ActionStatus::Failed);
    assert!(
        result.text.contains("blocked by Timem safety policy"),
        "{}",
        result.text
    );
    assert!(
        result.text.contains("approval_status: approved_by_user"),
        "{}",
        result.text
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
        ActionExecution::Completed(outcome) => {
            assert_eq!(outcome.status, ActionStatus::Completed);
            assert!(outcome.text.contains("Exit code: 0"), "{}", outcome.text);
            assert!(!target.exists(), "safe temp dir should be removable");
        }
        other => panic!("expected safe command to run, got {other:?}"),
    }
}

#[test]
fn foreground_run_bash_preserves_raw_output_and_tail_policy_for_the_gate() {
    let store = FileShellJobStore::new(&tmp_memory_dir("foreground_tail_out"));
    let cwd = tmp_cwd("foreground_tail_out");
    let command =
        "printf BEGIN_MARKER; i=0; while [ $i -lt 33000 ]; do printf x; i=$((i+1)); done; printf END_MARKER"
            .to_string();

    let result = execute_run_bash_with_tail(
        &command,
        &cwd,
        false,
        5000,
        None,
        5000,
        BashApprovalMode::Approve,
        &store,
        "session_tail",
        "turn_tail",
        "test_call",
        true,
        true,
        &mut NeverCancelRuntime,
    );
    let ActionExecution::Completed(outcome) = result else {
        panic!("approve mode should execute directly");
    };

    assert_eq!(outcome.status, ActionStatus::Completed);
    let rendered_output = outcome
        .text
        .split_once("Return:\n")
        .expect("finished result should contain a return section")
        .1;
    assert!(!rendered_output.contains("truncated"), "{rendered_output}");
    assert!(rendered_output.contains("END_MARKER"), "{rendered_output}");
    assert!(
        rendered_output.contains("BEGIN_MARKER"),
        "{rendered_output}"
    );
}

#[test]
fn polling_result_preserves_raw_output_for_the_gate() {
    let cwd = tmp_cwd("polling_tail_out");
    let command =
        "printf BEGIN_MARKER; i=0; while [ $i -lt 33000 ]; do printf x; i=$((i+1)); done; printf END_MARKER; exit 0"
            .to_string();
    let outcome = execute_polling_bash_outcome_with_tail(
        &command,
        &cwd,
        10,
        5000,
        5000,
        true,
        &mut NeverCancelRuntime,
    );

    assert_eq!(outcome.status, ActionStatus::Completed);
    let last_output = outcome
        .text
        .split_once("Last output:\n")
        .expect("polling result should contain a last-output section")
        .1;
    assert!(!last_output.contains("truncated"), "{last_output}");
    assert!(last_output.contains("END_MARKER"), "{last_output}");
    assert!(last_output.contains("BEGIN_MARKER"), "{last_output}");
}

#[test]
fn background_tail_out_is_persisted_until_exit_refresh() {
    let dir = tmp_memory_dir("background_tail_out");
    let store = FileShellJobStore::new(&dir);
    let command =
        "printf BEGIN_MARKER; i=0; while [ $i -lt 33000 ]; do printf x; i=$((i+1)); done; printf END_MARKER"
            .to_string();
    let started = store.spawn_background_outcome(
        &command,
        &dir,
        "session_tail_background",
        "turn_tail_background",
        "test_call",
        true,
    );
    assert_eq!(started.status, ActionStatus::BackgroundRunning);

    let record = store
        .guard
        .with_read(|| store.records_unlocked().into_iter().next())
        .unwrap()
        .expect("persisted background record");
    assert!(record.tail_out);

    let wait_started = Instant::now();
    let update = loop {
        let (_, updates) = store.refresh_for_session("session_tail_background");
        if let Some(update) = updates.into_iter().next() {
            break update;
        }
        assert!(
            wait_started.elapsed() < Duration::from_secs(5),
            "tail background command did not finish"
        );
        thread::sleep(Duration::from_millis(20));
    };

    assert!(!update.output.contains("truncated"), "{}", update.output);
    assert!(update.output.contains("END_MARKER"), "{}", update.output);
    assert!(update.output.contains("BEGIN_MARKER"), "{}", update.output);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn approval_pending_action_preserves_tail_out() {
    let store = FileShellJobStore::new(&tmp_memory_dir("approval_tail_out"));
    let cwd = tmp_cwd("approval_tail_out");
    let result = execute_run_bash_with_tail(
        "printf approved",
        &cwd,
        false,
        5000,
        None,
        5000,
        BashApprovalMode::Ask,
        &store,
        "session_approval_tail",
        "turn_approval_tail",
        "test_call",
        true,
        true,
        &mut NeverCancelRuntime,
    );

    let ActionExecution::NeedsApproval(pending) = result else {
        panic!("ask mode should return an approval request");
    };
    match pending.approved_action {
        PendingApprovedAction::RunBash { tail_out, .. } => assert!(tail_out),
        other => panic!("unexpected pending action: {other:?}"),
    }
}

#[test]
fn finished_shell_job_temporary_items_are_sized_and_deleted_as_complete_bundles() {
    let dir = tmp_memory_dir("finished_temporary_items");
    let shell_dir = dir.join("shell_jobs");
    fs::create_dir_all(&shell_dir).unwrap();
    let finished_output = shell_dir.join("finished.out");
    let finished_stderr = shell_dir.join("finished.err");
    let finished_status = shell_dir.join("finished.status");
    let finished_notified = shell_dir.join("finished.status.notified");
    let active_output = shell_dir.join("active.out");
    let active_stderr = shell_dir.join("active.err");
    let active_status = shell_dir.join("active.status");
    for (path, contents) in [
        (&finished_output, "12345"),
        (&finished_stderr, "678"),
        (&finished_status, "0"),
        (&finished_notified, "ok"),
        (&active_output, "active"),
        (&active_stderr, ""),
        (&active_status, ""),
    ] {
        fs::write(path, contents).unwrap();
    }
    let record = |id: &str, output: &Path, stderr: &Path, status: &Path| ShellJobRecord {
        id: id.to_string(),
        created_at_ms: 42,
        kind: "test".to_string(),
        session_id: "session_a".to_string(),
        turn_id: "turn_a".to_string(),
        pid: std::process::id(),
        process_identity: None,
        tool_call_id: format!("call-{id}"),
        owner_id: None,
        command: "true".to_string(),
        cwd: dir.display().to_string(),
        output_file: output.display().to_string(),
        stderr_file: stderr.display().to_string(),
        status_file: status.display().to_string(),
        tail_out: false,
    };
    let finished = record(
        "finished-job",
        &finished_output,
        &finished_stderr,
        &finished_status,
    );
    let active = record("active-job", &active_output, &active_stderr, &active_status);
    fs::write(
        shell_dir.join("jobs.jsonl"),
        format!(
            "{}\n{}\n",
            serde_json::to_string(&finished).unwrap(),
            serde_json::to_string(&active).unwrap()
        ),
    )
    .unwrap();

    let store = FileShellJobStore::new(&dir);
    let items = store.finished_temporary_items().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "finished-job");
    assert_eq!(items[0].bytes, 11);
    assert!(store
        .delete_finished_temporary_items(&["active-job".to_string()])
        .is_err());
    assert_eq!(
        store
            .delete_finished_temporary_items(&["finished-job".to_string()])
            .unwrap(),
        1
    );
    for path in [
        &finished_output,
        &finished_stderr,
        &finished_status,
        &finished_notified,
    ] {
        assert!(!path.exists(), "{} should be deleted", path.display());
    }
    assert!(active_output.exists());
    let index = fs::read_to_string(shell_dir.join("jobs.jsonl")).unwrap();
    assert!(!index.contains("finished-job"));
    assert!(index.contains("active-job"));
    let _ = fs::remove_dir_all(dir);
}

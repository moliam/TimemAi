use super::*;

fn temp_data_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "timem-web-lifecycle-{label}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

#[test]
fn arguments_record_only_option_names_and_resolve_mem_root() {
    let args = vec![
        "--api-key".into(),
        "sk-secret".into(),
        "--base-url=https://private.invalid".into(),
        "positional-private".into(),
        "--debug".into(),
    ];
    assert_eq!(
        sanitized_option_names(&args),
        vec!["--api-key", "--base-url", "--debug"]
    );
    let memory_root = temp_data_root("arguments-mem");
    assert_eq!(
        memory_root_from_args(&[format!("--space={}", memory_root.display())]).unwrap(),
        memory_root
    );
    assert!(memory_root_from_args(&["--data-dir=/tmp/removed".into()])
        .unwrap_err()
        .contains("unsupported_option:--data-dir"));
}

#[test]
fn event_ring_is_strictly_bounded_and_keeps_newest_entries() {
    let data_root = temp_data_root("ring");
    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    for index in 0..(EVENT_LIMIT + 10) {
        diagnostics.event("tick", serde_json::json!({"index": index}));
    }
    let events = snapshot_events(&diagnostics.inner);
    assert_eq!(events.len(), EVENT_LIMIT);
    assert_eq!(events.last().unwrap().details["index"], EVENT_LIMIT + 9);
    diagnostics.finish("test_complete", true, None);
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn checkpoint_updates_the_bounded_running_snapshot() {
    let data_root = temp_data_root("checkpoint");
    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    diagnostics.checkpoint("configuration_parsed", serde_json::Value::Null);
    diagnostics.checkpoint("listener_bound", serde_json::json!({"port": 15001}));
    let current: serde_json::Value =
        serde_json::from_slice(&fs::read(&diagnostics.inner.current_path).unwrap()).unwrap();
    let events = current["recent_lifecycle_events"].as_array().unwrap();
    assert_eq!(events.last().unwrap()["name"], "listener_bound");
    assert_eq!(events.last().unwrap()["details"]["port"], 15001);
    assert!(current["last_checkpoint_at_ms"].as_u64().is_some());
    assert_eq!(current["argument_options"], serde_json::json!([]));
    diagnostics.finish("test_complete", true, None);
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn clean_finish_persists_reason_events_and_removes_running_marker() {
    let data_root = temp_data_root("finish");
    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let root = diagnostics.root().unwrap().to_path_buf();
    diagnostics.event("listener_bound", serde_json::json!({"port": 15001}));
    diagnostics.finish("ctrl_c", true, None);
    assert!(!diagnostics.inner.current_path.exists());
    let exit: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(LAST_EXIT_FILE)).unwrap()).unwrap();
    assert_eq!(exit["exit_reason"], "ctrl_c");
    assert_eq!(exit["graceful"], true);
    assert_eq!(
        exit["recent_lifecycle_events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["name"],
        "listener_bound"
    );
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn live_run_markers_are_not_promoted_or_overwritten() {
    let data_root = temp_data_root("concurrent-live");
    let first = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let first_path = first.inner.current_path.clone();
    let second = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let second_path = second.inner.current_path.clone();

    assert_ne!(first_path, second_path);
    assert!(first_path.is_file());
    assert!(second_path.is_file());
    assert!(!first.inner.root.join(PREVIOUS_ABNORMAL_FILE).exists());

    second.finish("second_complete", true, None);
    assert!(
        first_path.is_file(),
        "one run must not remove another marker"
    );
    assert!(!second_path.exists());
    first.finish("first_complete", true, None);
    assert!(!first_path.exists());
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn definitely_stale_run_is_promoted_without_claiming_a_specific_cause() {
    let data_root = temp_data_root("abnormal");
    let root = data_root.join(DIAGNOSTICS_DIR);
    create_private_dir(&root).unwrap();
    create_private_dir(&root.join(CURRENT_RUNS_DIR)).unwrap();
    create_private_dir(&root.join(RUN_ARCHIVE_DIR)).unwrap();
    let stale = root.join(CURRENT_RUNS_DIR).join("stale.json");
    atomic_json_write(
        &stale,
        &serde_json::json!({
            "schema_version": 1,
            "run_id": "dead-run",
            "pid": std::process::id(),
            "process_identity": "deliberately-not-the-current-process"
        }),
    )
    .unwrap();

    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let previous: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(PREVIOUS_ABNORMAL_FILE)).unwrap()).unwrap();
    assert_eq!(previous["status"], "abnormal_exit_detected_on_next_start");
    assert_eq!(previous["exact_cause"], "unknown");
    assert!(!stale.exists());
    diagnostics.finish("test_complete", true, None);
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn corrupt_running_marker_is_salvaged_as_unknown_abnormal_exit() {
    let data_root = temp_data_root("corrupt-current");
    let root = data_root.join(DIAGNOSTICS_DIR);
    create_private_dir(&root).unwrap();
    create_private_dir(&root.join(CURRENT_RUNS_DIR)).unwrap();
    create_private_dir(&root.join(RUN_ARCHIVE_DIR)).unwrap();
    fs::write(
        root.join(CURRENT_RUNS_DIR).join("corrupt.json"),
        b"{not valid json",
    )
    .unwrap();

    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let previous: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(PREVIOUS_ABNORMAL_FILE)).unwrap()).unwrap();
    assert_eq!(previous["schema_version"], 1);
    assert_eq!(previous["status"], "abnormal_exit_detected_on_next_start");
    assert_eq!(previous["exact_cause"], "unknown");
    diagnostics.finish("test_complete", true, None);
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn errors_are_redacted_utf8_safe_and_bounded() {
    let source = format!(
        "Bearer private-token api_key=private sk-private {} 中文",
        "x".repeat(ERROR_LIMIT * 2)
    );
    let result = redact_and_bound(&source);
    assert!(!result.contains("private-token"));
    assert!(!result.contains("api_key=private"));
    assert!(!result.contains("sk-private"));
    assert!(result.len() <= ERROR_LIMIT + "\n[truncated]".len());
    assert_eq!(bound_text("你好吗", 4), "你\n[truncated]");
    assert_eq!(redact_and_bound("Bearer line-end-secret"), "[REDACTED]");
    assert_eq!(
        redact_and_bound("prefix api-key=value\nnext"),
        "prefix [REDACTED]\nnext"
    );
    assert_eq!(redact_and_bound("sk-secret-at-end"), "[REDACTED]");
}

#[test]
fn panic_event_capture_never_waits_on_a_busy_event_lock() {
    let data_root = temp_data_root("panic-lock");
    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let guard = diagnostics.inner.events.lock().unwrap();
    try_push_panic_event(&diagnostics.inner);
    assert_eq!(
        guard.len(),
        1,
        "busy panic capture must skip instead of deadlocking"
    );
    drop(guard);
    diagnostics.finish("test_complete", true, None);
    fs::remove_dir_all(data_root).unwrap();
}

#[test]
fn panic_report_contains_bounded_backtrace_events_and_redacted_message() {
    let report = render_panic_report(
        "run-1",
        42,
        123,
        "worker",
        "src/server.rs:9:3",
        "failed with Bearer private-token",
        r#"[{"name":"listener_bound"}]"#,
        &"frame\n".repeat(BACKTRACE_LIMIT),
    );
    assert!(report.contains("run_id: run-1"));
    assert!(report.contains("thread: worker"));
    assert!(report.contains("location: src/server.rs:9:3"));
    assert!(report.contains("listener_bound"));
    assert!(!report.contains("private-token"));
    assert!(report.contains("[truncated]"));
    assert!(report.len() < BACKTRACE_LIMIT + 2_000);
}

#[test]
fn repeated_finish_is_idempotent() {
    let data_root = temp_data_root("idempotent");
    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let root = diagnostics.root().unwrap().to_path_buf();
    diagnostics.finish("first", true, None);
    diagnostics.finish("second", false, Some("must not replace"));
    let exit: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(LAST_EXIT_FILE)).unwrap()).unwrap();
    assert_eq!(exit["exit_reason"], "first");
    fs::remove_dir_all(data_root).unwrap();
}

#[cfg(unix)]
#[test]
fn artifacts_are_owner_only_and_atomic_writes_leave_no_temp_files() {
    use std::os::unix::fs::PermissionsExt;
    let data_root = temp_data_root("permissions");
    let diagnostics = LifecycleDiagnostics::install_in(&data_root, &[], false).unwrap();
    let root = diagnostics.root().unwrap().to_path_buf();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&diagnostics.inner.current_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    diagnostics.finish("test_complete", true, None);
    assert_eq!(
        fs::metadata(root.join(LAST_EXIT_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("tmp-")));
    fs::remove_dir_all(data_root).unwrap();
}

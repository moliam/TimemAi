use super::*;

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "timem-runtime-log-{label}-{}-{}.log",
        std::process::id(),
        now_ms()
    ))
}

#[test]
fn diagnostic_root_places_runtime_log_beside_optional_debug_artifacts() {
    let root = Arc::new(crate::debug_session::TemporaryDebugRoot::create().unwrap());
    let root_path = root.path().to_path_buf();
    assert!(root_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with(&format!("timem-debug-{}-", std::process::id())));

    let log = RuntimeLog::with_diagnostic_root(root.clone());
    assert_eq!(log.path(), Some(root_path.join("runtime.log").as_path()));
    log.record("sample", json!({ "kind": "performance" }));
    assert!(root_path.join("runtime.log").exists());

    let debug = crate::debug_session::DebugStore::with_root(root);
    assert_eq!(debug.root(), root_path.as_path());
    drop(log);
    assert!(root_path.exists(), "DebugStore still owns the shared root");
    drop(debug);
    assert!(!root_path.exists(), "last owner cleans the temporary root");
}

#[test]
fn disabled_trace_has_no_path_and_performs_no_io() {
    let trace = RuntimeLog::default();
    assert!(!trace.enabled());
    assert!(trace.path().is_none());
    trace.record("ignored", json!({}));
}

#[test]
fn runtime_log_records_structured_lines_with_process_scope() {
    let path = temp_path("jsonl");
    let trace = RuntimeLog::with_path_and_limit(path.clone(), 4096);
    trace.record("sample", json!({ "session_id": "session-a" }));
    let text = fs::read_to_string(&path).unwrap();
    let value: Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(value["pid"], std::process::id());
    assert_eq!(value["stage"], "sample");
    assert_eq!(value["fields"]["session_id"], "session-a");
    let _ = fs::remove_file(path);
}

#[test]
fn trace_wraps_in_place_without_exceeding_limit() {
    let path = temp_path("wrap");
    let limit = 300;
    let trace = RuntimeLog::with_path_and_limit(path.clone(), limit);
    for ordinal in 0..20 {
        trace.record(
            "sample",
            json!({ "ordinal": ordinal, "padding": "x".repeat(80) }),
        );
        assert!(fs::metadata(&path).unwrap().len() <= limit);
    }
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("log_wrapped"));
    let _ = fs::remove_file(path);
}

#[test]
fn oversized_single_record_is_dropped() {
    let path = temp_path("oversized");
    let trace = RuntimeLog::with_path_and_limit(path.clone(), 128);
    trace.record("sample", json!({ "padding": "x".repeat(1024) }));
    assert!(!path.exists());
}

#[test]
fn client_trace_rejects_unknown_stages_and_invalid_ids() {
    let path = temp_path("validation");
    let trace = RuntimeLog::with_path_and_limit(path, 4096);
    assert!(trace
        .record_client("unknown", "s".into(), "c".into(), None, None, None)
        .is_err());
    assert!(trace
        .record_client(
            "browser_send",
            "s".into(),
            "bad\nid".into(),
            None,
            None,
            None
        )
        .is_err());
    assert!(trace
        .record_client(
            "browser_send",
            "s".repeat(257),
            "c".into(),
            None,
            None,
            None
        )
        .is_err());
    assert!(trace
        .record_client(
            "browser_painted",
            "s".into(),
            "c".into(),
            Some("bad\nturn".into()),
            Some(1.0),
            Some(1)
        )
        .is_err());
    assert!(trace
        .record_client(
            "browser_painted",
            "s".into(),
            "c".into(),
            None,
            Some(-1.0),
            None
        )
        .is_err());
    assert!(trace
        .record_client(
            "browser_painted",
            "s".into(),
            "c".into(),
            None,
            Some(1.0),
            Some(1_000_001)
        )
        .is_err());
}

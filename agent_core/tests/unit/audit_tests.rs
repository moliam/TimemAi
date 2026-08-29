use super::*;

#[test]
fn append_audit_writes_json_document() {
    let mut path = std::env::temp_dir();
    path.push(format!("timem_core_audit_{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);

    append_audit_event(&path, &json!({"type":"turn_final","ok":true})).unwrap();
    append_audit_event(&path, &json!({"type":"llm_request","ok":true})).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["version"], 1);
    let events = doc["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "turn_final");
    assert_eq!(events[1]["type"], "llm_request");
    assert!(events.iter().all(|event| event["time_ms"].is_i64()));
    let _ = std::fs::remove_file(path);
}

#[test]
fn model_input_overflow_recovery_event_keeps_delta_and_size_evidence() {
    let event = model_input_overflow_recovery_audit_event(
        "session_1",
        "turn_1",
        "pd_7",
        131_072,
        "model_http_413: payload too large",
    );
    assert_eq!(event["type"], "model_input_overflow_recovery");
    assert_eq!(event["session"], "session_1");
    assert_eq!(event["turn_id"], "turn_1");
    assert_eq!(event["removed_delta_id"], "pd_7");
    assert_eq!(event["removed_action_output_bytes"], 131_072);
    assert_eq!(event["error"], "model_http_413: payload too large");
}

#[test]
fn append_audit_migrates_legacy_jsonl_without_applying_retention_policy() {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "timem_core_legacy_audit_{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "{\"type\":\"turn_start\",\"ok\":true}\n").unwrap();

    append_audit_event(&path, &json!({"type":"turn_final","ok":true})).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["version"], 1);
    let events = doc["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "turn_start");
    assert!(events[0].get("time_ms").is_none());
    assert_eq!(events[1]["type"], "turn_final");
    assert!(events[1]["time_ms"].is_i64());
    assert!(!text.lines().next().unwrap().starts_with(r#"{"type""#));
    let _ = std::fs::remove_file(path);
}

#[test]
fn append_audit_uses_jsonl_sidecar_for_large_documents_and_read_merges_events() {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "timem_core_large_audit_{}.json",
        std::process::id()
    ));
    let sidecar = audit_sidecar_path(&path);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&sidecar);

    let large_text = "x".repeat(AUDIT_SIDECAR_THRESHOLD_BYTES as usize);
    let now_ms = audit_now_ms();
    std::fs::write(
        &path,
        serde_json::to_string(&json!({
            "version": 1,
            "events": [{"type":"seed", "payload": large_text, "time_ms": now_ms}]
        }))
        .unwrap(),
    )
    .unwrap();

    append_audit_event(&path, &json!({"type":"turn_final","ok":true})).unwrap();

    assert!(segmented_directory(&sidecar).exists());
    let sidecar_text = read_segmented_records(&sidecar)
        .unwrap()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert!(String::from_utf8(sidecar_text)
        .unwrap()
        .contains("\"turn_final\""));
    let doc = read_audit_doc(&path).unwrap();
    let events = doc["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "seed");
    assert_eq!(events[1]["type"], "turn_final");

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(sidecar);
}

#[test]
fn turn_audit_event_builders_keep_runtime_schema_in_core() {
    let stats = UsageStats {
        llm_calls: 2,
        prompt_tokens: 120,
        completion_tokens: 20,
        ..UsageStats::zero()
    };
    let latest = UsageStats {
        prompt_tokens: 80,
        completion_tokens: 8,
        ..UsageStats::zero()
    };

    let start = turn_start_audit_event("s", "t", "hello");
    assert_eq!(start["type"], "turn_start");
    assert_eq!(start["user_input"], "hello");

    let host_start = host_start_audit_event(
        "shell",
        "s",
        ".test_mem",
        "https://example.test/v1",
        &crate::ApiProtocol::OpenAiCompatible,
        "qwen-plus",
        100_000,
        crate::BashApprovalMode::Approve,
    );
    assert_eq!(host_start["type"], "shell_start");
    assert_eq!(host_start["api_protocol"], "openai-compatible");
    assert_eq!(host_start["bash_approval"], "approve");

    let final_event = turn_final_audit_event(
        "s",
        "t",
        "done",
        &stats,
        Some(&latest),
        Some("repair_issue"),
        None,
        Duration::from_millis(123),
    );
    assert_eq!(final_event["type"], "turn_final");
    assert_eq!(final_event["stats"]["llm_calls"], 2);
    assert_eq!(final_event["latest_usage"]["prompt_tokens"], 80);
    assert_eq!(final_event["repair_issue"], "repair_issue");
    assert_eq!(final_event["stop_summary"], Value::Null);
    assert_eq!(final_event["elapsed_ms"], 123);
}

#[test]
fn action_related_audit_event_builders_are_structured() {
    let approval = ApprovalRequest {
        approval_id: "approval_1".into(),
        action: "run_bash".into(),
        command: "true".into(),
        reason: "ask mode".into(),
        risk: "user_approval_required".into(),
    };

    let approval_event = user_approval_audit_event("s", "t", &approval, true);
    assert_eq!(approval_event["type"], "user_approval");
    assert_eq!(approval_event["approval_id"], "approval_1");
    assert_eq!(approval_event["approved"], true);

    let retry = model_retry_audit_event("s", "t", 1, 5, Duration::from_secs(10), "model_http_500");
    assert_eq!(retry["type"], "model_retry");
    assert_eq!(retry["delay_ms"], 10_000);

    let stale = stale_context_choice_audit_event("s", Duration::from_secs(7), 12_345, false);
    assert_eq!(stale["type"], "stale_context_choice");
    assert_eq!(stale["session"], "s");
    assert_eq!(stale["idle_secs"], 7);
    assert_eq!(stale["dynamic_context_tokens"], 12_345);
    assert_eq!(stale["continue_old_context"], false);

    let repair = model_repair_request_audit_event(
        "s",
        "t",
        Some("invalid_json"),
        "m",
        &UsageStats::zero(),
        true,
        3,
        1,
    );
    assert_eq!(repair["type"], "model_repair_request");
    assert_eq!(repair["issue"], "invalid_json");
    assert_eq!(repair["truncated"], true);
    assert_eq!(repair["repair_calls_delta"], 1);

    let repair_output = model_repair_output_event(
        "s",
        "t",
        Some("invalid_json"),
        "Ai1",
        "<ASSISTANT>bad</ASSISTANT>\n[BEGIN DELTA]",
        "Ai1's previous response is not protocol compliant.\nerror: invalid_json",
        "m",
        &UsageStats::zero(),
        false,
        3,
        1,
        &crate::response_protocol::XML_PROMPT_BOUNDARIES,
    );
    assert_eq!(repair_output["kind"], "model_output_repair");
    assert_eq!(repair_output["assistant_name"], "Ai1");
    assert!(repair_output["rendered"]
        .as_str()
        .unwrap()
        .contains("## ASSISTANT:\n<ASSISTANT>bad</ASSISTANT>"));
    assert!(repair_output["rendered"]
        .as_str()
        .unwrap()
        .contains("## RUNTIME\nAi1's previous response"));
}

#[test]
fn timestamp_audit_event_preserves_past_times_and_normalizes_invalid_values() {
    let now_ms = 200_000;
    let fresh_ms = now_ms - 1;
    assert_eq!(
        timestamp_audit_event(&json!({"type":"fresh", "time_ms":fresh_ms}), now_ms)["time_ms"],
        fresh_ms
    );
    assert_eq!(
        timestamp_audit_event(&json!({"type":"old", "time_ms":1}), now_ms)["time_ms"],
        1
    );
    assert_eq!(
        timestamp_audit_event(&json!({"type":"future", "time_ms":now_ms + 1}), now_ms)["time_ms"],
        now_ms
    );
    let scalar = timestamp_audit_event(&json!("legacy"), now_ms);
    assert_eq!(scalar["time_ms"], now_ms);
    assert_eq!(scalar["event"], "legacy");
}

#[test]
fn api_audit_retention_uses_the_requested_rolling_cutoff_across_json_and_jsonl() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_audit_retention_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("api_audit.json");
    let sidecar = audit_sidecar_path(&path);
    let now_ms = 300_000;
    let cutoff_ms = 200_000;

    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "version": 1,
            "events": [
                {"type":"base_old", "time_ms":cutoff_ms - 1},
                {"type":"base_boundary", "time_ms":cutoff_ms},
                {"type":"base_missing"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut sidecar_bytes = Vec::new();
    sidecar_bytes.extend_from_slice(
        format!(
            "{{\"type\":\"sidecar_fresh\",\"time_ms\":{}}}\n",
            now_ms - 1
        )
        .as_bytes(),
    );
    sidecar_bytes.extend_from_slice(b"not json\n");
    sidecar_bytes.extend_from_slice(b"\xff\xfe\n");
    sidecar_bytes.extend_from_slice(
        format!(
            "{{\"type\":\"sidecar_future\",\"time_ms\":{}}}\n",
            now_ms + 1
        )
        .as_bytes(),
    );
    sidecar_bytes.extend_from_slice(b"{\"type\":\"sidecar_missing\"}\n");
    std::fs::write(&sidecar, sidecar_bytes).unwrap();

    assert_eq!(prune_api_audit_before(&path, cutoff_ms, now_ms).unwrap(), 6);
    assert_eq!(
        prune_api_audit_before(&path, cutoff_ms, now_ms).unwrap(),
        0,
        "repeating the same rolling cleanup must be idempotent"
    );

    let doc = read_audit_doc(&path).unwrap();
    let types = doc["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(types, vec!["base_boundary", "sidecar_fresh"]);
    let sidecar_text = read_segmented_records(&sidecar)
        .unwrap()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let sidecar_text = String::from_utf8(sidecar_text).unwrap();
    assert!(sidecar_text.contains("sidecar_fresh"));
    assert!(!sidecar_text.contains("sidecar_future"));
    assert!(!sidecar_text.contains("sidecar_missing"));
    assert!(!root
        .join(format!(
            ".api_audit.jsonl.retention.tmp-{}-{}",
            std::process::id(),
            now_ms
        ))
        .exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn jsonl_tail_compaction_keeps_only_complete_latest_records() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_audit_tail_compaction_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("api_audit.jsonl");
    use std::fmt::Write as _;
    let mut lines = String::new();
    for index in 0..12 {
        writeln!(
            lines,
            "{{\"index\":{index},\"payload\":\"{}\"}}",
            "x".repeat(20)
        )
        .unwrap();
    }
    std::fs::write(&path, lines).unwrap();

    compact_jsonl_tail_in_place(&path, 150).unwrap();

    let retained = std::fs::read_to_string(&path).unwrap();
    let records = retained
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(!records.is_empty());
    assert_eq!(records.last().unwrap()["index"], 11);
    assert!(records.first().unwrap()["index"].as_u64().unwrap() > 0);
    assert!(std::fs::metadata(&path).unwrap().len() <= 150);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn oversized_audit_event_is_replaced_by_bounded_summary() {
    let event = json!({
        "type": "llm_request",
        "session": "session_1",
        "payload": "x".repeat(AUDIT_EVENT_MAX_BYTES + 1),
    });
    let bounded = bounded_audit_event(event).unwrap();
    assert_eq!(bounded["type"], "llm_request");
    assert_eq!(bounded["session"], "session_1");
    assert_eq!(bounded["payload_omitted"], true);
    assert!(bounded["payload_bytes"].as_u64().unwrap() > AUDIT_EVENT_MAX_BYTES as u64);
    assert!(serde_json::to_vec(&bounded).unwrap().len() < 1024);
}

#[test]
fn audit_budget_cleanup_removes_interrupted_retention_files() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_audit_temp_cleanup_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let stale = root.join(".api_audit.jsonl.retention.tmp-1-2");
    let unrelated = root.join("keep.txt");
    std::fs::write(&stale, b"stale").unwrap();
    std::fs::write(&unrelated, b"keep").unwrap();

    cleanup_stale_audit_temps(&root).unwrap();

    assert!(!stale.exists());
    assert!(unrelated.exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_base_audit_document_keeps_schema_and_latest_events_when_compacted() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_legacy_base_audit_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("api_audit.json");
    let events = (0..20)
        .map(|index| json!({"type":"legacy", "index":index, "payload":"x".repeat(80)}))
        .collect::<Vec<_>>();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({"version":1, "events":events})).unwrap(),
    )
    .unwrap();

    compact_json_array_document(&path, "events", 700).unwrap();

    let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc["version"], 1);
    let retained = doc["events"].as_array().unwrap();
    assert!(!retained.is_empty());
    assert_eq!(retained.last().unwrap()["index"], 19);
    assert!(retained.first().unwrap()["index"].as_u64().unwrap() > 0);
    assert!(std::fs::metadata(&path).unwrap().len() <= 700);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_root_sidecar_remains_readable_and_counts_toward_workspace_budget() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_legacy_root_sidecar_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    let audit_dir = root.join("audit");
    std::fs::create_dir_all(&audit_dir).unwrap();
    let legacy = root.join("api_audit.jsonl");
    let current = audit_dir.join("api_audit.jsonl");
    std::fs::write(&legacy, b"{\"type\":\"legacy\"}\n").unwrap();
    std::fs::write(&current, b"{\"type\":\"current\"}\n").unwrap();

    assert_eq!(
        audit_storage_bytes(&audit_dir, Some(&legacy)).unwrap(),
        std::fs::metadata(&legacy).unwrap().len() + std::fs::metadata(&current).unwrap().len()
    );
    let legacy_doc = read_audit_doc_single(&legacy).unwrap();
    assert_eq!(legacy_doc["events"][0]["type"], "legacy");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn audit_age_prune_deletes_expired_slice_without_rewriting_fresh_slice() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_audit_slice_prune_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("api_audit.jsonl");
    let directory = segmented_directory(&path);
    std::fs::create_dir_all(&directory).unwrap();
    let expired = directory.join("segment-0000000000000001.jsonl");
    let fresh = directory.join("segment-0000000000000002.jsonl");
    std::fs::write(
        &expired,
        b"{\"type\":\"old_1\",\"time_ms\":10}\n{\"type\":\"old_2\",\"time_ms\":20}\n",
    )
    .unwrap();
    std::fs::write(
        &fresh,
        b"{\"type\":\"new_1\",\"time_ms\":100}\n{\"type\":\"new_2\",\"time_ms\":110}\n",
    )
    .unwrap();
    let fresh_bytes = std::fs::read(&fresh).unwrap();
    let fresh_modified = std::fs::metadata(&fresh).unwrap().modified().unwrap();

    assert_eq!(prune_audit_jsonl(&path, 50, 120).unwrap(), 2);

    assert!(!expired.exists());
    assert_eq!(std::fs::read(&fresh).unwrap(), fresh_bytes);
    assert_eq!(
        std::fs::metadata(&fresh).unwrap().modified().unwrap(),
        fresh_modified
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn audit_segment_summary_handles_out_of_order_event_times() {
    let root = std::env::temp_dir().join(format!(
        "timem_core_audit_unordered_slice_{}_{}",
        std::process::id(),
        audit_now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("api_audit.jsonl");
    let directory = segmented_directory(&path);
    std::fs::create_dir_all(&directory).unwrap();
    let mixed = directory.join("segment-0000000000000001.jsonl");
    std::fs::write(
        &mixed,
        b"{\"type\":\"fresh_first\",\"time_ms\":100}\n{\"type\":\"expired_middle\",\"time_ms\":10}\n{\"type\":\"fresh_last\",\"time_ms\":110}\n",
    )
    .unwrap();

    assert_eq!(prune_audit_jsonl(&path, 50, 120).unwrap(), 1);

    let retained = std::fs::read_to_string(&mixed).unwrap();
    assert!(retained.contains("fresh_first"));
    assert!(retained.contains("fresh_last"));
    assert!(!retained.contains("expired_middle"));
    let summary = audit_segment_summary(&mixed).unwrap().unwrap();
    assert_eq!(summary.records, 2);
    assert_eq!(summary.min_time_ms, 100);
    assert_eq!(summary.max_time_ms, 110);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn audit_capacity_uses_16_mib_slices_and_reserves_one_for_writes() {
    use crate::rolling_file_store::RollingCapacity;

    const UNIT: u64 = 16 * 1024 * 1024;
    let normal =
        RollingCapacity::with_slice_bytes(DEFAULT_AUDIT_DIRECTORY_MAX_BYTES, UNIT).unwrap();
    let debug = RollingCapacity::with_slice_bytes(DEBUG_AUDIT_DIRECTORY_MAX_BYTES, UNIT).unwrap();

    assert_eq!(DEFAULT_AUDIT_DIRECTORY_MAX_BYTES, 4 * UNIT);
    assert_eq!(normal.stable_bytes, 3 * UNIT);
    assert_eq!(normal.reserved_bytes, UNIT);
    assert_eq!(DEBUG_AUDIT_DIRECTORY_MAX_BYTES, 32 * UNIT);
    assert_eq!(debug.stable_bytes, 31 * UNIT);
    assert_eq!(debug.reserved_bytes, UNIT);
    assert_eq!(API_AUDIT_BASE_MAX_BYTES, UNIT);
    assert_eq!(ACTION_AUDIT_MAX_BYTES, UNIT);
    assert_eq!(REPAIR_OUTPUT_MAX_BYTES, UNIT);
}

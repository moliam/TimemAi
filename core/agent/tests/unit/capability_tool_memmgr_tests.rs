use super::*;

#[test]
fn scratch_kind_aliases_are_normalized() {
    assert_eq!(normalize_scratch_kind("note"), "notes");
    assert_eq!(normalize_scratch_kind("custom"), "custom");
}

#[test]
fn raw_chat_search_status_is_independent_of_result_text() {
    use crate::{ActionStatus, CoreProfile};
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "timem_memmgr_status_words_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    let audit_file = core.chat_history.audit_file.clone();
    fs::create_dir_all(audit_file.parent().unwrap()).unwrap();
    let events = [
        json!({
            "type": "turn_start",
            "session": "status-test",
            "turn_id": "turn_1786970000000",
            "user_input": "search marker STATUS-PAYLOAD-42"
        }),
        json!({
            "type": "turn_final",
            "session": "status-test",
            "turn_id": "turn_1786970000000",
            "assistant_output": "readfile timed out; error: example; cancelled"
        }),
    ];
    fs::write(
        &audit_file,
        events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();

    let action = ParsedAction {
        action: "memmgr".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "global",
            "search_text": "STATUS-PAYLOAD-42",
            "limit": 5
        }),
    };
    let outcome = execute_outcome(&mut core, &action);

    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(
        outcome.text.contains("readfile timed out"),
        "{}",
        outcome.text
    );
    assert!(outcome.text.contains("error: example"), "{}", outcome.text);
    assert!(outcome.text.contains("cancelled"), "{}", outcome.text);
    let evidence = outcome.memmgr_result.expect("memmgr evidence");
    assert_eq!(evidence.memory_type, "raw_chat");
    assert_eq!(evidence.op, "search");
    assert_eq!(evidence.error_type, None);
    assert!(evidence.content.contains("STATUS-PAYLOAD-42"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_reports_real_timestamps_for_sequence_turn_ids() {
    use crate::{ActionStatus, CoreProfile};
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "timem_memmgr_turn_ts_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    let audit_file = core.chat_history.audit_file.clone();
    fs::create_dir_all(audit_file.parent().unwrap()).unwrap();
    // Real audit turn ids are `turn_<start_millis>_<sequence>` and every
    // persisted event carries `time_ms`; both must surface as the record time.
    let events = [
        json!({
            "type": "turn_start",
            "session": "ts-test",
            "turn_id": "turn_1791947307784_36",
            "time_ms": 1791947308000i64,
            "user_input": "search marker TS-PAYLOAD-SEQ"
        }),
        json!({
            "type": "turn_start",
            "session": "ts-test",
            "turn_id": "turn_not-a-number",
            "time_ms": 1791947400000i64,
            "user_input": "search marker TS-PAYLOAD-FALLBACK"
        }),
    ];
    fs::write(
        &audit_file,
        events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();

    let action = ParsedAction {
        action: "memmgr".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "global",
            "search_text": "TS-PAYLOAD",
            "limit": 5
        }),
    };
    let outcome = execute_outcome(&mut core, &action);
    assert_eq!(outcome.status, ActionStatus::Completed);
    let text = outcome.text;
    assert!(
        text.contains("turn_id=turn_1791947307784_36 role=user created_at_ms=1791947307784"),
        "{}",
        text
    );
    assert!(
        text.contains("turn_id=turn_not-a-number role=user created_at_ms=1791947400000"),
        "{}",
        text
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_orders_newest_first_and_honors_time_windows() {
    use crate::CoreProfile;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "timem_memmgr_order_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    let audit_file = core.chat_history.audit_file.clone();
    fs::create_dir_all(audit_file.parent().unwrap()).unwrap();
    let events = [
        json!({
            "type": "turn_start",
            "session": "order-test",
            "turn_id": "turn_1791947300000_1",
            "user_input": "search marker ORDER-OLD"
        }),
        json!({
            "type": "turn_start",
            "session": "order-test",
            "turn_id": "turn_1791947400000_2",
            "user_input": "search marker ORDER-NEW"
        }),
    ];
    fs::write(
        &audit_file,
        events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();

    let search = |core: &mut AgentCore, extra: serde_json::Value| {
        let mut raw_input = json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "global",
            "search_text": "ORDER",
            "limit": 5
        });
        if let (Some(target), Some(source)) = (raw_input.as_object_mut(), extra.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        let action = ParsedAction {
            action: "memmgr".to_string(),
            name: None,
            call_id: "test_call".to_string(),
            raw_input,
        };
        execute_outcome(core, &action).text
    };

    let text = search(&mut core, json!({}));
    let new_pos = text.find("ORDER-NEW").expect("ORDER-NEW missing");
    let old_pos = text.find("ORDER-OLD").expect("ORDER-OLD missing");
    assert!(
        new_pos < old_pos,
        "newest record must be listed first: {text}"
    );

    let before = search(&mut core, json!({"before_ms": 1791947350000i64}));
    assert!(before.contains("ORDER-OLD"), "{before}");
    assert!(!before.contains("ORDER-NEW"), "{before}");

    let after = search(&mut core, json!({"after_ms": 1791947350000i64}));
    assert!(after.contains("ORDER-NEW"), "{after}");
    assert!(!after.contains("ORDER-OLD"), "{after}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn raw_chat_session_scope_survives_a_busier_sibling_session() {
    use crate::{ActionStatus, CoreProfile};
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    let root = std::env::temp_dir().join(format!(
        "timem_memmgr_scope_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    let audit_file = core.chat_history.audit_file.clone();
    fs::create_dir_all(audit_file.parent().unwrap()).unwrap();

    // A sibling session with many newer turns must not crowd the scoped
    // session's older marker record out of the bounded query window.
    let mut lines = Vec::new();
    for index in 0..55 {
        lines.push(
            json!({
                "type": "turn_start",
                "session": "busy-sibling",
                "turn_id": format!("turn_{}_{}", 1791948100000i64 + index, index),
                "user_input": format!("search marker SCOPE-NOISE-{index}")
            })
            .to_string(),
        );
    }
    lines.push(
        json!({
            "type": "turn_start",
            "session": "scoped-target",
            "turn_id": "turn_1791947300000_9",
            "user_input": "search marker SCOPE-TARGET"
        })
        .to_string(),
    );
    fs::write(&audit_file, lines.join("\n") + "\n").unwrap();

    let action = ParsedAction {
        action: "memmgr".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "session",
            "session_id": "scoped-target",
            "search_text": "SCOPE-TARGET",
            "limit": 5
        }),
    };
    let outcome = execute_outcome(&mut core, &action);
    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(outcome.text.contains("SCOPE-TARGET"), "{}", outcome.text);
    assert!(
        outcome.text.contains("created_at_ms=1791947300000"),
        "{}",
        outcome.text
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unsupported_memmgr_operation_has_structured_invalid_input_evidence() {
    use crate::CoreProfile;
    use serde_json::json;

    let root = std::env::temp_dir().join(format!("timem_memmgr_invalid_{}", std::process::id()));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    let action = ParsedAction {
        action: "memmgr".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input: json!({"type": "durable", "op": "unsupported"}),
    };

    let outcome = execute_outcome(&mut core, &action);
    let evidence = outcome.memmgr_result.expect("memmgr error evidence");
    assert_eq!(evidence.memory_type, "durable");
    assert_eq!(evidence.op, "unsupported");
    assert_eq!(evidence.error_type.as_deref(), Some("InvalidInput"));
    let _ = std::fs::remove_dir_all(root);
}

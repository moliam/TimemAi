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

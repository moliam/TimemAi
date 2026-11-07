use super::*;

fn write_session_history(session_id: &str, core: &AgentCore, messages: &[serde_json::Value]) {
    let dir = core.chat_history.sessions_dir.join(session_id);
    std::fs::create_dir_all(&dir).unwrap();
    let lines = messages
        .iter()
        .map(message_line)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.join("raw_chat_history.jsonl"), lines + "\n").unwrap();
}

/// The runtime persists `ChatHistoryRecord` as an internally tagged enum, so
/// the `"type"` field is always the first key of the line; emit fixtures in
/// the same shape because the search prefilter reads that prefix.
fn message_line(message: &serde_json::Value) -> String {
    let object = message.as_object().expect("fixture must be an object");
    let mut line = String::from("{");
    let mut first = true;
    let mut ordered_keys = Vec::with_capacity(object.len());
    ordered_keys.push("type");
    ordered_keys.extend(
        object
            .keys()
            .filter(|key| *key != "type")
            .map(String::as_str),
    );
    for key in ordered_keys {
        if !first {
            line.push(',');
        }
        first = false;
        let value = object.get(key).expect("fixture key must exist");
        line.push_str(&format!(
            "\"{}\":{}",
            key,
            serde_json::to_string(value).unwrap()
        ));
    }
    line.push('}');
    line
}

fn user_message(turn_id: &str, created_at_ms: i64, content: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "role": "user",
        "turn_id": turn_id,
        "created_at_ms": created_at_ms,
        "kind": "task",
        "content": content
    })
}

fn assistant_message(turn_id: &str, created_at_ms: i64, content: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "role": "assistant",
        "turn_id": turn_id,
        "created_at_ms": created_at_ms,
        "content": content
    })
}

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
    let messages = [
        user_message(
            "turn_1786970000000",
            1786970000000,
            "search marker STATUS-PAYLOAD-42",
        ),
        assistant_message(
            "turn_1786970000000",
            1786970001000,
            "readfile timed out; error: example; cancelled",
        ),
    ];
    write_session_history("status-test", &core, &messages);

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
    // Real turn ids are `turn_<start_millis>_<sequence>`, but the record time
    // must come from the persisted message timestamp, not from the id shape.
    let messages = [
        user_message(
            "turn_1791947307784_36",
            1791947307784,
            "search marker TS-PAYLOAD-SEQ",
        ),
        user_message(
            "turn_not-a-number",
            1791947400000,
            "search marker TS-PAYLOAD-FALLBACK",
        ),
    ];
    write_session_history("ts-test", &core, &messages);

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
    let messages = [
        user_message(
            "turn_1791947300000_1",
            1791947300000,
            "search marker ORDER-OLD",
        ),
        user_message(
            "turn_1791947400000_2",
            1791947400000,
            "search marker ORDER-NEW",
        ),
    ];
    write_session_history("order-test", &core, &messages);

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
    // A sibling session with many newer turns must not crowd the scoped
    // session's older marker record out of the bounded query window.
    let mut busy = Vec::new();
    for index in 0..55 {
        busy.push(user_message(
            &format!("turn_{}_{}", 1791948100000i64 + index, index),
            1791948100000 + index,
            &format!("search marker SCOPE-NOISE-{index}"),
        ));
    }
    write_session_history("busy-sibling", &core, &busy);
    write_session_history(
        "scoped-target",
        &core,
        &[user_message(
            "turn_1791947300000_9",
            1791947300000,
            "search marker SCOPE-TARGET",
        )],
    );

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
    assert!(
        outcome.text.contains("content=search marker SCOPE-TARGET"),
        "{}",
        outcome.text
    );
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

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "timem_memmgr_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn memmgr_outcome(core: &mut AgentCore, raw_input: serde_json::Value) -> crate::ActionOutcome {
    let action = ParsedAction {
        action: "memmgr".to_string(),
        name: None,
        call_id: "test_call".to_string(),
        raw_input,
    };
    execute_outcome(core, &action)
}

fn search_raw_chat(core: &mut AgentCore, text: &str) -> crate::ActionOutcome {
    memmgr_outcome(
        core,
        serde_json::json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "global",
            "search_text": text,
            "limit": 5
        }),
    )
}

#[test]
fn raw_chat_search_includes_user_supplements_in_the_turn_pair() {
    use crate::CoreProfile;

    let root = temp_root("supplement");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "supplement-test",
        &core,
        &[
            user_message("turn_1791950000000_1", 1791950000000, "给这个方案起个名字"),
            serde_json::json!({
                "type": "message",
                "role": "user",
                "turn_id": "turn_1791950000000_1",
                "created_at_ms": 1791950000500i64,
                "kind": "supplement",
                "content": "补充：名字里要带紫色标记"
            }),
            assistant_message(
                "turn_1791950000000_1",
                1791950001000,
                "方案名：紫色标记计划",
            ),
        ],
    );

    // Matching only the supplement must still return the whole user/AI pair.
    let outcome = search_raw_chat(&mut core, "紫色标记");
    assert!(outcome.text.contains("role=user"), "{}", outcome.text);
    assert!(
        outcome.text.contains("名字里要带紫色标记"),
        "{}",
        outcome.text
    );
    assert!(outcome.text.contains("role=assistant"), "{}", outcome.text);
    assert!(
        outcome.text.contains("方案名：紫色标记计划"),
        "{}",
        outcome.text
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_ignores_machine_injected_user_entries() {
    use crate::CoreProfile;

    let root = temp_root("machine_kind");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "machine-test",
        &core,
        &[
            serde_json::json!({
                "type": "message",
                "role": "user",
                "turn_id": "turn_1791960000000_1",
                "created_at_ms": 1791960000000i64,
                "kind": "mailbox",
                "content": "MAILBOX-ONLY-MARKER"
            }),
            assistant_message("turn_1791960000000_1", 1791960001000, "收到邮件"),
        ],
    );

    let outcome = search_raw_chat(&mut core, "MAILBOX-ONLY-MARKER");
    assert!(outcome.text.contains("results: none"), "{}", outcome.text);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_is_fail_safe_without_history_or_with_a_corrupt_index() {
    use crate::{ActionStatus, CoreProfile};

    let root = temp_root("fail_safe");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );

    // No sessions directory at all: a search is an empty result, not an error.
    let outcome = search_raw_chat(&mut core, "ANYTHING");
    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(outcome.text.contains("results: none"), "{}", outcome.text);

    // A scoped session whose history was deleted by capacity eviction.
    let scoped = memmgr_outcome(
        &mut core,
        serde_json::json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "session",
            "session_id": "missing-session",
            "search_text": "ANYTHING"
        }),
    );
    assert_eq!(scoped.status, ActionStatus::Completed);
    assert!(scoped.text.contains("results: none"), "{}", scoped.text);

    // A corrupt persisted index cache must not affect the file-based search.
    write_session_history(
        "index-test",
        &core,
        &[user_message(
            "turn_1791980000000_1",
            1791980000000,
            "INDEX-SAFE-MARKER",
        )],
    );
    std::fs::write(
        root.join("sessions/index-test/raw_chat_history.index.json"),
        "{corrupt",
    )
    .unwrap();
    let outcome = search_raw_chat(&mut core, "INDEX-SAFE-MARKER");
    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(
        outcome.text.contains("content=INDEX-SAFE-MARKER"),
        "{}",
        outcome.text
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_delete_rewrites_history_and_invalidates_the_index() {
    use crate::{ActionStatus, CoreProfile};

    let root = temp_root("delete");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "delete-test",
        &core,
        &[
            user_message("turn_1791970000000_1", 1791970000000, "删除目标 ALPHA"),
            assistant_message("turn_1791970000000_1", 1791970001000, "将删除"),
            user_message("turn_1791970000000_2", 1791970002000, "保留目标 BETA"),
            assistant_message("turn_1791970000000_2", 1791970003000, "将保留"),
        ],
    );
    let index_path = root
        .join("sessions/delete-test")
        .join("raw_chat_history.index.json");
    std::fs::write(&index_path, "{\"indexed_len\":0,\"entries\":[]}").unwrap();

    let outcome = memmgr_outcome(
        &mut core,
        serde_json::json!({
            "type": "raw_chat",
            "op": "delete",
            "id": "turn_1791970000000_1",
            "search_text": "删除目标"
        }),
    );
    assert_eq!(outcome.status, ActionStatus::Completed);
    assert!(
        outcome.text.contains("deleted_count: 1"),
        "{}",
        outcome.text
    );
    assert!(!index_path.exists(), "stale index must be invalidated");

    let deleted = search_raw_chat(&mut core, "ALPHA");
    assert!(deleted.text.contains("results: none"), "{}", deleted.text);
    let kept = search_raw_chat(&mut core, "BETA");
    assert!(kept.text.contains("content=保留目标 BETA"), "{}", kept.text);

    // Deleting an unknown turn reports zero instead of failing.
    let missing = memmgr_outcome(
        &mut core,
        serde_json::json!({
            "type": "raw_chat",
            "op": "delete",
            "id": "turn_missing",
            "search_text": "nothing"
        }),
    );
    assert_eq!(missing.status, ActionStatus::Completed);
    assert!(
        missing.text.contains("deleted_count: 0"),
        "{}",
        missing.text
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_survives_invalid_utf8_and_padded_lines() {
    use crate::CoreProfile;

    let root = temp_root("tolerant_parse");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    let dir = core.chat_history.sessions_dir.join("tolerant-test");
    std::fs::create_dir_all(&dir).unwrap();
    let mut bytes = Vec::new();
    // A corrupt byte sequence must only poison its own line.
    bytes.extend_from_slice(b"{\"type\":\"message\",\"role\":\"user\",\"turn_id\":\"turn_1\",\"created_at_ms\":1,\"content\":\"bad \xff\xfe line\"}\n");
    // Leading whitespace and a UTF-8 BOM must not hide a valid record.
    bytes.extend_from_slice(
        "\u{feff}   {\"type\":\"message\",\"role\":\"user\",\"turn_id\":\"turn_1791990000000_1\",\"created_at_ms\":1791990000000,\"kind\":\"task\",\"content\":\"PADDED-MARKER\"}\n"
            .as_bytes(),
    );
    bytes.extend_from_slice(
        b"{\"type\":\"message\",\"role\":\"assistant\",\"turn_id\":\"turn_1791990000000_1\",\"created_at_ms\":1791990001000,\"content\":\"ok\"}\n",
    );
    std::fs::write(dir.join("raw_chat_history.jsonl"), bytes).unwrap();

    let outcome = search_raw_chat(&mut core, "PADDED-MARKER");
    assert!(
        !outcome.text.contains("results: none"),
        "BOM and leading padding must not hide the record: {}",
        outcome.text
    );
    assert!(
        outcome.text.contains("content=PADDED-MARKER"),
        "{}",
        outcome.text
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_skips_assistant_replies_to_machine_entries() {
    use crate::CoreProfile;

    let root = temp_root("machine_turn");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "machine-turn-test",
        &core,
        &[
            serde_json::json!({
                "type": "message",
                "role": "user",
                "turn_id": "turn_1791995000000_1",
                "created_at_ms": 1791995000000i64,
                "kind": "timer",
                "content": "定时任务触发"
            }),
            assistant_message(
                "turn_1791995000000_1",
                1791995001000,
                "MAILBOX-REPLY-MARKER",
            ),
        ],
    );

    let outcome = search_raw_chat(&mut core, "MAILBOX-REPLY-MARKER");
    assert!(outcome.text.contains("results: none"), "{}", outcome.text);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_does_not_match_session_or_turn_metadata() {
    use crate::CoreProfile;

    let root = temp_root("metadata_not_text");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "metadata-test",
        &core,
        &[user_message(
            "turn_1792010000000_1",
            1792010000000,
            "只有正文内容",
        )],
    );

    // Session and turn IDs are routing metadata; a query made only of them
    // must not return the record's text as if it matched.
    for query in ["metadata-test", "turn_1792010000000_1"] {
        let outcome = search_raw_chat(&mut core, query);
        assert!(
            outcome.text.contains("results: none"),
            "query {query} must not match routing metadata: {}",
            outcome.text
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_keeps_emoji_only_queries_as_literal_text() {
    use crate::CoreProfile;

    let root = temp_root("emoji_query");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "emoji-test",
        &core,
        &[
            user_message("turn_1792020000000_1", 1792020000000, "猫咪 🐱 记录"),
            user_message("turn_1792020001000_2", 1792020001000, "狗狗 🐶 记录"),
        ],
    );

    // Emoji carry no alphanumeric token; they must still filter instead of
    // degrading into an unfiltered recent list.
    let outcome = search_raw_chat(&mut core, "🐱");
    assert!(!outcome.text.contains("results: none"), "{}", outcome.text);
    assert!(outcome.text.contains("猫咪 🐱 记录"), "{}", outcome.text);
    assert!(!outcome.text.contains("狗狗 🐶 记录"), "{}", outcome.text);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn raw_chat_search_keeps_only_the_newest_bounded_pairs() {
    use crate::CoreProfile;

    let root = temp_root("bounded_topk");
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            model: "test".to_string(),
        },
        &root,
    );
    write_session_history(
        "topk-test",
        &core,
        &[
            user_message("turn_1792000000000_1", 1792000000000, "LIMIT-MARKER oldest"),
            user_message("turn_1792000001000_2", 1792000001000, "LIMIT-MARKER middle"),
            user_message("turn_1792000002000_3", 1792000002000, "LIMIT-MARKER newest"),
        ],
    );

    let outcome = memmgr_outcome(
        &mut core,
        serde_json::json!({
            "type": "raw_chat",
            "op": "search",
            "scope": "global",
            "search_text": "LIMIT-MARKER",
            "limit": 2
        }),
    );
    let text = &outcome.text;
    assert!(text.contains("newest"), "{text}");
    assert!(text.contains("middle"), "{text}");
    assert!(!text.contains("oldest"), "{text}");
    assert!(
        text.find("newest").unwrap() < text.find("middle").unwrap(),
        "newest-first ordering: {text}"
    );
    let _ = std::fs::remove_dir_all(root);
}

use agent_core::session_store::{
    chat_history_prompt_format_hint, new_stored_session, read_all_history_records,
    ChatHistoryEventKind, ChatHistoryRecord, ChatHistoryRole, SessionResumeNotice, SessionStore,
    StoredSessionProfile, StoredSessionState,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "timem_session_store_test_{}_{}_{}_{}",
        name,
        std::process::id(),
        now_ms(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn profile() -> StoredSessionProfile {
    StoredSessionProfile {
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
        api_protocol: "openai-compatible".to_string(),
        response_protocol: "xml".to_string(),
    }
}

fn message(turn: usize) -> ChatHistoryRecord {
    ChatHistoryRecord::Message {
        role: ChatHistoryRole::User,
        turn_id: format!("turn_{turn}"),
        created_at_ms: turn as i64,
        content: format!("message {turn}"),
    }
}

#[test]
fn chat_history_records_round_trip_as_jsonl() {
    let root = tmp_dir("round_trip");
    let store = SessionStore::new(&root);
    let mut extra = BTreeMap::new();
    extra.insert("tool".to_string(), Value::String("run_bash".to_string()));
    extra.insert("cmd".to_string(), Value::String("printf ok".to_string()));
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 10,
            content: "hello".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "turn_1".to_string(),
            created_at_ms: 11,
            kind: ChatHistoryEventKind::ActionResult,
            content: "Action result: run_bash\nok".to_string(),
            extra,
        },
    ];

    for record in &records {
        store.append_history_record("session_a", record).unwrap();
    }

    let path = store.history_path_for_session("session_a");
    let loaded = read_all_history_records(&path).unwrap();
    assert_eq!(loaded, records);
    for line in fs::read_to_string(path).unwrap().lines() {
        serde_json::from_str::<ChatHistoryRecord>(line).unwrap();
    }
}

#[test]
fn prompt_format_hint_examples_are_generated_from_real_schema() {
    let path = PathBuf::from("/tmp/raw_chat_history.jsonl");
    let hint = chat_history_prompt_format_hint(&path);
    assert!(hint.contains("path: /tmp/raw_chat_history.jsonl"));
    assert!(hint.contains("format: JSONL, one record per line."));

    let example_lines = hint
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .collect::<Vec<_>>();
    assert_eq!(example_lines.len(), 2);
    for example in example_lines {
        let value = serde_json::from_str::<Value>(example).unwrap();
        assert!(value.get("type").is_some());
        assert!(value.get("role").is_some());
        assert!(value.get("turn_id").is_some());
        assert!(value.get("created_at_ms").is_some());
        assert!(value.get("content").is_some());
        serde_json::from_value::<ChatHistoryRecord>(value).unwrap();
    }
}

#[test]
fn history_page_loads_latest_then_older_without_overlap() {
    let root = tmp_dir("paging");
    let store = SessionStore::new(&root);
    for index in 0..450 {
        store
            .append_history_record("session_a", &message(index))
            .unwrap();
    }

    let latest = store.read_history_page("session_a", None, 200).unwrap();
    assert_eq!(latest.records.len(), 200);
    assert_eq!(latest.records.first().unwrap().turn_id(), "turn_250");
    assert_eq!(latest.records.last().unwrap().turn_id(), "turn_449");
    assert_eq!(latest.before_cursor.as_deref(), Some("250"));
    assert!(latest.has_more);

    let previous = store
        .read_history_page("session_a", latest.before_cursor.as_deref(), 200)
        .unwrap();
    assert_eq!(previous.records.len(), 200);
    assert_eq!(previous.records.first().unwrap().turn_id(), "turn_50");
    assert_eq!(previous.records.last().unwrap().turn_id(), "turn_249");
    assert_eq!(previous.before_cursor.as_deref(), Some("50"));
    assert!(previous.has_more);

    let oldest = store
        .read_history_page("session_a", previous.before_cursor.as_deref(), 200)
        .unwrap();
    assert_eq!(oldest.records.len(), 50);
    assert_eq!(oldest.records.first().unwrap().turn_id(), "turn_0");
    assert_eq!(oldest.records.last().unwrap().turn_id(), "turn_49");
    assert!(oldest.before_cursor.is_none());
    assert!(!oldest.has_more);
}

#[test]
fn stored_sessions_are_host_agnostic_and_sorted_by_recent_update() {
    let root = tmp_dir("stored_sessions");
    let store = SessionStore::new(&root);
    let mut first = new_stored_session(
        "session_web",
        "Project work",
        "/tmp/project",
        profile(),
        store.history_path_for_session("session_web"),
    );
    first.updated_at_ms = 10;
    let mut second = new_stored_session(
        "session_shell",
        "Shell follow-up",
        "/tmp/project",
        profile(),
        store.history_path_for_session("session_shell"),
    );
    second.updated_at_ms = 20;
    second.state = StoredSessionState::Interrupted;

    store.upsert_session(&first).unwrap();
    store.upsert_session(&second).unwrap();
    first.display_name = "Renamed project work".to_string();
    first.updated_at_ms = 30;
    store.upsert_session(&first).unwrap();

    let sessions = store.list_sessions().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session_id, "session_web");
    assert_eq!(sessions[0].display_name, "Renamed project work");
    assert_eq!(sessions[1].session_id, "session_shell");
    assert_eq!(sessions[1].state, StoredSessionState::Interrupted);
}

#[test]
fn resume_notice_references_history_format_without_web_specific_language() {
    let notice = SessionResumeNotice {
        history_path: PathBuf::from("/tmp/session/raw_chat_history.jsonl"),
        current_dir: PathBuf::from("/work/project"),
    };
    let rendered = notice.render();
    assert!(rendered.starts_with("## SYSTEM"));
    assert!(rendered
        .contains("This session was restored and may not include the full previous context."));
    assert!(rendered.contains("path: /tmp/session/raw_chat_history.jsonl"));
    assert!(rendered.contains("format: JSONL, one record per line."));
    assert!(rendered.contains("Try to use efficient tools such as tail, rg, jq"));
    assert!(rendered.contains("Current cwd: /work/project"));
    assert!(!rendered.to_lowercase().contains("web"));
}

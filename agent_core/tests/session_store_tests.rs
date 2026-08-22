use agent_core::session_store::{
    chat_history_prompt_format_hint, new_stored_session, read_all_history_records,
    read_history_page_from_path, ChatHistoryEventKind, ChatHistoryRecord, ChatHistoryRole,
    SessionResumeNotice, SessionStore, StoredSessionProfile, StoredSessionState,
};

#[test]
fn chat_history_message_command_id_round_trips_for_exactly_once_recovery() {
    let record = ChatHistoryRecord::Message {
        role: ChatHistoryRole::User,
        turn_id: "turn_1".to_string(),
        created_at_ms: 1,
        kind: Some("task".to_string()),
        command_id: Some("command_1".to_string()),
        delivery_state: None,
        content: "run once".to_string(),
    };
    let encoded = serde_json::to_string(&record).unwrap();
    let decoded: ChatHistoryRecord = serde_json::from_str(&encoded).unwrap();
    assert!(matches!(
        decoded,
        ChatHistoryRecord::Message { command_id: Some(command_id), .. }
            if command_id == "command_1"
    ));
}
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
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
        kind: None,
        command_id: None,
        delivery_state: None,
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
            kind: None,
            command_id: None,
            delivery_state: None,
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
fn chat_history_user_entry_kind_is_optional_and_round_trips() {
    let without_kind = ChatHistoryRecord::Message {
        role: ChatHistoryRole::User,
        turn_id: "turn_1".to_string(),
        created_at_ms: 10,
        kind: None,
        command_id: None,
        delivery_state: None,
        content: "plain task".to_string(),
    };
    let text = serde_json::to_string(&without_kind).unwrap();
    assert!(!text.contains("\"kind\""));

    let with_kind = ChatHistoryRecord::Message {
        role: ChatHistoryRole::User,
        turn_id: "turn_1".to_string(),
        created_at_ms: 11,
        kind: Some("supplement".to_string()),
        command_id: None,
        delivery_state: None,
        content: "extra instruction".to_string(),
    };
    let value = serde_json::to_value(&with_kind).unwrap();
    assert_eq!(value["kind"], "supplement");
    assert_eq!(
        serde_json::from_value::<ChatHistoryRecord>(value).unwrap(),
        with_kind
    );
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
fn indexed_history_pages_match_the_uncached_reader_and_refresh_after_append() {
    let root = tmp_dir("indexed_history");
    let store = SessionStore::new(&root);
    for index in 0..215 {
        store
            .append_history_record("session_a", &message(index))
            .unwrap();
    }
    let path = store.history_path_for_session("session_a");

    let indexed_latest = store.read_history_page("session_a", None, 20).unwrap();
    let uncached_latest = read_history_page_from_path(&path, None, 20).unwrap();
    assert_eq!(indexed_latest, uncached_latest);

    let indexed_previous = store
        .read_history_page("session_a", indexed_latest.before_cursor.as_deref(), 20)
        .unwrap();
    let uncached_previous =
        read_history_page_from_path(&path, uncached_latest.before_cursor.as_deref(), 20).unwrap();
    assert_eq!(indexed_previous, uncached_previous);

    for index in 215..225 {
        store
            .append_history_record("session_a", &message(index))
            .unwrap();
    }
    let refreshed = store.read_history_page("session_a", None, 20).unwrap();
    assert_eq!(refreshed.records.first().unwrap().turn_id(), "turn_205");
    assert_eq!(refreshed.records.last().unwrap().turn_id(), "turn_224");
}

#[test]
fn indexed_history_reader_treats_a_missing_history_file_as_empty() {
    let root = tmp_dir("missing_indexed_history");
    let store = SessionStore::new(&root);

    let page = store
        .read_history_page("session_without_history", None, 200)
        .unwrap();

    assert!(page.records.is_empty());
    assert!(page.before_cursor.is_none());
    assert!(!page.has_more);
}

#[test]
fn indexed_history_reader_pages_a_large_session_without_losing_turn_order() {
    let root = tmp_dir("large_indexed_history");
    let store = SessionStore::new(&root);
    let path = store.history_path_for_session("session_a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut history = String::new();
    for index in 0..5_000 {
        history.push_str(&serde_json::to_string(&message(index)).unwrap());
        history.push('\n');
    }
    fs::write(path, history).unwrap();

    let latest = store.read_history_page("session_a", None, 200).unwrap();
    assert_eq!(latest.records.len(), 200);
    assert_eq!(latest.records.first().unwrap().turn_id(), "turn_4800");
    assert_eq!(latest.records.last().unwrap().turn_id(), "turn_4999");
    assert_eq!(latest.before_cursor.as_deref(), Some("4800"));

    let previous = store
        .read_history_page("session_a", latest.before_cursor.as_deref(), 200)
        .unwrap();
    assert_eq!(previous.records.len(), 200);
    assert_eq!(previous.records.first().unwrap().turn_id(), "turn_4600");
    assert_eq!(previous.records.last().unwrap().turn_id(), "turn_4799");
    assert_eq!(previous.before_cursor.as_deref(), Some("4600"));
}

#[test]
fn history_readers_skip_malformed_jsonl_lines() {
    let root = tmp_dir("malformed_history");
    let store = SessionStore::new(&root);
    let path = store.history_path_for_session("session_a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let valid_1 = serde_json::to_string(&message(1)).unwrap();
    let valid_2 = serde_json::to_string(&message(2)).unwrap();
    fs::write(&path, format!("{valid_1}\n{{not valid json\n\n{valid_2}\n")).unwrap();

    let records = read_all_history_records(&path).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].turn_id(), "turn_1");
    assert_eq!(records[1].turn_id(), "turn_2");
}

#[test]
fn history_page_cursor_counts_valid_records_when_bad_lines_exist() {
    let root = tmp_dir("malformed_history_paging");
    let store = SessionStore::new(&root);
    let path = store.history_path_for_session("session_a");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut lines = Vec::new();
    for index in 0..5 {
        lines.push(serde_json::to_string(&message(index)).unwrap());
        if index == 1 || index == 3 {
            lines.push("not-json".to_string());
        }
    }
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();

    let latest = store.read_history_page("session_a", None, 2).unwrap();
    assert_eq!(
        latest
            .records
            .iter()
            .map(ChatHistoryRecord::turn_id)
            .collect::<Vec<_>>(),
        vec!["turn_3", "turn_4"]
    );
    assert_eq!(latest.before_cursor.as_deref(), Some("3"));
    assert!(latest.has_more);

    let previous = store
        .read_history_page("session_a", latest.before_cursor.as_deref(), 2)
        .unwrap();
    assert_eq!(
        previous
            .records
            .iter()
            .map(ChatHistoryRecord::turn_id)
            .collect::<Vec<_>>(),
        vec!["turn_1", "turn_2"]
    );
    assert_eq!(previous.before_cursor.as_deref(), Some("1"));
    assert!(previous.has_more);
}

#[test]
fn history_pages_never_restore_a_supplement_without_its_turn_task() {
    let root = tmp_dir("turn_aligned_paging");
    let store = SessionStore::new(&root);
    let session_id = "session_a";
    let records = [
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_long".to_string(),
            created_at_ms: 1,
            kind: Some("task".to_string()),
            command_id: None,
            delivery_state: None,
            content: "original milestone request".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "turn_long".to_string(),
            created_at_ms: 2,
            kind: ChatHistoryEventKind::Action,
            content: "first action".to_string(),
            extra: BTreeMap::new(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_long".to_string(),
            created_at_ms: 3,
            kind: Some("supplement".to_string()),
            command_id: None,
            delivery_state: None,
            content: "还有一个 tar_log，下面是 clp 压缩的日志".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "turn_long".to_string(),
            created_at_ms: 4,
            kind: ChatHistoryEventKind::ActionResult,
            content: "action result".to_string(),
            extra: BTreeMap::new(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_latest".to_string(),
            created_at_ms: 5,
            kind: Some("task".to_string()),
            command_id: None,
            delivery_state: None,
            content: "latest task".to_string(),
        },
    ];
    for record in &records {
        store.append_history_record(session_id, record).unwrap();
    }

    let latest = store.read_history_page(session_id, None, 1).unwrap();
    assert_eq!(latest.records.len(), 1);
    assert_eq!(latest.records[0].turn_id(), "turn_latest");
    assert_eq!(latest.before_cursor.as_deref(), Some("4"));

    let previous = store
        .read_history_page(session_id, latest.before_cursor.as_deref(), 1)
        .unwrap();
    assert_eq!(previous.records.len(), 4);
    assert!(previous
        .records
        .iter()
        .any(|record| matches!(record, ChatHistoryRecord::Message { kind: Some(kind), content, .. } if kind == "task" && content == "original milestone request")));
    assert!(previous
        .records
        .iter()
        .any(|record| matches!(record, ChatHistoryRecord::Message { kind: Some(kind), content, .. } if kind == "supplement" && content.contains("tar_log"))));
    assert!(previous.before_cursor.is_none());
}

#[test]
fn history_page_limit_counts_complete_turns_instead_of_records() {
    let root = tmp_dir("turn_count_paging");
    let store = SessionStore::new(&root);
    for turn in 0..5 {
        for record in 0..40 {
            store
                .append_history_record(
                    "session_a",
                    &ChatHistoryRecord::Event {
                        role: ChatHistoryRole::System,
                        turn_id: format!("turn_{turn}"),
                        created_at_ms: (turn * 40 + record) as i64,
                        kind: ChatHistoryEventKind::Action,
                        content: format!("action {record}"),
                        extra: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
    }

    let latest = store.read_history_page("session_a", None, 3).unwrap();
    let turn_ids = latest
        .records
        .iter()
        .map(ChatHistoryRecord::turn_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        turn_ids,
        ["turn_2", "turn_3", "turn_4"].into_iter().collect()
    );
    assert_eq!(latest.records.len(), 120);
    assert_eq!(latest.before_cursor.as_deref(), Some("80"));
    assert!(latest.has_more);
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
    first.mcp_server_ids = vec!["github".to_string(), "filesystem".to_string()];
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
    assert_eq!(sessions[0].mcp_server_ids, vec!["github", "filesystem"]);
    assert_eq!(sessions[1].session_id, "session_shell");
    assert_eq!(sessions[1].state, StoredSessionState::Interrupted);
}

#[test]
fn resilient_session_index_load_preserves_valid_records_and_backs_up_corruption() {
    let root = tmp_dir("resilient_corrupt_index");
    let store = SessionStore::new(&root);
    std::fs::create_dir_all(store.sessions_dir()).unwrap();
    let valid = new_stored_session(
        "session_valid",
        "Recovered",
        "/tmp/project",
        profile(),
        store.history_path_for_session("session_valid"),
    );
    let valid_line = serde_json::to_string(&valid).unwrap();
    let original = format!("{valid_line}\ntruncated-json\n");
    std::fs::write(store.index_path(), &original).unwrap();

    let recovery = store.list_sessions_resilient().unwrap();
    assert!(recovery.repaired());
    assert_eq!(recovery.invalid_records, 1);
    assert_eq!(recovery.sessions, vec![valid]);
    let backup = recovery.backup_path.unwrap();
    assert_eq!(std::fs::read_to_string(backup).unwrap(), original);
    assert_eq!(store.list_sessions().unwrap().len(), 1);
}

#[test]
fn resilient_session_index_load_quarantines_non_utf8_records() {
    let root = tmp_dir("resilient_non_utf8_index");
    let store = SessionStore::new(&root);
    std::fs::create_dir_all(store.sessions_dir()).unwrap();
    let original = [b'{', 0xff, b'}', b'\n'];
    std::fs::write(store.index_path(), original).unwrap();

    let recovery = store.list_sessions_resilient().unwrap();
    assert!(recovery.sessions.is_empty());
    assert_eq!(recovery.invalid_records, 1);
    assert_eq!(
        std::fs::read(recovery.backup_path.unwrap()).unwrap(),
        original
    );
}

#[test]
fn resilient_session_index_load_bounds_oversized_records_and_keeps_following_sessions() {
    let root = tmp_dir("resilient_oversized_index");
    let store = SessionStore::new(&root);
    std::fs::create_dir_all(store.sessions_dir()).unwrap();
    let valid = new_stored_session(
        "session_after_large_record",
        "Recovered after oversized record",
        "/tmp/project",
        profile(),
        store.history_path_for_session("session_after_large_record"),
    );
    let valid_line = serde_json::to_string(&valid).unwrap();
    let mut original = vec![b'x'; 1024 * 1024 + 100];
    original.push(b'\n');
    original.extend_from_slice(valid_line.as_bytes());
    original.push(b'\n');
    std::fs::write(store.index_path(), &original).unwrap();

    let recovery = store.list_sessions_resilient().unwrap();
    assert_eq!(recovery.invalid_records, 1);
    assert_eq!(recovery.sessions, vec![valid]);
    assert_eq!(
        std::fs::read(recovery.backup_path.unwrap()).unwrap(),
        original
    );
}

#[test]
fn resilient_session_index_load_quarantines_fully_corrupt_index_before_reset() {
    let root = tmp_dir("resilient_fully_corrupt_index");
    let store = SessionStore::new(&root);
    std::fs::create_dir_all(store.sessions_dir()).unwrap();
    std::fs::write(store.index_path(), b"not-json\n").unwrap();

    let recovery = store.list_sessions_resilient().unwrap();
    assert!(recovery.sessions.is_empty());
    assert_eq!(recovery.invalid_records, 1);
    assert_eq!(
        std::fs::read(recovery.backup_path.unwrap()).unwrap(),
        b"not-json\n"
    );
    assert!(store.list_sessions().unwrap().is_empty());
}

#[test]
fn concurrent_session_store_instances_never_expose_partial_or_lose_index_records() {
    const WRITERS: usize = 4;
    const UPDATES: usize = 12;
    let root = tmp_dir("concurrent_index");
    let barrier = Arc::new(Barrier::new(WRITERS + 1));
    let mut workers = Vec::new();

    for ordinal in 0..WRITERS {
        let root = root.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let store = SessionStore::new(&root);
            barrier.wait();
            for update in 0..UPDATES {
                let mut session = new_stored_session(
                    format!("session_{ordinal}"),
                    format!("Session {ordinal} update {update}"),
                    "/tmp/project",
                    profile(),
                    store.history_path_for_session(&format!("session_{ordinal}")),
                );
                session.updated_at_ms = (update * WRITERS + ordinal) as i64;
                store.upsert_session(&session).unwrap();
                assert!(store.list_sessions().is_ok());
            }
        }));
    }

    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }

    let sessions = SessionStore::new(&root).list_sessions().unwrap();
    assert_eq!(sessions.len(), WRITERS);
    for ordinal in 0..WRITERS {
        let session = sessions
            .iter()
            .find(|session| session.session_id == format!("session_{ordinal}"))
            .unwrap();
        assert_eq!(
            session.display_name,
            format!("Session {ordinal} update {}", UPDATES - 1)
        );
    }
}

#[test]
fn deleting_a_session_removes_its_index_entry_and_persisted_data() {
    let root = tmp_dir("delete_session");
    let store = SessionStore::new(&root);
    let session = new_stored_session(
        "session_delete",
        "Delete me",
        "/tmp/project",
        profile(),
        store.history_path_for_session("session_delete"),
    );
    store.upsert_session(&session).unwrap();
    store
        .append_history_record("session_delete", &message(1))
        .unwrap();
    let session_dir = store
        .history_path_for_session("session_delete")
        .parent()
        .unwrap()
        .to_path_buf();

    store.delete_session("session_delete").unwrap();

    assert!(store.load_session("session_delete").unwrap().is_none());
    assert!(!session_dir.exists());
    assert_eq!(
        store.delete_session("session_delete").unwrap_err(),
        "session_not_found"
    );
}

#[test]
fn deleting_one_history_message_rewrites_raw_chat_and_preserves_other_records() {
    let root = tmp_dir("delete_history_message");
    let store = SessionStore::new(&root);
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 1,
            kind: Some("task".to_string()),
            command_id: None,
            delivery_state: None,
            content: "keep first user entry".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 2,
            kind: Some("supplement".to_string()),
            command_id: None,
            delivery_state: None,
            content: "delete second user entry".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "turn_1".to_string(),
            created_at_ms: 3,
            kind: ChatHistoryEventKind::Progress,
            content: "keep event".to_string(),
            extra: BTreeMap::new(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_1".to_string(),
            created_at_ms: 4,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "keep assistant".to_string(),
        },
    ];
    for record in &records {
        store.append_history_record("session_a", record).unwrap();
    }

    let deleted = store
        .delete_history_message("session_a", "turn_1", ChatHistoryRole::User, 1)
        .unwrap();
    assert!(matches!(
        deleted,
        ChatHistoryRecord::Message { content, .. } if content == "delete second user entry"
    ));
    let remaining = read_all_history_records(&store.history_path_for_session("session_a")).unwrap();
    assert_eq!(
        remaining,
        vec![records[0].clone(), records[2].clone(), records[3].clone()]
    );
    assert_eq!(
        store
            .delete_history_message("session_a", "turn_1", ChatHistoryRole::User, 1)
            .unwrap_err(),
        "chat_message_not_found"
    );
}

#[cfg(unix)]
#[test]
fn session_index_permissions_protect_cached_environment() {
    use std::os::unix::fs::PermissionsExt;

    let root = tmp_dir("session_env_permissions");
    let store = SessionStore::new(&root);
    let mut session = new_stored_session(
        "session_secure",
        "Secure session",
        "/tmp/project",
        profile(),
        store.history_path_for_session("session_secure"),
    );
    session
        .env
        .insert("TIMEM_API_KEY".to_string(), "local-secret".to_string());

    store.upsert_session(&session).unwrap();

    let directory_mode = std::fs::metadata(store.sessions_dir())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let index_mode = std::fs::metadata(store.index_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(directory_mode, 0o700);
    assert_eq!(index_mode, 0o600);
    assert_eq!(
        store
            .load_session("session_secure")
            .unwrap()
            .unwrap()
            .env
            .get("TIMEM_API_KEY")
            .map(String::as_str),
        Some("local-secret")
    );
}

#[test]
fn resume_notice_references_history_format_without_web_specific_language() {
    let notice = SessionResumeNotice {
        history_path: PathBuf::from("/tmp/session/raw_chat_history.jsonl"),
        current_dir: PathBuf::from("/work/project"),
    };
    let rendered = notice.render();
    assert!(rendered.starts_with("Runtime just restarted."));
    assert!(!rendered.contains("## RUNTIME"));
    assert!(!rendered.contains("<RUNTIME>"));
    assert!(rendered.contains(
        "Runtime just restarted. Previous chat history's runtime info/tasks are invalid/outdated unless user asks to retrieve them."
    ));
    assert!(!rendered.contains("Previous audit chat history's runtime info are valid."));
    assert!(!rendered
        .contains("This session was restored and may not include the full previous context."));
    assert!(rendered.contains("path: /tmp/session/raw_chat_history.jsonl"));
    assert!(rendered.contains("format: JSONL, one record per line."));
    assert!(!rendered.contains("Do not assume the whole previous context is loaded."));
    assert!(!rendered.contains("Read this file only when needed for the current task."));
    assert!(!rendered.contains("Try to use efficient tools such as tail, rg, jq"));
    assert!(!rendered.contains("instead of a huge cat"));
    assert!(rendered.contains("Current cwd: /work/project"));
    assert!(!rendered.to_lowercase().contains("web"));
}

use agent_core::chat_library::{
    ChatLibrary, CreateFavoriteOutcome, FAVORITES_LIMIT_1_GB, FAVORITES_LIMIT_256_MB,
};
use agent_core::session_store::{
    new_stored_session, ChatHistoryRecord, ChatHistoryRole, SessionStore, StoredSessionProfile,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(1);

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "timem_chat_library_{name}_{}_{}",
        now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn prepare() -> (PathBuf, SessionStore) {
    let root = temp_dir("store");
    let store = SessionStore::new(&root);
    let profile = StoredSessionProfile {
        model: "test".into(),
        api_protocol: "openai-compatible".into(),
        response_protocol: "json".into(),
    };
    for (id, name) in [("session_a", "Alpha"), ("session_b", "Beta")] {
        let session = new_stored_session(
            id,
            name,
            &root,
            profile.clone(),
            store.history_path_for_session(id),
        );
        store.upsert_session(&session).unwrap();
    }
    store
        .append_history_record(
            "session_a",
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: "turn_a".into(),
                created_at_ms: 10,
                kind: None,
                command_id: None,
                delivery_state: None,
                content: "请设计 local search".into(),
            },
        )
        .unwrap();
    store
        .append_history_record(
            "session_a",
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::Assistant,
                turn_id: "turn_a".into(),
                created_at_ms: 11,
                kind: None,
                command_id: None,
                delivery_state: None,
                content: "Local Search 架构方案".into(),
            },
        )
        .unwrap();
    store
        .append_history_record(
            "session_b",
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::Assistant,
                turn_id: "turn_b".into(),
                created_at_ms: 20,
                kind: None,
                command_id: None,
                delivery_state: None,
                content: "另一份 search 结果".into(),
            },
        )
        .unwrap();
    (root, store)
}

#[test]
fn search_is_case_insensitive_and_honors_session_scope() {
    let (root, store) = prepare();
    let library = ChatLibrary::new(&root);
    let all = library.search(&store, "LOCAL search", None, 50).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|hit| hit.session_id == "session_a"));
    assert!(all.iter().any(|hit| hit.role == ChatHistoryRole::User));
    assert!(all.iter().any(|hit| hit.role == ChatHistoryRole::Assistant));
    let scoped = library
        .search(&store, "search", Some("session_b"), 50)
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].turn_id, "turn_b");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn favorite_creation_is_idempotent_and_search_reports_state() {
    let (root, store) = prepare();
    let library = ChatLibrary::new(&root);
    let CreateFavoriteOutcome::Created {
        favorite: first, ..
    } = library
        .create_favorite(&store, "session_a", "turn_a")
        .unwrap()
    else {
        panic!("favorite should be created")
    };
    let CreateFavoriteOutcome::Created {
        favorite: second, ..
    } = library
        .create_favorite(&store, "session_a", "turn_a")
        .unwrap()
    else {
        panic!("duplicate should return existing favorite")
    };
    assert_eq!(first.id, second.id);
    assert_eq!(library.list_favorites().unwrap().len(), 1);
    let hits = library.search(&store, "架构", None, 50).unwrap();
    assert_eq!(hits[0].favorite_id.as_deref(), Some(first.id.as_str()));
    library.delete_favorite(&first.id).unwrap();
    assert!(library.list_favorites().unwrap().is_empty());
    let hits = library.search(&store, "架构", None, 50).unwrap();
    assert!(hits[0].favorite_id.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn favorite_uses_server_history_content_not_client_text() {
    let (root, store) = prepare();
    let library = ChatLibrary::new(&root);
    let CreateFavoriteOutcome::Created { favorite, .. } = library
        .create_favorite(&store, "session_a", "turn_a")
        .unwrap()
    else {
        panic!("favorite should be created")
    };
    assert_eq!(favorite.content_snapshot, "Local Search 架构方案");
    assert_eq!(favorite.session_display_name, "Alpha");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn capacity_settings_are_mem_scoped_and_delete_compacts_log() {
    let (root, store) = prepare();
    let library = ChatLibrary::new(&root);
    let initial = library.capacity().unwrap();
    assert_eq!(initial.limit_bytes, Some(FAVORITES_LIMIT_256_MB));
    assert_eq!(initial.used_bytes, 0);
    assert_eq!(
        library
            .update_capacity_limit(Some(FAVORITES_LIMIT_1_GB))
            .unwrap()
            .limit_bytes,
        Some(FAVORITES_LIMIT_1_GB)
    );
    assert_eq!(
        ChatLibrary::new(&root).capacity().unwrap().limit_bytes,
        Some(FAVORITES_LIMIT_1_GB)
    );
    assert!(library.update_capacity_limit(Some(123)).is_err());
    let CreateFavoriteOutcome::Created { favorite, .. } = library
        .create_favorite(&store, "session_a", "turn_a")
        .unwrap()
    else {
        panic!("favorite should be created")
    };
    assert!(library.capacity().unwrap().used_bytes > 0);
    library.delete_favorite(&favorite.id).unwrap();
    assert_eq!(library.capacity().unwrap().used_bytes, 0);
    assert!(library.list_favorites().unwrap().is_empty());
    assert_eq!(
        library.update_capacity_limit(None).unwrap().limit_bytes,
        None
    );
    fs::remove_dir_all(root).unwrap();
}

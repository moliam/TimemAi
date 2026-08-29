use super::*;
use agent_core::session_runtime::ModelClient;
use agent_core::session_store::read_all_history_records;
use agent_core::{
    core_initialized_topic_event, CoreProfile, CoreSessionState, CoreSessionWorkerWorkspace,
    CoreTopic, CoreTopicEvent, LlmResponse, TurnOutcome, UsageStats, CORE_TOPIC_ACTION,
    CORE_TOPIC_MODEL_REPAIR, CORE_TOPIC_MODEL_RESPONSE,
};
use std::sync::atomic::AtomicUsize;
use std::sync::Condvar;
use std::thread;
use std::time::{Duration, Instant};

const TEST_PORT: u16 = 12345;

fn confirmed_xml_response(body: &str) -> String {
    format!(
        "<ASSISTANT><finish_confirm>{} verified</finish_confirm>{body}</ASSISTANT>",
        agent_core::response_protocol::xml_suite::FINISH_CONFIRM_PREFIX
    )
}

#[test]
fn reliable_command_wire_is_legacy_compatible_and_ack_is_correlated() {
    let reliable: BrowserCommand = serde_json::from_value(json!({
        "type": "turn_cancel", "session_id": "session_a", "command_id": "cancel_1"
    }))
    .unwrap();
    assert_eq!(reliable.command_id.as_deref(), Some("cancel_1"));
    let legacy: BrowserCommand = serde_json::from_value(json!({
        "type": "turn_cancel", "session_id": "session_a"
    }))
    .unwrap();
    assert!(legacy.command_id.is_none());
    let targeted: BrowserCommand = serde_json::from_value(json!({
        "type": "turn_cancel",
        "session_id": "session_a",
        "target_command_id": "submit_1",
        "command_id": "cancel_2"
    }))
    .unwrap();
    assert!(matches!(
        targeted.command,
        ClientCommand::TurnCancel {
            ref session_id,
            target_command_id: Some(ref target_command_id),
        } if session_id == "session_a" && target_command_id == "submit_1"
    ));
    let ack = serde_json::to_value(command_ack(
        "cancel_1",
        CommandAckStatus::Rejected,
        Some("browser_command_queue_full".to_string()),
    ))
    .unwrap();
    assert_eq!(ack["command_id"], "cancel_1");
    assert_eq!(ack["status"], "rejected");
}

#[test]
fn chat_library_commands_have_stable_wire_and_delivery_contracts() {
    let search: BrowserCommand = serde_json::from_value(json!({
        "type": "chat_search",
        "query": "release notes",
        "session_id": "session_a",
        "limit": 25
    }))
    .unwrap();
    assert!(search.command.mutation_lane().is_none());
    assert!(search.command.result_is_direct());

    let list: BrowserCommand = serde_json::from_value(json!({ "type": "favorites_list" })).unwrap();
    assert!(list.command.mutation_lane().is_none());
    assert!(list.command.result_is_direct());

    let create: BrowserCommand = serde_json::from_value(json!({
        "type": "favorite_create",
        "session_id": "session_a",
        "turn_id": "turn_1"
    }))
    .unwrap();
    assert_eq!(
        create.command.mutation_lane().as_deref(),
        Some("chat-library")
    );
    assert!(create.command.result_is_direct());

    let delete: BrowserCommand = serde_json::from_value(json!({
        "type": "favorite_delete",
        "favorite_id": "favorite_1"
    }))
    .unwrap();
    assert_eq!(
        delete.command.mutation_lane().as_deref(),
        Some("chat-library")
    );
    assert!(delete.command.result_is_direct());

    let capacity_update: BrowserCommand = serde_json::from_value(json!({
        "type": "favorite_capacity_update",
        "max_bytes": 1073741824_u64
    }))
    .unwrap();
    assert_eq!(
        capacity_update.command.mutation_lane().as_deref(),
        Some("chat-library")
    );
    assert!(capacity_update.command.result_is_direct());

    let unlimited: BrowserCommand = serde_json::from_value(json!({
        "type": "favorite_capacity_update",
        "max_bytes": null
    }))
    .unwrap();
    assert!(matches!(
        unlimited.command,
        ClientCommand::FavoriteCapacityUpdate { max_bytes: None }
    ));

    let search_event = serde_json::to_value(WireEvent::ChatSearchResult {
        query: "release notes".to_string(),
        hits: Vec::new(),
    })
    .unwrap();
    assert_eq!(search_event["type"], "chat_search_result");
    assert_eq!(search_event["query"], "release notes");
    assert_eq!(search_event["hits"], json!([]));

    let capacity = ChatLibraryCapacity {
        used_bytes: 243_000_000,
        limit_bytes: Some(268_435_456),
        used_percent: Some(91),
    };
    let capacity_event =
        serde_json::to_value(WireEvent::FavoriteCapacityUpdated { capacity }).unwrap();
    assert_eq!(capacity_event["type"], "favorite_capacity_updated");
    assert_eq!(capacity_event["capacity"]["used_percent"], 91);
    assert_eq!(capacity_event["capacity"]["limit_bytes"], 268_435_456_u64);

    let reached_event =
        serde_json::to_value(WireEvent::FavoriteCapacityReached { capacity }).unwrap();
    assert_eq!(reached_event["type"], "favorite_capacity_reached");
    assert_eq!(reached_event["capacity"]["used_percent"], 91);

    let deleted_event = serde_json::to_value(WireEvent::FavoriteDeleted {
        favorite_id: "favorite_1".to_string(),
    })
    .unwrap();
    assert_eq!(deleted_event["type"], "favorite_deleted");
    assert_eq!(deleted_event["favorite_id"], "favorite_1");
}

#[test]
fn worker_role_create_preserves_client_id_and_correlates_authoritative_event() {
    let state = routing_test_state();
    let event = handle_command_with_id(
        &state,
        TEST_PORT,
        Some("role-command-1"),
        ClientCommand::WorkerRoleCreate {
            session_id: "session_a".to_string(),
            role_id: Some("role-client-1".to_string()),
            name: "Builder".to_string(),
            description: "Build carefully.".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::WorkerRoleLibraryUpdated {
        library,
        command_id,
    } = event
    else {
        panic!("role creation should return the correlated authoritative library");
    };
    assert_eq!(command_id.as_deref(), Some("role-command-1"));
    assert_eq!(library.roles[0].id, "role-client-1");
}

#[test]
fn worker_roles_are_global_grouped_persisted_and_render_exact_system_context() {
    let state = routing_test_state();
    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::WorkerRoleCreate {
            session_id: "session_a".to_string(),
            role_id: None,
            name: "Reviewer".to_string(),
            description: "Inspect evidence before changing code.".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::WorkerRoleLibraryUpdated { library, .. } = event else {
        panic!("role creation should publish the authoritative global library")
    };
    assert_eq!(library.roles.len(), 1);
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"].roles,
        library.roles
    );
    assert_eq!(
        state.sessions.lock().unwrap()["session_b"].roles,
        library.roles
    );
    assert_eq!(
        load_role_library(&role_library_path(
            &current_mem_state(&state).unwrap().layout.memory_dir()
        ))
        .unwrap(),
        library
    );
    assert_eq!(
        worker_roles_context(&library.roles).as_deref(),
        Some(
            r#"TIMEM_WORKER_ROLE_CONTEXT: {"description":"Inspect evidence before changing code.","name":"Reviewer"}"#
        )
    );

    let role_id = library.roles[0].id.clone();
    let group_event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::WorkerRoleGroupCreate {
            session_id: "session_b".to_string(),
            name: "Quality".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::WorkerRoleLibraryUpdated { library, .. } = group_event else {
        panic!("group creation should publish the global library")
    };
    let group = library.groups[0].clone();
    let reordered = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::WorkerRoleLibraryReorder {
            session_id: "session_a".to_string(),
            groups: vec![WorkerRoleGroup {
                id: group.id.clone(),
                name: group.name.clone(),
                role_ids: vec![role_id.clone()],
            }],
            ungrouped_role_ids: Vec::new(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::WorkerRoleLibraryUpdated { library, .. } = reordered else {
        panic!("role reorder should publish the global library")
    };
    assert_eq!(library.groups[0].role_ids, vec![role_id.clone()]);

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::WorkerRoleUpdate {
            session_id: "session_b".to_string(),
            role_id: role_id.clone(),
            name: "Evidence reviewer".to_string(),
            description: "Review logs before changing code.".to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"].roles[0].name,
        "Evidence reviewer"
    );
    assert_eq!(
        state.sessions.lock().unwrap()["session_b"].roles[0].name,
        "Evidence reviewer"
    );

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::WorkerRoleDelete {
            session_id: "session_a".to_string(),
            role_id,
        },
    )
    .unwrap();
    assert!(state.sessions.lock().unwrap()["session_a"].roles.is_empty());
    assert!(state.sessions.lock().unwrap()["session_b"].roles.is_empty());
    let persisted = load_role_library(&role_library_path(
        &current_mem_state(&state).unwrap().layout.memory_dir(),
    ))
    .unwrap();
    assert!(persisted.roles.is_empty());
    assert!(persisted.groups[0].role_ids.is_empty());
}

#[test]
fn worker_role_snapshots_survive_raw_history_reconstruction() {
    let state = routing_test_state();
    let role = WorkerRole {
        id: "role_reviewer".to_string(),
        name: "Reviewer".to_string(),
        description: "Review carefully.".to_string(),
    };
    let second_role = WorkerRole {
        id: "role_tester".to_string(),
        name: "Tester".to_string(),
        description: "Test thoroughly.".to_string(),
    };
    let selected_roles = vec![role.clone(), second_role];
    let turn = start_web_turn_with_selected_attachments_and_roles(
        &state,
        "session_a",
        "inspect this",
        None,
        Some("role_history_command"),
        selected_roles.clone(),
    )
    .unwrap();
    let records = read_all_history_records(
        &current_session_store(&state)
            .unwrap()
            .history_path_for_session("session_a"),
    )
    .unwrap();
    let restored = restored_turns_from_history_records(&records);
    let restored_entry = restored
        .iter()
        .find(|candidate| candidate.turn_id == turn.turn_id)
        .and_then(|candidate| candidate.user_entries.first())
        .unwrap();
    assert_eq!(restored_entry.worker_roles, selected_roles);
}

#[test]
fn structured_sub_answers_survive_raw_history_reconstruction() {
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_sub_answer".to_string(),
            created_at_ms: 1,
            kind: Some("task".to_string()),
            command_id: None,
            delivery_state: None,
            content: "answer both questions".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "turn_sub_answer".to_string(),
            created_at_ms: 2,
            kind: ChatHistoryEventKind::SubAnswer,
            content: "shown".to_string(),
            extra: BTreeMap::from([
                (
                    "source".to_string(),
                    Value::String("core_topic".to_string()),
                ),
                (
                    "payload".to_string(),
                    json!({
                        "session_id": "session_a",
                        "topic": {"name": CORE_TOPIC_SUB_ANSWER},
                        "state": {"name": "running"},
                        "payload": {
                            "sub_answer_id": "sub_answer_1",
                            "ordinal": 1,
                            "task": "First question",
                            "answer": "First answer"
                        }
                    }),
                ),
            ]),
        },
    ];

    let restored = restored_turns_from_history_records(&records);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].sub_answers.len(), 1);
    assert_eq!(restored[0].sub_answers[0].sub_answer_id, "sub_answer_1");
    assert_eq!(restored[0].sub_answers[0].ordinal, 1);
    assert_eq!(restored[0].sub_answers[0].task, "First question");
    assert_eq!(restored[0].sub_answers[0].answer, "First answer");
}

#[test]
fn multiple_worker_roles_resolve_in_message_order_and_render_all_contexts() {
    let state = routing_test_state();
    let roles = vec![
        WorkerRole {
            id: "role_reviewer".to_string(),
            name: "Reviewer".to_string(),
            description: "Review evidence.".to_string(),
        },
        WorkerRole {
            id: "role_tester".to_string(),
            name: "Tester".to_string(),
            description: "Run regression tests.".to_string(),
        },
    ];
    {
        let mut mem = state.mem.lock().unwrap();
        mem.role_library.roles = roles.clone();
    }
    for session in state.sessions.lock().unwrap().values_mut() {
        session.roles = roles.clone();
    }
    let selected = resolve_worker_roles(
        &state,
        "session_a",
        &[
            "role_tester".to_string(),
            "role_reviewer".to_string(),
            "role_tester".to_string(),
        ],
        None,
    )
    .unwrap();
    assert_eq!(selected, vec![roles[1].clone(), roles[0].clone()]);
    let context = worker_roles_context(&selected).unwrap();
    assert!(context.contains(r#""name":"Tester""#));
    assert!(context.contains(r#""name":"Reviewer""#));
    assert!(context.find("Tester").unwrap() < context.find("Reviewer").unwrap());
    assert_eq!(
        context
            .matches(agent_core::WORKER_ROLE_CONTEXT_PREFIX)
            .count(),
        2
    );
    assert_eq!(
        resolve_worker_roles(&state, "session_a", &["missing".to_string()], None).unwrap_err(),
        "worker_role_not_found"
    );
}

#[test]
fn turn_submit_wire_accepts_multiple_worker_role_ids_and_legacy_single_id() {
    let multi: ClientCommand = serde_json::from_value(json!({
        "type": "turn_submit",
        "session_id": "session_a",
        "text": "review and test",
        "role_ids": ["role_reviewer", "role_tester"]
    }))
    .unwrap();
    let ClientCommand::TurnSubmit {
        role_ids, role_id, ..
    } = multi
    else {
        panic!("expected turn_submit")
    };
    assert_eq!(role_ids, vec!["role_reviewer", "role_tester"]);
    assert_eq!(role_id, None);

    let legacy: ClientCommand = serde_json::from_value(json!({
        "type": "turn_submit",
        "session_id": "session_a",
        "text": "review",
        "role_id": "role_reviewer"
    }))
    .unwrap();
    let ClientCommand::TurnSubmit {
        role_ids, role_id, ..
    } = legacy
    else {
        panic!("expected legacy turn_submit")
    };
    assert!(role_ids.is_empty());
    assert_eq!(role_id.as_deref(), Some("role_reviewer"));
}

#[test]
fn chat_message_delete_removes_user_and_assistant_content_from_ui_state_and_raw_log() {
    let state = routing_test_state();
    let turn = start_web_turn(&state, "session_a", "original task").unwrap();
    append_turn_user_entry(
        &state,
        "session_a",
        "supplement",
        "delete this supplement".to_string(),
    )
    .unwrap();
    append_message(
        &state,
        "session_a",
        "assistant",
        "delete this answer".to_string(),
    )
    .unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.active_turn_id = None;
        session.pending_turn_id = None;
        session.state = "ready".to_string();
        let stored_turn = session
            .turns
            .iter_mut()
            .find(|candidate| candidate.turn_id == turn.turn_id)
            .unwrap();
        stored_turn.state = "finished".to_string();
        stored_turn.final_answer = Some("delete this answer".to_string());
    }

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ChatMessageDelete {
            session_id: "session_a".to_string(),
            turn_id: turn.turn_id.clone(),
            role: "user".to_string(),
            role_index: 1,
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        event,
        WireEvent::ChatMessageDeleted { role, role_index: 1, .. } if role == "user"
    ));

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ChatMessageDelete {
            session_id: "session_a".to_string(),
            turn_id: turn.turn_id.clone(),
            role: "assistant".to_string(),
            role_index: 0,
        },
    )
    .unwrap();

    let session = state
        .sessions
        .lock()
        .unwrap()
        .get("session_a")
        .unwrap()
        .clone();
    let stored_turn = session
        .turns
        .iter()
        .find(|candidate| candidate.turn_id == turn.turn_id)
        .unwrap();
    assert_eq!(
        stored_turn
            .user_entries
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>(),
        vec!["original task"]
    );
    assert!(stored_turn.final_answer.is_none());
    assert!(!session
        .messages
        .iter()
        .any(|message| message.text == "delete this supplement"
            || message.text == "delete this answer"));

    let history = read_all_history_records(
        &current_session_store(&state)
            .unwrap()
            .history_path_for_session("session_a"),
    )
    .unwrap();
    let serialized = serde_json::to_string(&history).unwrap();
    assert!(serialized.contains("original task"));
    assert!(!serialized.contains("delete this supplement"));
    assert!(!serialized.contains("delete this answer"));
}

#[test]
fn chat_message_delete_rejects_an_active_turn_without_touching_raw_log() {
    let state = routing_test_state();
    let turn = start_web_turn(&state, "session_a", "still executing").unwrap();

    let error = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ChatMessageDelete {
            session_id: "session_a".to_string(),
            turn_id: turn.turn_id,
            role: "user".to_string(),
            role_index: 0,
        },
    )
    .unwrap_err();
    assert_eq!(error, "active_turn_message_delete_not_allowed");
    let raw = std::fs::read_to_string(
        current_session_store(&state)
            .unwrap()
            .history_path_for_session("session_a"),
    )
    .unwrap();
    assert!(raw.contains("still executing"));
}

#[test]
fn production_semantic_envelope_has_one_sequence_and_one_nested_event() {
    let envelope = semantic_event_envelope(
        42,
        WireEvent::SessionRenamed {
            session_id: "session_a".to_string(),
            display_name: "Renamed".to_string(),
        },
    );
    let wire = serde_json::to_value(envelope).unwrap();
    assert_eq!(wire["type"], "semantic_event");
    assert_eq!(wire["event_seq"], 42);
    assert_eq!(wire["event"]["type"], "session_renamed");
    assert_eq!(wire["event"]["session_id"], "session_a");
    assert!(wire.get("session_id").is_none());
}

#[test]
fn reliable_mutation_returns_only_ack_while_authoritative_result_is_enveloped() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "ENVELOPE_ONLY_MUTATION");
    let command_id = "rename_envelope_only";
    assert!(reserve_command_dedup(&state, command_id).unwrap().is_none());
    let mut live = state.events.subscribe();
    let completion = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: Some(command_id.to_string()),
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::SessionRename {
                session_id: session_id.clone(),
                display_name: "Envelope only".to_string(),
            },
        },
    );
    assert!(completion.event.is_none());
    assert!(matches!(
        completion.ack,
        Some(WireEvent::CommandAck {
            status: CommandAckStatus::Committed,
            ..
        })
    ));
    let envelope = serde_json::to_value(live.try_recv().unwrap()).unwrap();
    assert_eq!(envelope["type"], "semantic_event");
    assert_eq!(envelope["event_seq"], 1);
    assert_eq!(envelope["event"]["type"], "session_renamed");
    assert_eq!(envelope["event"]["session_id"], session_id);
}

#[test]
fn lagged_live_receiver_requires_a_fresh_snapshot_baseline() {
    let state = routing_test_state();
    let mut receiver = state.events.subscribe();
    let count = EVENT_CHANNEL_CAPACITY + 17;
    for ordinal in 0..count {
        publish_semantic(
            &state,
            WireEvent::SessionRenamed {
                session_id: "session_a".to_string(),
                display_name: format!("rename-{ordinal}"),
            },
        );
    }
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Lagged(_))
    ));
    assert_eq!(state.semantic_delivery.baseline(), count as u64);
    assert!(!current_mem_state(&state)
        .unwrap()
        .layout
        .memory_dir()
        .join("web_events.ndjson")
        .exists());
}

#[test]
fn command_dedup_terminal_result_survives_restart_without_persisting_secrets() {
    let dir = std::env::temp_dir().join(unique_web_id("command_dedup_test"));
    let path = dir.join("dedup.json");
    let mut cache = CommandDedupCache::default();
    assert!(cache.reserve("rename_1").is_none());
    cache.finish(
        "rename_1",
        CommandDedupState::Committed {
            event: None,
            serialized_event: durable_command_result(&WireEvent::SessionRenamed {
                session_id: "session_a".to_string(),
                display_name: "Durable".to_string(),
            }),
        },
    );
    cache.save(&path).unwrap();
    assert!(matches!(
        CommandDedupCache::load(&path).unwrap().reserve("rename_1"),
        Some(CommandDedupState::Committed {
            serialized_event: Some(_),
            ..
        })
    ));
    assert!(durable_command_result(&WireEvent::SessionApiKeyRevealed {
        session_id: "session_a".to_string(),
        api_key: "secret".to_string(),
    })
    .is_none());
    assert!(
        durable_command_result(&WireEvent::ModelEndpointSecretRevealed {
            endpoint_id: "endpoint_a".to_string(),
            api_key: "secret".to_string(),
            http_headers: Default::default(),
            request_fields: Default::default(),
        })
        .is_none()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupt_command_dedup_cache_is_backed_up_without_blocking_web_startup() {
    let dir = std::env::temp_dir().join(unique_web_id("corrupt_command_dedup"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("web_command_dedup.json");
    std::fs::write(&path, b"not-json").unwrap();

    let mut cache = load_command_dedup_resilient(&path).unwrap();
    assert!(cache.reserve("new-command").is_none());
    let backups = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("web_command_dedup.json.command-dedup-corrupt-backup-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"not-json");
    assert!(CommandDedupCache::load(&path).is_ok());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupt_mcp_config_is_backed_up_without_blocking_web_startup() {
    let data_dir = std::env::temp_dir().join(unique_web_id("corrupt_mcp_config"));
    let space = ".mcp_recovery";
    let memory_dir = RuntimeDataLayout::new(&data_dir, space).memory_dir();
    std::fs::create_dir_all(&memory_dir).unwrap();
    let path = memory_dir.join("mcp_servers.json");
    std::fs::write(&path, b"not-json").unwrap();

    let mem = WebMemState::new(data_dir.clone(), space.to_string()).unwrap();
    assert!(mem.mcp_configs.is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]\n");
    let backups = std::fs::read_dir(&memory_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("mcp_servers.json.mcp-config-corrupt-backup-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"not-json");
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn corrupt_worker_role_library_is_backed_up_without_blocking_web_startup() {
    let data_dir = std::env::temp_dir().join(unique_web_id("corrupt_worker_roles"));
    let space = ".worker_role_recovery";
    let memory_dir = RuntimeDataLayout::new(&data_dir, space).memory_dir();
    std::fs::create_dir_all(&memory_dir).unwrap();
    let path = role_library_path(&memory_dir);
    std::fs::write(&path, b"not-json").unwrap();

    let mem = WebMemState::new(data_dir.clone(), space.to_string()).unwrap();
    assert_eq!(mem.role_library, WorkerRoleLibrary::default());
    assert_eq!(
        load_role_library(&path).unwrap(),
        WorkerRoleLibrary::default()
    );
    let backups = std::fs::read_dir(&memory_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("worker_roles.json.worker-roles-corrupt-backup-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(backups[0].path()).unwrap(), b"not-json");
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn oversized_worker_role_library_is_moved_aside_without_loading_it_into_memory() {
    let data_dir = std::env::temp_dir().join(unique_web_id("oversized_worker_roles"));
    let space = ".worker_role_oversized_recovery";
    let memory_dir = RuntimeDataLayout::new(&data_dir, space).memory_dir();
    std::fs::create_dir_all(&memory_dir).unwrap();
    let path = role_library_path(&memory_dir);
    let original = vec![b'x'; MAX_ROLE_FILE_BYTES as usize + 1];
    std::fs::write(&path, &original).unwrap();

    let mem = WebMemState::new(data_dir.clone(), space.to_string()).unwrap();
    assert_eq!(mem.role_library, WorkerRoleLibrary::default());
    let backup = std::fs::read_dir(&memory_dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("worker_roles.json.worker-roles-corrupt-backup-")
        })
        .unwrap();
    assert_eq!(
        std::fs::metadata(backup.path()).unwrap().len(),
        original.len() as u64
    );
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn structurally_invalid_worker_role_library_is_quarantined_on_startup() {
    let data_dir = std::env::temp_dir().join(unique_web_id("invalid_worker_roles"));
    let space = ".worker_role_validation_recovery";
    let memory_dir = RuntimeDataLayout::new(&data_dir, space).memory_dir();
    std::fs::create_dir_all(&memory_dir).unwrap();
    let path = role_library_path(&memory_dir);
    let original = br#"{"roles":[{"id":"bad id","name":"Broken","description":"Invalid id"},{"id":"role_valid","name":"Reviewer","description":"Review"}],"groups":[]}"#;
    std::fs::write(&path, original).unwrap();

    let mem = WebMemState::new(data_dir.clone(), space.to_string()).unwrap();
    assert_eq!(mem.role_library.roles.len(), 1);
    assert_eq!(mem.role_library.roles[0].id, "role_valid");
    let backup = std::fs::read_dir(&memory_dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("worker_roles.json.worker-roles-corrupt-backup-")
        })
        .unwrap();
    assert_eq!(std::fs::read(backup.path()).unwrap(), original);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn corrupt_session_index_record_is_backed_up_while_valid_sessions_remain_usable() {
    let memory_dir = std::env::temp_dir().join(unique_web_id("corrupt_session_index"));
    let store = SessionStore::new(&memory_dir);
    std::fs::create_dir_all(store.sessions_dir()).unwrap();
    let valid = StoredSession {
        session_id: "session-valid".to_string(),
        display_name: "Recovered".to_string(),
        created_at_ms: 1,
        updated_at_ms: 2,
        current_dir: memory_dir.display().to_string(),
        profile: StoredSessionProfile::default(),
        env: BTreeMap::new(),
        env_overrides: None,
        mcp_server_ids: Vec::new(),
        state: StoredSessionState::Ready,
        last_turn_id: None,
        raw_chat_history_path: memory_dir
            .join("sessions/session-valid/raw_chat_history.jsonl")
            .display()
            .to_string(),
        group_id: None,
    };
    let valid_line = serde_json::to_string(&valid).unwrap();
    let original = format!("{valid_line}\nnot-json\n");
    std::fs::write(store.index_path(), &original).unwrap();

    let recovered = list_stored_sessions_resilient(&store).unwrap();
    assert_eq!(recovered, vec![valid]);
    assert_eq!(store.list_sessions().unwrap().len(), 1);
    let backups = std::fs::read_dir(store.sessions_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("index.jsonl.session-index-corrupt-backup-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read_to_string(backups[0].path()).unwrap(),
        original
    );
    let _ = std::fs::remove_dir_all(memory_dir);
}

#[test]
fn concurrent_same_command_id_has_one_executor_but_distinct_ids_both_execute() {
    let cache = Arc::new(Mutex::new(CommandDedupCache::default()));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let executions = Arc::new(AtomicUsize::new(0));
    let threads = (0..8)
        .map(|_| {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            let executions = Arc::clone(&executions);
            thread::spawn(move || {
                barrier.wait();
                if cache.lock().unwrap().reserve("same_id").is_none() {
                    executions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(executions.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(cache.lock().unwrap().reserve("different_id_a").is_none());
    assert!(cache.lock().unwrap().reserve("different_id_b").is_none());
}

#[test]
fn command_lanes_serialize_one_session_without_globally_serializing_other_sessions() {
    let state = routing_test_state();
    let command = |session_id: &str| ClientCommand::TurnCancel {
        session_id: session_id.to_string(),
        target_command_id: None,
    };
    let (_, session_a_first) = command_lane(&state, &command("session_a")).unwrap();
    let (_, session_a_second) = command_lane(&state, &command("session_a")).unwrap();
    let (_, session_b) = command_lane(&state, &command("session_b")).unwrap();
    assert!(Arc::ptr_eq(&session_a_first, &session_a_second));
    assert!(!Arc::ptr_eq(&session_a_first, &session_b));

    let held_a = session_a_first
        .enter(session_a_first.issue().unwrap())
        .unwrap();
    assert_eq!(session_a_second.state.lock().unwrap().serving_ticket, 0);
    let held_b = session_b.enter(session_b.issue().unwrap()).unwrap();
    drop(held_b);
    drop(held_a);
    assert!(session_a_second
        .enter(session_a_second.issue().unwrap())
        .is_ok());
}

#[test]
fn completed_command_lanes_are_reclaimed_without_removing_a_lane_with_pending_tickets() {
    let state = routing_test_state();
    for ordinal in 0..256 {
        let command = ClientCommand::TurnCancel {
            session_id: format!("ephemeral-session-{ordinal}"),
            target_command_id: None,
        };
        let (key, lane) = command_lane(&state, &command).unwrap();
        let accepted = AcceptedCommandLane {
            key,
            ticket: lane.issue().unwrap(),
            lane: Arc::clone(&lane),
            lanes: Arc::clone(&state.command_lanes),
        };
        let guard = lane.enter(accepted.ticket).unwrap();
        drop(guard);
        drop(lane);
        drop(accepted);
    }
    assert!(
        state.command_lanes.lock().unwrap().is_empty(),
        "completed one-shot session IDs must not grow the lane map forever"
    );

    let command = ClientCommand::TurnCancel {
        session_id: "shared-pending-session".to_string(),
        target_command_id: None,
    };
    let (first_key, lane) = command_lane(&state, &command).unwrap();
    let first = AcceptedCommandLane {
        key: first_key,
        ticket: lane.issue().unwrap(),
        lane: Arc::clone(&lane),
        lanes: Arc::clone(&state.command_lanes),
    };
    let (second_key, same_lane) = command_lane(&state, &command).unwrap();
    let second = AcceptedCommandLane {
        key: second_key,
        ticket: same_lane.issue().unwrap(),
        lane: Arc::clone(&same_lane),
        lanes: Arc::clone(&state.command_lanes),
    };
    assert!(Arc::ptr_eq(&lane, &same_lane));
    let first_guard = lane.enter(first.ticket).unwrap();
    drop(first_guard);
    drop(first);
    assert_eq!(
        state.command_lanes.lock().unwrap().len(),
        1,
        "the map must retain the FIFO lane while a later accepted ticket exists"
    );
    let second_guard = same_lane.enter(second.ticket).unwrap();
    drop(second_guard);
    drop(lane);
    drop(same_lane);
    drop(second);
    assert!(state.command_lanes.lock().unwrap().is_empty());
}

#[test]
fn ticket_lane_is_fifo_even_when_a_later_waiter_is_scheduled_aggressively() {
    let lane = Arc::new(TicketCommandLane::default());
    let held = lane.enter(lane.issue().unwrap()).unwrap();
    let (order_tx, order_rx) = std::sync::mpsc::channel();

    let first = {
        let lane = Arc::clone(&lane);
        let order_tx = order_tx.clone();
        thread::spawn(move || {
            let ticket = lane.issue().unwrap();
            let _guard = lane.enter(ticket).unwrap();
            order_tx.send("first").unwrap();
        })
    };
    while lane.state.lock().unwrap().next_ticket < 2 {
        thread::yield_now();
    }
    let second = {
        let lane = Arc::clone(&lane);
        thread::spawn(move || {
            let ticket = lane.issue().unwrap();
            let _guard = lane.enter(ticket).unwrap();
            order_tx.send("second").unwrap();
        })
    };
    while lane.state.lock().unwrap().next_ticket < 3 {
        thread::yield_now();
    }
    drop(held);

    assert_eq!(order_rx.recv().unwrap(), "first");
    assert_eq!(order_rx.recv().unwrap(), "second");
    first.join().unwrap();
    second.join().unwrap();
}

#[test]
fn skipped_queue_full_ticket_does_not_block_the_next_accepted_command() {
    let lane = Arc::new(TicketCommandLane::default());
    let first_ticket = lane.issue().unwrap();
    let rejected_ticket = lane.issue().unwrap();
    let next_ticket = lane.issue().unwrap();
    let first = lane.enter(first_ticket).unwrap();
    lane.skip(rejected_ticket).unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let waiter_lane = Arc::clone(&lane);
    let waiter = thread::spawn(move || {
        let _guard = waiter_lane.enter(next_ticket).unwrap();
        entered_tx.send(()).unwrap();
    });
    assert!(entered_rx.try_recv().is_err());
    drop(first);
    entered_rx.recv().unwrap();
    waiter.join().unwrap();
}

#[test]
fn global_writer_waits_for_all_session_readers_and_blocks_new_readers() {
    let barrier = Arc::new(RwLock::new(()));
    let reader_a = barrier.read().unwrap();
    let reader_b = barrier.read().unwrap();
    let (writer_entered_tx, writer_entered_rx) = std::sync::mpsc::channel();
    let (writer_release_tx, writer_release_rx) = std::sync::mpsc::channel();
    let writer_barrier = Arc::clone(&barrier);
    let writer = thread::spawn(move || {
        let _writer = writer_barrier.write().unwrap();
        writer_entered_tx.send(()).unwrap();
        writer_release_rx.recv().unwrap();
    });

    assert!(writer_entered_rx.try_recv().is_err());
    drop(reader_a);
    assert!(writer_entered_rx.try_recv().is_err());
    drop(reader_b);
    writer_entered_rx.recv().unwrap();
    assert!(barrier.try_read().is_err());
    writer_release_tx.send(()).unwrap();
    writer.join().unwrap();
    assert!(barrier.read().is_ok());
}

#[test]
fn queued_command_from_old_mem_epoch_is_rejected_before_domain_execution() {
    let state = routing_test_state();
    *state.mem_epoch.write().unwrap() = 2;
    let completion = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: None,
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::SessionRename {
                session_id: "session_a".to_string(),
                display_name: "must not run".to_string(),
            },
        },
    );
    assert_eq!(
        completion.legacy_error.as_deref(),
        Some("command_mem_epoch_stale")
    );
    assert_ne!(
        state.sessions.lock().unwrap()["session_a"].display_name,
        "must not run"
    );
}

#[test]
fn terminal_persist_failure_never_reports_an_applied_effect_as_rejected() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "TERMINAL_PERSIST_FAILURE");
    let command_id = "rename_terminal_persist_failure";
    assert!(reserve_command_dedup(&state, command_id).unwrap().is_none());
    let blocker = std::env::temp_dir().join(unique_web_id("dedup_parent_blocker"));
    std::fs::write(&blocker, b"not a directory").unwrap();
    state.mem.lock().unwrap().layout = RuntimeDataLayout::new(&blocker, ".blocked");

    let completion = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: Some(command_id.to_string()),
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::SessionRename {
                session_id: session_id.clone(),
                display_name: "Applied despite terminal journal failure".to_string(),
            },
        },
    );
    assert!(matches!(
        completion.ack,
        Some(WireEvent::CommandAck {
            status: CommandAckStatus::Accepted,
            error: Some(ref error),
            ..
        }) if error.starts_with("command_terminal_persist_pending:")
    ));
    assert_eq!(
        state.sessions.lock().unwrap()[&session_id].display_name,
        "Applied despite terminal journal failure"
    );
    let _ = std::fs::remove_file(blocker);
}

#[test]
fn accepted_record_survives_restart_as_uncertain_instead_of_reexecuting() {
    let dir = std::env::temp_dir().join(unique_web_id("uncertain_restart"));
    let path = dir.join("dedup.json");
    let mut before_crash = CommandDedupCache::default();
    assert!(before_crash.reserve("uncertain_non_idempotent").is_none());
    before_crash.save(&path).unwrap();

    let mut after_restart = CommandDedupCache::load(&path).unwrap();
    assert!(matches!(
        after_restart.reserve("uncertain_non_idempotent"),
        Some(CommandDedupState::Accepted)
    ));
    assert_eq!(after_restart.records.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn all_accepted_command_cache_is_bounded_instead_of_evicting_ownership() {
    let state = routing_test_state();
    {
        let mut cache = state.command_dedup.lock().unwrap();
        for ordinal in 0..COMMAND_DEDUP_CAPACITY {
            assert!(cache.reserve(&format!("accepted_{ordinal}")).is_none());
        }
    }

    assert!(matches!(
        reserve_command_dedup(&state, "one_too_many"),
        Err(error) if error == "command_dedup_capacity_exhausted"
    ));
    let cache = state.command_dedup.lock().unwrap();
    assert_eq!(cache.records.len(), COMMAND_DEDUP_CAPACITY);
    assert!(cache.records.contains_key("accepted_0"));
    assert!(!cache.records.contains_key("one_too_many"));
}

#[test]
fn core_acceptance_ack_is_correlated_live_control_not_semantic_delivery_data() {
    let state = routing_test_state();
    let command_id = "core_acceptance_control_ack";
    start_web_turn_with_command_id(
        &state,
        "session_a",
        "durable command awaiting Core",
        Some(command_id),
    )
    .unwrap();
    assert!(reserve_command_dedup(&state, command_id).unwrap().is_none());
    let baseline = state.semantic_delivery.baseline();
    let mut live = state.events.subscribe();
    mark_core_command_accepted(&state, "session_a", command_id);
    let event = live.try_recv().unwrap();
    assert!(
        matches!(event, WireEvent::CommandAck { command_id: ref actual, status: CommandAckStatus::Committed, .. } if actual == command_id)
    );
    assert_eq!(state.semantic_delivery.baseline(), baseline);
}

#[test]
fn persisted_turn_command_id_prevents_reexecution_after_terminal_ack_loss() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "DELIVERY_REPLAY");
    let command_id = "turn_submit_crash_window";
    let first = start_web_turn_with_command_id(
        &state,
        &session_id,
        "perform exactly once",
        Some(command_id),
    )
    .unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.turns.last_mut().unwrap().user_entries[0].delivery_state =
            Some(ChatCommandDeliveryState::CoreAccepted);
        session.active_turn_id = None;
        session.state = "ready".to_string();
    }
    assert_eq!(
        state.sessions.lock().unwrap()[&session_id]
            .turns
            .last()
            .unwrap()
            .user_entries[0]
            .delivery_state,
        Some(ChatCommandDeliveryState::CoreAccepted)
    );

    let replay = handle_command_with_id(
        &state,
        TEST_PORT,
        Some(command_id),
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "perform exactly once".to_string(),
            input_kind: None,
            source_turn_id: None,
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::TurnUpdated { turn, .. } = replay else {
        panic!("replay should return the original domain result")
    };
    assert_eq!(turn.turn_id, first.turn_id);
    assert_eq!(state.sessions.lock().unwrap()[&session_id].turns.len(), 1);
}

#[test]
fn pending_turn_identity_is_serialized_for_refresh_recovery() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "PENDING_SNAPSHOT");
    let pending = start_web_turn_with_command_id(
        &state,
        &session_id,
        "recover this pending intent",
        Some("submit-refresh"),
    )
    .unwrap();

    let snapshot = serde_json::to_value(snapshot_for(&state, TEST_PORT)).unwrap();
    let serialized = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == session_id)
        .unwrap();
    assert_eq!(serialized["pending_turn_id"], pending.turn_id);
    assert!(serialized["active_turn_id"].is_null());
    assert_eq!(
        serialized["turns"][0]["user_entries"][0]["command_id"],
        "submit-refresh"
    );
}

#[test]
fn accepted_targeted_cancel_is_serialized_until_the_target_turn_finishes() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "CANCELLING_SNAPSHOT");
    let pending = start_web_turn_with_command_id(
        &state,
        &session_id,
        "cancel and refresh",
        Some("submit-cancelling"),
    )
    .unwrap();

    let (context_id, worker_id) = primary_worker_scope(&state, &session_id).unwrap();
    handle_scoped_worker_event(
        &state,
        &session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::TurnStarted {
            command_id: Some("submit-cancelling".to_string()),
        },
    );

    let mut events = state.events.subscribe();
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.clone(),
            target_command_id: Some("submit-cancelling".to_string()),
        },
    )
    .unwrap()
    .is_none());
    assert!(drain_wire_events(&mut events).iter().any(|event| matches!(
        event,
        WireEvent::TurnCancelling {
            session,
            target_command_id: Some(target_command_id),
        } if session.session_id == session_id
            && session.cancelling_turn_id.as_deref() == Some(pending.turn_id.as_str())
            && session.turns.iter().any(|turn| turn.turn_id == pending.turn_id
                && turn.state == "finished"
                && turn.completion.as_ref().and_then(|value| value["stop_reason"].as_str())
                    == Some("CancelledByUser"))
            && target_command_id == "submit-cancelling"
    )));

    let snapshot = serde_json::to_value(snapshot_for(&state, TEST_PORT)).unwrap();
    let serialized = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == session_id)
        .unwrap();
    assert!(serialized["pending_turn_id"].is_null());
    assert_eq!(serialized["cancelling_turn_id"], pending.turn_id);
    assert_eq!(serialized["active_turn_id"], pending.turn_id);
    assert_eq!(serialized["turns"][0]["state"], "finished");
    assert_eq!(
        serialized["turns"][0]["completion"]["stop_reason"],
        "CancelledByUser"
    );
    assert_eq!(serialized["state"], "ready");
    assert!(serialized["workers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|worker| worker["state"] != "working"));
}

#[test]
fn late_core_working_events_cannot_revive_an_accepted_cancellation() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "CANCELLING_LATE_EVENTS");
    let command_id = "submit-cancelling-late";
    let pending = start_web_turn_with_command_id(
        &state,
        &session_id,
        "cancel before late Core events",
        Some(command_id),
    )
    .unwrap();
    let (context_id, worker_id) = primary_worker_scope(&state, &session_id).unwrap();

    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.clone(),
            target_command_id: Some(command_id.to_string()),
        },
    )
    .unwrap()
    .is_none());

    handle_scoped_worker_event(
        &state,
        &session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::TurnStarted {
            command_id: Some(command_id.to_string()),
        },
    );
    handle_scoped_worker_event(
        &state,
        &session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(
        session.cancelling_turn_id.as_deref(),
        Some(pending.turn_id.as_str())
    );
    assert_eq!(
        session.active_turn_id.as_deref(),
        Some(pending.turn_id.as_str())
    );
    assert_eq!(session.pending_turn_id, None);
    assert_eq!(session.turns.last().unwrap().state, "finished");
    assert_eq!(
        session.turns.last().unwrap().completion.as_ref().unwrap()["stop_reason"],
        "CancelledByUser"
    );
    assert_eq!(session.state, "ready");
    assert!(session
        .workers
        .iter()
        .all(|worker| worker.state != "working"));
    drop(sessions);

    let snapshot = serde_json::to_value(snapshot_for(&state, TEST_PORT)).unwrap();
    let serialized = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["session_id"] == session_id)
        .unwrap();
    assert_eq!(serialized["cancelling_turn_id"], pending.turn_id);
    assert_eq!(serialized["active_turn_id"], pending.turn_id);
    assert_eq!(serialized["state"], "ready");
    assert!(serialized["workers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|worker| worker["state"] != "working"));
}

#[test]
fn targeted_cancel_rejects_a_different_current_turn_without_mutation() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "CANCEL_TARGET_MISMATCH");
    let pending = start_web_turn_with_command_id(
        &state,
        &session_id,
        "do not cancel the wrong turn",
        Some("submit-current"),
    )
    .unwrap();

    let error = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.clone(),
            target_command_id: Some("submit-other".to_string()),
        },
    )
    .unwrap_err();
    assert_eq!(error, "turn_cancel_target_mismatch");

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(
        session.pending_turn_id.as_deref(),
        Some(pending.turn_id.as_str())
    );
    assert_eq!(session.state, "ready");
    assert_eq!(session.turns.last().unwrap().state, "pending");
    assert!(session.turns.last().unwrap().completion.is_none());
}

#[test]
fn core_turn_started_immediately_publishes_authoritative_live_state() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "CORE_STARTED_WIRE");
    let command_id = "core_started_wire_command";
    let pending_turn = start_web_turn_with_command_id(
        &state,
        &session_id,
        "wait for the Core boundary",
        Some(command_id),
    )
    .unwrap();
    let (context_id, worker_id) = {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert_eq!(session.state, "ready");
        assert_eq!(session.active_turn_id, None);
        assert_eq!(
            session.pending_turn_id.as_deref(),
            Some(pending_turn.turn_id.as_str())
        );
        assert_eq!(session.turns.last().unwrap().state, "pending");
        assert!(session.workers.iter().all(|worker| worker.state == "ready"));
        let worker = session
            .workers
            .iter()
            .find(|worker| worker.worker_id == session.primary_worker_id)
            .unwrap();
        (worker.context_id.clone(), worker.worker_id.clone())
    };
    let mut events = state.events.subscribe();

    handle_scoped_worker_event(
        &state,
        &session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::TurnStarted {
            command_id: Some(command_id.to_string()),
        },
    );

    let published = drain_wire_events(&mut events);
    let started_turn = published
        .iter()
        .find_map(|event| match event {
            WireEvent::TurnStarted {
                session_id: event_session_id,
                context_id: event_context_id,
                worker_id: event_worker_id,
                turn,
            } => {
                assert_eq!(event_session_id, &session_id);
                assert_eq!(event_context_id, &context_id);
                assert_eq!(event_worker_id, &worker_id);
                Some(turn)
            }
            _ => None,
        })
        .expect("Core TurnStarted must immediately publish browser live state");
    assert_eq!(started_turn.turn_id, pending_turn.turn_id);
    assert_eq!(started_turn.state, "working");
    assert!(published
        .iter()
        .all(|event| !matches!(event, WireEvent::WorkerActivity { .. })));

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(session.state, "working");
    assert_eq!(
        session.active_turn_id.as_deref(),
        Some(pending_turn.turn_id.as_str())
    );
    assert_eq!(session.pending_turn_id, None);
    assert_eq!(session.turns.last().unwrap().state, "working");
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == worker_id)
            .unwrap()
            .state,
        "working"
    );
}

#[test]
fn unmatched_core_turn_started_does_not_activate_an_unrelated_pending_intent() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "UNMATCHED_CORE_STARTED");
    let pending_turn = start_web_turn_with_command_id(
        &state,
        &session_id,
        "this intent belongs to another command",
        Some("expected_command"),
    )
    .unwrap();
    let (context_id, worker_id) = {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert_eq!(
            session.pending_turn_id.as_deref(),
            Some(pending_turn.turn_id.as_str())
        );
        let worker = session
            .workers
            .iter()
            .find(|worker| worker.worker_id == session.primary_worker_id)
            .unwrap();
        (worker.context_id.clone(), worker.worker_id.clone())
    };
    let mut events = state.events.subscribe();

    handle_scoped_worker_event(
        &state,
        &session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::TurnStarted {
            command_id: Some("different_command".to_string()),
        },
    );

    assert!(drain_wire_events(&mut events)
        .iter()
        .all(|event| !matches!(event, WireEvent::TurnStarted { .. })));
    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(session.state, "ready");
    assert_eq!(session.active_turn_id, None);
    assert_eq!(
        session.pending_turn_id.as_deref(),
        Some(pending_turn.turn_id.as_str())
    );
    assert_eq!(session.turns.last().unwrap().state, "pending");
    assert!(session.workers.iter().all(|worker| worker.state == "ready"));
}

#[test]
fn restored_interrupted_marker_targets_only_the_persisted_last_turn() {
    let mut turns = vec![
        WebTurn {
            turn_id: "older_unfinished".to_string(),
            state: "restored".to_string(),
            created_at_ms: 1,
            user_entries: Vec::new(),
            events: Vec::new(),
            sub_answers: Vec::new(),
            final_answer: None,
            completion: None,
        },
        WebTurn {
            turn_id: "persisted_last".to_string(),
            state: "restored".to_string(),
            created_at_ms: 2,
            user_entries: Vec::new(),
            events: Vec::new(),
            sub_answers: Vec::new(),
            final_answer: None,
            completion: None,
        },
    ];

    mark_restored_interrupted_turn(
        &mut turns,
        StoredSessionState::Interrupted,
        Some("persisted_last"),
    );

    assert_eq!(turns[0].state, "restored");
    assert_eq!(turns[1].state, "interrupted");
}

#[test]
fn restored_interrupted_marker_never_relabels_terminal_or_unrelated_turns() {
    let terminal = WebTurn {
        turn_id: "terminal_last".to_string(),
        state: "completed".to_string(),
        created_at_ms: 2,
        user_entries: Vec::new(),
        events: Vec::new(),
        sub_answers: Vec::new(),
        final_answer: None,
        completion: Some(json!({"stop_reason": "model_error"})),
    };
    let older = WebTurn {
        turn_id: "older_unfinished".to_string(),
        state: "restored".to_string(),
        created_at_ms: 1,
        user_entries: Vec::new(),
        events: Vec::new(),
        sub_answers: Vec::new(),
        final_answer: None,
        completion: None,
    };
    let mut turns = vec![older, terminal];

    mark_restored_interrupted_turn(
        &mut turns,
        StoredSessionState::Interrupted,
        Some("terminal_last"),
    );
    assert_eq!(turns[0].state, "restored");
    assert_eq!(turns[1].state, "completed");

    mark_restored_interrupted_turn(
        &mut turns,
        StoredSessionState::Ready,
        Some("older_unfinished"),
    );
    mark_restored_interrupted_turn(&mut turns, StoredSessionState::Interrupted, None);
    mark_restored_interrupted_turn(
        &mut turns,
        StoredSessionState::Interrupted,
        Some("missing_turn"),
    );
    assert_eq!(turns[0].state, "restored");
    assert_eq!(turns[1].state, "completed");
}

#[test]
fn aggregate_session_state_preserves_interrupted_until_real_activity_changes_it() {
    let ready_worker = WebWorker {
        worker_id: "worker".to_string(),
        context_id: "context".to_string(),
        display_name: "Worker".to_string(),
        ordinal: 0,
        state: "ready".to_string(),
        parent_worker_id: None,
    };
    assert_eq!(
        aggregate_web_session_state(std::slice::from_ref(&ready_worker), "interrupted"),
        "interrupted"
    );
    assert_eq!(
        aggregate_web_session_state(std::slice::from_ref(&ready_worker), "ready"),
        "ready"
    );

    let mut working = ready_worker.clone();
    working.state = "working".to_string();
    assert_eq!(
        aggregate_web_session_state(&[working], "interrupted"),
        "working"
    );

    let mut error = ready_worker.clone();
    error.state = "error".to_string();
    assert_eq!(
        aggregate_web_session_state(&[error], "interrupted"),
        "error"
    );

    let mut stopped = ready_worker;
    stopped.state = "stopped".to_string();
    assert_eq!(
        aggregate_web_session_state(&[stopped], "interrupted"),
        "stopped"
    );
    assert_eq!(
        aggregate_web_session_state(&[], "interrupted"),
        "interrupted"
    );
}

#[test]
fn interrupted_session_persists_without_an_active_or_pending_turn() {
    let state = routing_test_state();
    let sessions = state.sessions.lock().unwrap();
    let mut session = sessions["session_a"].clone();
    drop(sessions);
    session.state = "interrupted".to_string();
    session.active_turn_id = None;
    session.pending_turn_id = None;
    session.turns.push(WebTurn {
        turn_id: "interrupted_turn".to_string(),
        state: "interrupted".to_string(),
        created_at_ms: 1,
        user_entries: Vec::new(),
        events: Vec::new(),
        sub_answers: Vec::new(),
        final_answer: None,
        completion: None,
    });

    let stored = stored_session_from_web_session(&state, &session);
    assert_eq!(stored.state, StoredSessionState::Interrupted);
    assert_eq!(stored.last_turn_id.as_deref(), Some("interrupted_turn"));
}

#[test]
fn stale_turn_started_without_command_id_cannot_revive_an_interrupted_session() {
    let state = routing_test_state();
    let session_id = "session_a";
    let worker_id = state.sessions.lock().unwrap()[session_id]
        .primary_worker_id
        .clone();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.state = "interrupted".to_string();
        session.active_turn_id = None;
        session.pending_turn_id = None;
        session.turns.push(WebTurn {
            turn_id: "old_interrupted_turn".to_string(),
            state: "interrupted".to_string(),
            created_at_ms: 1,
            user_entries: Vec::new(),
            events: Vec::new(),
            sub_answers: Vec::new(),
            final_answer: None,
            completion: None,
        });
    }

    assert!(activate_core_started_turn(&state, session_id, &worker_id, None).is_none());
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[session_id].state, "interrupted");
    assert_eq!(
        sessions[session_id].turns.last().unwrap().state,
        "interrupted"
    );
}

#[test]
fn stale_started_event_is_isolated_to_its_session() {
    let state = routing_test_state();
    let worker_a = state.sessions.lock().unwrap()["session_a"]
        .primary_worker_id
        .clone();
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.get_mut("session_a").unwrap().state = "interrupted".to_string();
        sessions.get_mut("session_b").unwrap().state = "ready".to_string();
    }

    assert!(activate_core_started_turn(
        &state,
        "session_a",
        &worker_a,
        Some("stale_before_restart"),
    )
    .is_none());
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions["session_a"].state, "interrupted");
    assert_eq!(sessions["session_b"].state, "ready");
    assert_eq!(sessions["session_b"].active_turn_id, None);
    assert_eq!(sessions["session_b"].pending_turn_id, None);
}

#[test]
fn stale_turn_started_cannot_revive_an_interrupted_turn_but_new_pending_turn_can_start() {
    let state = routing_test_state();
    let session_id = "session_a";
    let worker_id = state.sessions.lock().unwrap()[session_id]
        .primary_worker_id
        .clone();
    let stale_command_id = "stale_before_restart";
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.state = "interrupted".to_string();
        session.turns.push(WebTurn {
            turn_id: "interrupted_turn".to_string(),
            state: "interrupted".to_string(),
            created_at_ms: 1,
            user_entries: vec![WebTurnUserEntry {
                kind: "task".to_string(),
                text: "old work".to_string(),
                attachments: Vec::new(),
                created_at_ms: 1,
                command_id: Some(stale_command_id.to_string()),
                delivery_state: Some(ChatCommandDeliveryState::CoreAccepted),
                worker_roles: Vec::new(),
            }],
            events: Vec::new(),
            sub_answers: Vec::new(),
            final_answer: None,
            completion: None,
        });
    }

    assert!(
        activate_core_started_turn(&state, session_id, &worker_id, Some(stale_command_id),)
            .is_none()
    );
    {
        let sessions = state.sessions.lock().unwrap();
        assert_eq!(sessions[session_id].state, "interrupted");
        assert_eq!(
            sessions[session_id].turns.last().unwrap().state,
            "interrupted"
        );
    }

    let new_turn = start_web_turn_with_command_id(
        &state,
        session_id,
        "new work after restart",
        Some("new_after_restart"),
    )
    .unwrap();
    let started =
        activate_core_started_turn(&state, session_id, &worker_id, Some("new_after_restart"))
            .expect("a registered new pending command may start");
    assert_eq!(started.turn_id, new_turn.turn_id);
    assert_eq!(started.state, "working");
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[session_id].state, "working");
    assert_eq!(sessions[session_id].turns[0].state, "interrupted");
}

#[test]
fn core_runtime_process_restart_marks_running_session_and_latest_unfinished_turn_interrupted_without_redrive(
) {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("restart_interrupted_session"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "restart_interrupted_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();

    let session_id = create_session(
        &state,
        Some("Interrupted on restart".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let turn = start_web_turn_with_command_id(
        &state,
        &session_id,
        "do not resume this command after restart",
        Some("accepted_before_restart"),
    )
    .unwrap();
    {
        let store = current_session_store(&state).unwrap();
        let stored = store.load_session(&session_id).unwrap().unwrap();
        assert_eq!(stored.state, StoredSessionState::Interrupted);
    }

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);
    assert_eq!(
        restore_stored_sessions_after_runtime_restart(&restarted).unwrap(),
        1
    );

    {
        let sessions = restarted.sessions.lock().unwrap();
        let restored = &sessions[&session_id];
        assert_eq!(restored.state, "interrupted");
        assert_eq!(restored.active_turn_id, None);
        assert_eq!(restored.pending_turn_id, None);
        assert!(restored
            .workers
            .iter()
            .all(|worker| worker.state == "ready"));
        let restored_turn = restored
            .turns
            .iter()
            .find(|candidate| candidate.turn_id == turn.turn_id)
            .unwrap();
        assert_eq!(restored_turn.state, "interrupted");
        assert_eq!(restored_turn.final_answer, None);
        assert_eq!(restored_turn.completion, None);
    }

    thread::sleep(Duration::from_millis(30));
    assert!(drain_worker_events(&restarted)
        .iter()
        .all(|(_, _, _, event)| { !matches!(event, CoreSessionWorkerEvent::TurnStarted { .. }) }));
    let persisted = current_session_store(&restarted)
        .unwrap()
        .load_session(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.state, StoredSessionState::Interrupted);

    let manager = {
        let mut guard = restarted.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
}

fn force_session_history_persistence_failure(state: &AppState, label: &str) -> PathBuf {
    let blocker = std::env::temp_dir().join(unique_web_id(label));
    std::fs::write(&blocker, b"not a directory").unwrap();
    state.mem.lock().unwrap().session_store = SessionStore::new(&blocker);
    blocker
}

#[test]
fn failed_new_turn_persistence_rolls_back_all_in_memory_state() {
    let state = routing_test_state();
    let attachment = WebAttachment {
        id: "rollback_attachment".to_string(),
        name: "evidence.txt".to_string(),
        path: "/tmp/evidence.txt".to_string(),
        bytes: 8,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .attachments
        .push(attachment.clone());
    let before = state.sessions.lock().unwrap()["session_a"].clone();
    let blocker = force_session_history_persistence_failure(&state, "turn_persist_blocker");

    assert!(start_web_turn_with_command_id(
        &state,
        "session_a",
        "must not become a ghost turn",
        Some("failed_turn_command"),
    )
    .is_err());

    let after = &state.sessions.lock().unwrap()["session_a"];
    assert_eq!(after.active_turn_id, before.active_turn_id);
    assert_eq!(after.state, before.state);
    assert_eq!(after.turns.len(), before.turns.len());
    assert_eq!(after.messages.len(), before.messages.len());
    assert_eq!(after.attachments, vec![attachment]);
    let _ = std::fs::remove_file(blocker);
}

#[test]
fn failed_supplement_persistence_restores_entry_and_pending_attachments() {
    let state = routing_test_state();
    let turn = start_web_turn(&state, "session_a", "existing task").unwrap();
    let attachment = WebAttachment {
        id: "supplement_rollback_attachment".to_string(),
        name: "late.txt".to_string(),
        path: "/tmp/late.txt".to_string(),
        bytes: 4,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .attachments
        .push(attachment.clone());
    let blocker = force_session_history_persistence_failure(&state, "supplement_persist_blocker");

    assert!(append_turn_supplement_with_pending_attachments(
        &state,
        "session_a",
        "must not become a ghost supplement".to_string(),
        Some("failed_supplement_command"),
    )
    .is_err());

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions["session_a"];
    assert_eq!(
        session.active_turn_id.as_deref(),
        Some(turn.turn_id.as_str())
    );
    assert_eq!(session.turns.last().unwrap().user_entries.len(), 1);
    assert_eq!(session.attachments, vec![attachment]);
    drop(sessions);
    let _ = std::fs::remove_file(blocker);
}

#[test]
fn same_id_racing_across_sockets_has_one_owner_and_distinct_ids_all_execute() {
    const CONNECTIONS: usize = 12;
    let cache = Arc::new(Mutex::new(CommandDedupCache::default()));
    let barrier = Arc::new(std::sync::Barrier::new(CONNECTIONS));
    let same_id_owners = Arc::new(AtomicUsize::new(0));
    let distinct_id_owners = Arc::new(AtomicUsize::new(0));
    let threads = (0..CONNECTIONS)
        .map(|connection| {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            let same_id_owners = Arc::clone(&same_id_owners);
            let distinct_id_owners = Arc::clone(&distinct_id_owners);
            thread::spawn(move || {
                barrier.wait();
                let mut cache = cache.lock().unwrap();
                if cache.reserve("same_command_from_every_socket").is_none() {
                    same_id_owners.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                // Payload equality is irrelevant. A distinct command ID means
                // a distinct user intent and must not be content-deduplicated.
                if cache.reserve(&format!("distinct_{connection}")).is_none() {
                    distinct_id_owners.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(same_id_owners.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        distinct_id_owners.load(std::sync::atomic::Ordering::SeqCst),
        CONNECTIONS
    );
}

#[test]
fn lost_terminal_ack_retry_replays_result_without_a_second_handler_effect() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "ACK_LOSS_RETRY");
    let command_id = "rename_with_lost_ack";
    assert!(reserve_command_dedup(&state, command_id).unwrap().is_none());
    let completion = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: Some(command_id.to_string()),
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::SessionRename {
                session_id: session_id.clone(),
                display_name: "Committed once".to_string(),
            },
        },
    );
    assert!(matches!(
        completion.ack,
        Some(WireEvent::CommandAck {
            status: CommandAckStatus::Committed,
            ..
        })
    ));
    // Simulate losing both the direct result and terminal ack. Retrying the
    // same ID must see the cache and must never execute handle_command again.
    let replay = reserve_command_dedup(&state, command_id).unwrap().unwrap();
    assert!(matches!(replay, CommandDedupState::Committed { .. }));
    assert_eq!(
        state.sessions.lock().unwrap()[&session_id].display_name,
        "Committed once"
    );
}

#[tokio::test]
async fn disconnect_after_acceptance_does_not_abort_queued_commands() {
    let (command_tx, command_rx) = tokio_mpsc::channel(2);
    let (result_tx, result_rx) = tokio_mpsc::unbounded_channel();
    let handled = Arc::new(AtomicUsize::new(0));
    let handled_worker = Arc::clone(&handled);
    let worker = tokio::spawn(run_ordered_blocking_queue(
        command_rx,
        result_tx,
        move |_: usize| {
            handled_worker.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        },
    ));
    command_tx.send(1).await.unwrap();
    command_tx.send(2).await.unwrap();
    drop(result_rx);
    drop(command_tx);
    worker.await.unwrap();
    assert_eq!(handled.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ordered_browser_command_worker_continues_after_one_handler_panics() {
    let (command_tx, command_rx) = tokio_mpsc::channel(2);
    let (result_tx, mut result_rx) = tokio_mpsc::unbounded_channel();
    let worker = tokio::spawn(run_ordered_blocking_queue(
        command_rx,
        result_tx,
        |value: usize| {
            if value == 1 {
                panic!("simulated browser command panic");
            }
            Ok(value)
        },
    ));

    command_tx.send(1).await.unwrap();
    command_tx.send(2).await.unwrap();

    let first = result_rx.recv().await.unwrap().unwrap_err();
    assert!(
        first.starts_with("browser_command_worker_failed:"),
        "{first}"
    );
    assert_eq!(result_rx.recv().await.unwrap().unwrap(), 2);

    drop(command_tx);
    worker.await.unwrap();
}

#[tokio::test]
async fn command_results_from_different_sockets_may_reorder_but_keep_their_ids() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (slow_tx, slow_rx) = tokio_mpsc::channel(1);
    let (slow_result_tx, mut slow_result_rx) = tokio_mpsc::unbounded_channel();
    let slow_gate = Arc::clone(&gate);
    let slow_worker = tokio::spawn(run_ordered_blocking_queue(
        slow_rx,
        slow_result_tx,
        move |command_id: String| {
            let (released, wake) = &*slow_gate;
            let mut released = released.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
            Ok(command_id)
        },
    ));

    let (fast_tx, fast_rx) = tokio_mpsc::channel(1);
    let (fast_result_tx, mut fast_result_rx) = tokio_mpsc::unbounded_channel();
    let fast_worker = tokio::spawn(run_ordered_blocking_queue(
        fast_rx,
        fast_result_tx,
        |command_id: String| Ok(command_id),
    ));

    slow_tx.send("socket_a_command".to_string()).await.unwrap();
    fast_tx.send("socket_b_command".to_string()).await.unwrap();
    assert_eq!(
        fast_result_rx.recv().await.unwrap().unwrap(),
        "socket_b_command"
    );
    assert!(slow_result_rx.try_recv().is_err());

    {
        let (released, wake) = &*gate;
        *released.lock().unwrap() = true;
        wake.notify_all();
    }
    assert_eq!(
        slow_result_rx.recv().await.unwrap().unwrap(),
        "socket_a_command"
    );
    drop(slow_tx);
    drop(fast_tx);
    slow_worker.await.unwrap();
    fast_worker.await.unwrap();
}

#[tokio::test]
async fn ordered_browser_command_worker_keeps_async_runtime_responsive_and_results_ordered() {
    let (commands_tx, commands_rx) = tokio_mpsc::channel(4);
    let (results_tx, mut results_rx) = tokio_mpsc::unbounded_channel();
    let worker = tokio::spawn(run_ordered_blocking_queue(
        commands_rx,
        results_tx,
        |value: usize| {
            thread::sleep(Duration::from_millis(40));
            Ok(value)
        },
    ));
    commands_tx.send(1).await.unwrap();
    commands_tx.send(2).await.unwrap();

    let heartbeat = tokio::time::timeout(Duration::from_millis(20), async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        "responsive"
    })
    .await
    .unwrap();
    assert_eq!(heartbeat, "responsive");
    assert_eq!(results_rx.recv().await.unwrap().unwrap(), 1);
    assert_eq!(results_rx.recv().await.unwrap().unwrap(), 2);
    drop(commands_tx);
    worker.await.unwrap();
}

#[tokio::test]
async fn browser_command_queue_is_bounded_under_click_flood() {
    let (commands_tx, mut commands_rx) = tokio_mpsc::channel::<usize>(2);
    commands_tx.try_send(1).unwrap();
    commands_tx.try_send(2).unwrap();
    assert!(matches!(
        commands_tx.try_send(3),
        Err(tokio_mpsc::error::TrySendError::Full(3))
    ));
    assert_eq!(commands_rx.recv().await, Some(1));
    assert_eq!(commands_rx.recv().await, Some(2));
}

#[test]
fn web_absolute_mem_layout_uses_the_selected_directory_directly() {
    let data_dir = PathBuf::from("/tmp/timem-config");
    let memory_dir = PathBuf::from("/tmp/timem selected mem");
    let layout = web_layout_for_space(&data_dir, memory_dir.to_str().unwrap());

    assert_eq!(layout.memory_dir(), memory_dir);
    assert_eq!(
        layout.api_audit_file(),
        PathBuf::from("/tmp/timem selected mem/audit/api_audit.json")
    );
}

#[test]
fn parses_basic_web_launch_options() {
    let options = WebLaunchOptions::parse(&[
        "--port".to_string(),
        "12345".to_string(),
        "--space".to_string(),
        "web_test".to_string(),
        "--model".to_string(),
        "test-model".to_string(),
    ])
    .unwrap();

    assert_eq!(options.port, Some(12345));
    assert_eq!(options.space.as_deref(), Some("web_test"));
    assert_eq!(options.model.as_deref(), Some("test-model"));
    assert!(!options.public_access);
    assert!(!options.debug);
    assert!(options.open_browser);

    let headless =
        WebLaunchOptions::parse(&["--no-open".to_string(), "--public".to_string()]).unwrap();
    assert!(!headless.open_browser);
    assert!(headless.public_access);

    let advertised = WebLaunchOptions::parse(&[
        "--public".to_string(),
        "--public-host".to_string(),
        "10.125.112.83".to_string(),
    ])
    .unwrap();
    assert_eq!(advertised.public_host.as_deref(), Some("10.125.112.83"));

    let debug = WebLaunchOptions::parse(&["--debug".to_string(), "--no-open".to_string()]).unwrap();
    assert!(debug.debug);
    assert!(!debug.open_browser);
}

#[test]
fn public_url_uses_explicit_host_without_placeholder() {
    assert_eq!(
        public_access_url(Some("10.125.112.83"), 14983, "token"),
        Some("http://10.125.112.83:14983/?token=token".to_string())
    );
    assert_eq!(
        public_access_url(Some("2001:db8::10"), 14983, "token"),
        Some("http://[2001:db8::10]:14983/?token=token".to_string())
    );
    let too_long = "a".repeat(254);
    for host in [
        "",
        "example.com/path",
        "example.com?x=1",
        "example.com#frag",
        "user@example.com",
        "example.com\nSet-Cookie: x=y",
        "example com",
        too_long.as_str(),
    ] {
        assert_eq!(
            public_access_url(Some(host), 14983, "token"),
            None,
            "{host}"
        );
    }
}

#[test]
fn mcp_definition_is_mem_scoped_and_session_enablement_is_isolated() {
    let state = routing_test_state();
    let config = McpServerConfig {
        id: "demo".to_string(),
        name: "Demo MCP".to_string(),
        enabled: true,
        transport: agent_core::mcp::McpTransportConfig::Stdio {
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                r#"while IFS= read -r line; do case "$line" in *\"method\":\"initialize\"*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26","capabilities":{},"serverInfo":{"name":"fake","version":"1"},"instructions":"Use Demo MCP only after validating its input."}}';; *\"method\":\"tools/list\"*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{}}}]}}';; esac; done"#.to_string(),
            ],
            env: BTreeMap::new(),
        },
        request_timeout_ms: 2_000,
    };

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::McpServerUpsert {
            session_id: "session_a".to_string(),
            config,
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(event, WireEvent::McpUpdated { .. }));
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions["session_a"].mcp_server_ids, vec!["demo"]);
    assert!(sessions["session_b"].mcp_server_ids.is_empty());
    assert_ne!(
        sessions["session_a"].mcp_config_revision,
        sessions["session_a"].applied_mcp_config_revision
    );
    drop(sessions);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mem = state.mem.lock().unwrap();
        assert!(mem.mcp_store.file().is_file());
        let connected = mcp_reports(&mem)
            .first()
            .is_some_and(|report| report.state == "connected" && !report.tools.is_empty());
        drop(mem);
        if connected {
            break;
        }
        assert!(Instant::now() < deadline, "MCP discovery did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        mcp_reports(&state.mem.lock().unwrap())[0].tools[0].action_name,
        "mcp_demo_mcp__echo"
    );
    assert_eq!(
        mcp_reports(&state.mem.lock().unwrap())[0]
            .instructions
            .as_deref(),
        Some("Use Demo MCP only after validating its input.")
    );

    assert!(apply_pending_session_mcp(&state, "session_a").unwrap());
    assert!(!apply_pending_session_mcp(&state, "session_a").unwrap());
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(
        sessions["session_a"].mcp_config_revision,
        sessions["session_a"].applied_mcp_config_revision
    );
    drop(sessions);
}

#[test]
fn mcp_toggle_is_deferred_until_the_next_new_turn_boundary() {
    let state = routing_test_state();
    {
        let mut mem = state.mem.lock().unwrap();
        mem.mcp_configs.push(McpServerConfig {
            id: "deferred".to_string(),
            name: "Deferred".to_string(),
            enabled: true,
            transport: agent_core::mcp::McpTransportConfig::Stdio {
                command: "/bin/false".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            request_timeout_ms: 10,
        });
    }

    enable_mcp_for_session(&state, "session_a", true, Some("deferred")).unwrap();
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions["session_a"].mcp_server_ids, vec!["deferred"]);
    let enabled_revision = sessions["session_a"].mcp_config_revision;
    assert_ne!(
        sessions["session_a"].mcp_config_revision,
        sessions["session_a"].applied_mcp_config_revision
    );
    drop(sessions);

    enable_mcp_for_session(&state, "session_a", true, Some("deferred")).unwrap();
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"].mcp_config_revision,
        enabled_revision,
        "repeating the same desired state must not create another pending revision"
    );

    enable_mcp_for_session(&state, "session_a", false, Some("deferred")).unwrap();
    let sessions = state.sessions.lock().unwrap();
    assert!(sessions["session_a"].mcp_server_ids.is_empty());
    assert_ne!(
        sessions["session_a"].mcp_config_revision,
        sessions["session_a"].applied_mcp_config_revision
    );
}

#[test]
fn deleting_mcp_definition_removes_it_from_every_session() {
    let state = routing_test_state();
    {
        let mut mem = state.mem.lock().unwrap();
        mem.mcp_configs.push(McpServerConfig {
            id: "gone".to_string(),
            name: "Gone".to_string(),
            enabled: false,
            transport: agent_core::mcp::McpTransportConfig::Stdio {
                command: "/bin/false".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            request_timeout_ms: 10,
        });
        mem.mcp_store.save(&mem.mcp_configs).unwrap();
    }
    for session in state.sessions.lock().unwrap().values_mut() {
        session.mcp_server_ids.push("gone".to_string());
    }
    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::McpServerDelete {
            server_id: "gone".to_string(),
        },
    )
    .unwrap();
    assert!(state
        .sessions
        .lock()
        .unwrap()
        .values()
        .all(|session| session.mcp_server_ids.is_empty()));
    assert!(state.mem.lock().unwrap().mcp_configs.is_empty());
}

#[test]
fn mcp_snapshot_redacts_secrets_and_edit_preserves_unmodified_values() {
    let state = routing_test_state();
    let config = McpServerConfig {
        id: "remote".to_string(),
        name: "Remote".to_string(),
        enabled: false,
        transport: agent_core::mcp::McpTransportConfig::StreamableHttp {
            url: "https://example.invalid/mcp".to_string(),
            headers: BTreeMap::from([
                ("Authorization".to_string(), "Bearer private".to_string()),
                ("X-Mode".to_string(), "read-only".to_string()),
            ]),
        },
        request_timeout_ms: 100,
    };
    upsert_mcp_server(&state, config).unwrap();
    let report = mcp_reports(&state.mem.lock().unwrap()).remove(0);
    let agent_core::mcp::McpTransportConfig::StreamableHttp { headers, .. } =
        &report.config.transport
    else {
        panic!("expected HTTP")
    };
    assert_eq!(headers["Authorization"], "****");
    assert_eq!(headers["X-Mode"], "read-only");

    let revealed = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::McpServerSecretsReveal {
            server_id: "remote".to_string(),
        },
    )
    .unwrap()
    .expect("secret reveal should reply only to the requesting socket");
    assert!(matches!(
        revealed,
        WireEvent::McpServerSecretsRevealed { ref server_id, ref values }
            if server_id == "remote"
                && values.get("Authorization").map(String::as_str) == Some("Bearer private")
                && !values.contains_key("X-Mode")
    ));

    let mut edited = report.config;
    edited.name = "Renamed".to_string();
    upsert_mcp_server(&state, edited).unwrap();
    let mem = state.mem.lock().unwrap();
    let agent_core::mcp::McpTransportConfig::StreamableHttp { headers, .. } =
        &mem.mcp_configs[0].transport
    else {
        panic!("expected HTTP")
    };
    assert_eq!(headers["Authorization"], "Bearer private");
}

#[test]
fn legacy_sse_snapshot_redacts_sensitive_headers() {
    let state = routing_test_state();
    upsert_mcp_server(
        &state,
        McpServerConfig {
            id: "legacy".to_string(),
            name: "Legacy SSE".to_string(),
            enabled: true,
            transport: agent_core::mcp::McpTransportConfig::Sse {
                url: "https://example.invalid/sse".to_string(),
                headers: BTreeMap::from([
                    ("Authorization".to_string(), "Bearer private".to_string()),
                    ("X-Mode".to_string(), "read-only".to_string()),
                ]),
            },
            request_timeout_ms: 100,
        },
    )
    .unwrap();
    let report = mcp_reports(&state.mem.lock().unwrap()).remove(0);
    let agent_core::mcp::McpTransportConfig::Sse { headers, .. } = report.config.transport else {
        panic!("expected SSE")
    };
    assert_eq!(headers["Authorization"], "****");
    assert_eq!(headers["X-Mode"], "read-only");
}

#[test]
fn web_shutdown_signal_names_cover_terminal_and_service_stops() {
    assert!(web_shutdown_signal_names().contains(&"Ctrl+C"));
    #[cfg(unix)]
    {
        assert!(web_shutdown_signal_names().contains(&"SIGTERM"));
        assert!(web_shutdown_signal_names().contains(&"SIGHUP"));
        assert!(web_shutdown_signal_names().contains(&"parent shell exit"));
    }
}

#[cfg(unix)]
#[test]
fn launch_parent_monitor_detects_shell_exit_and_ignores_a_live_parent() {
    assert!(!launch_parent_has_exited(42, 42));
    assert!(launch_parent_has_exited(42, 1));
    assert!(launch_parent_has_exited(42, 99));
    assert!(!launch_parent_has_exited(1, 1));
    assert!(!launch_parent_has_exited(0, 1));
}

#[test]
fn public_web_launch_keeps_token_auth_and_reports_bind_mode() {
    let mut state = routing_test_state();
    assert!(!state.public_access);
    let local = snapshot_for(&state, TEST_PORT);
    assert_eq!(local.server.bind_host, "127.0.0.1");
    assert!(!local.server.public_access);

    state.public_access = true;
    let public = snapshot_for(&state, TEST_PORT);
    assert_eq!(public.server.bind_host, "0.0.0.0");
    assert!(public.server.public_access);
    assert!(!authorized(
        &state,
        &AuthQuery { token: None },
        &HeaderMap::new()
    ));
    assert!(!authorized(
        &state,
        &AuthQuery {
            token: Some("wrong".to_string()),
        },
        &HeaderMap::new()
    ));
    assert!(!authorized(
        &state,
        &AuthQuery {
            token: Some("te".to_string()),
        },
        &HeaderMap::new()
    ));
    assert!(!authorized(
        &state,
        &AuthQuery {
            token: Some("test-extra".to_string()),
        },
        &HeaderMap::new()
    ));
    assert!(authorized(
        &state,
        &AuthQuery {
            token: Some("test".to_string()),
        },
        &HeaderMap::new()
    ));

    let mut cookie_headers = HeaderMap::new();
    cookie_headers.insert(
        header::COOKIE,
        HeaderValue::from_static("timem_web_token=test"),
    );
    assert!(authorized(
        &state,
        &AuthQuery { token: None },
        &cookie_headers
    ));
    assert!(!authorized(
        &state,
        &AuthQuery {
            token: Some("te".to_string()),
        },
        &cookie_headers
    ));

    let mut partial_cookie_headers = HeaderMap::new();
    partial_cookie_headers.insert(
        header::COOKIE,
        HeaderValue::from_static("timem_web_token=te"),
    );
    assert!(!authorized(
        &state,
        &AuthQuery { token: None },
        &partial_cookie_headers
    ));

    let mut similar_cookie_headers = HeaderMap::new();
    similar_cookie_headers.insert(
        header::COOKIE,
        HeaderValue::from_static("x_timem_web_token=test"),
    );
    assert!(!authorized(
        &state,
        &AuthQuery { token: None },
        &similar_cookie_headers
    ));
}

#[tokio::test]
async fn local_static_web_entry_needs_no_token_and_public_entry_sets_auth_cookie() {
    let local_state = routing_test_state();
    let local = static_asset(
        State((local_state, TEST_PORT)),
        Query(AuthQuery { token: None }),
        HeaderMap::new(),
        Uri::from_static("/"),
    )
    .await;
    assert_eq!(local.status(), StatusCode::OK);
    assert!(local.headers().get(header::SET_COOKIE).is_none());

    let mut state = routing_test_state();
    state.public_access = true;
    let denied = static_asset(
        State((state.clone(), TEST_PORT)),
        Query(AuthQuery { token: None }),
        HeaderMap::new(),
        Uri::from_static("/"),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let allowed = static_asset(
        State((state.clone(), TEST_PORT)),
        Query(AuthQuery {
            token: Some("test".to_string()),
        }),
        HeaderMap::new(),
        Uri::from_static("/"),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(
        allowed
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or(""),
        "timem_web_token=test; Path=/; SameSite=Strict; HttpOnly"
    );

    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_static("timem_web_token=test"),
    );
    let cookie_allowed = static_asset(
        State((state, TEST_PORT)),
        Query(AuthQuery { token: None }),
        headers,
        Uri::from_static("/assets/index.js"),
    )
    .await;
    assert_ne!(cookie_allowed.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reuses_the_same_authenticated_url_after_closing_and_reopening_a_page() {
    let state = routing_test_state();
    for _ in 0..3 {
        let response = static_asset(
            State((state.clone(), TEST_PORT)),
            Query(AuthQuery {
                token: Some("test".to_string()),
            }),
            HeaderMap::new(),
            Uri::from_static("/"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}

// Process-level coverage for the corresponding shutdown/restart behavior is
// provided by scripts/web_runtime_lifecycle_smoke.sh in the production CI gate.
#[test]
fn restarts_timem_web_after_runtime_shutdown_with_the_same_data_and_port() {
    let smoke = include_str!("../../../scripts/web_runtime_lifecycle_smoke.sh");
    assert!(smoke.contains("--port \"$first_port\""));
    assert!(smoke.contains("--space \"$test_root/lifecycle-mem\""));
    assert!(smoke.contains("kill -TERM"));
}

#[tokio::test]
async fn explicit_partial_public_static_token_does_not_fallback_to_cookie() {
    let mut state = routing_test_state();
    state.public_access = true;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        HeaderValue::from_static("timem_web_token=test"),
    );
    let denied = static_asset(
        State((state, TEST_PORT)),
        Query(AuthQuery {
            token: Some("te".to_string()),
        }),
        headers,
        Uri::from_static("/"),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn api_origin_check_requires_same_host_when_origin_is_present() {
    assert!(request_origin_allowed(&HeaderMap::new()));

    let mut same_origin = HeaderMap::new();
    same_origin.insert(header::HOST, HeaderValue::from_static("127.0.0.1:12345"));
    same_origin.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:12345"),
    );
    assert!(request_origin_allowed(&same_origin));

    let mut cross_origin = HeaderMap::new();
    cross_origin.insert(header::HOST, HeaderValue::from_static("127.0.0.1:12345"));
    cross_origin.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://evil.example"),
    );
    assert!(!request_origin_allowed(&cross_origin));

    let mut missing_host = HeaderMap::new();
    missing_host.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:12345"),
    );
    assert!(!request_origin_allowed(&missing_host));
}

#[tokio::test]
async fn api_routes_reject_cross_origin_even_with_valid_token() {
    let state = routing_test_state();
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:12345"));
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://evil.example"),
    );

    let denied = health(
        State((state, TEST_PORT)),
        Query(AuthQuery {
            token: Some("test".to_string()),
        }),
        headers,
    )
    .await;

    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upload_session_id_cannot_escape_upload_root_even_if_registered() {
    let state = routing_test_state();
    let malicious_session_id = "../escape";
    state.sessions.lock().unwrap().insert(
        malicious_session_id.to_string(),
        test_web_session(malicious_session_id, 9, "Bad Session".to_string()),
    );

    let result = store_upload(
        &state,
        malicious_session_id,
        "notes.md".to_string(),
        b"private notes",
    )
    .await;

    assert_eq!(result.unwrap_err(), "invalid_upload_session_id");
    assert!(!state.template.data_dir.join("escape").exists());
    assert!(!state
        .template
        .data_dir
        .join("web_uploads")
        .join("..")
        .join("escape")
        .exists());
}

#[test]
fn rejects_ports_outside_the_local_web_range() {
    let error = WebLaunchOptions::parse(&["--port".to_string(), "12344".to_string()]).unwrap_err();
    assert!(error.contains("12345..=23456"));

    let error = WebLaunchOptions::parse(&["--port".to_string(), "23457".to_string()]).unwrap_err();
    assert!(error.contains("12345..=23456"));
}

#[test]
fn rejects_invalid_numeric_launch_values_instead_of_silently_using_defaults() {
    assert_eq!(
        WebLaunchOptions::parse(&["--timeout".to_string(), "later".to_string()]).unwrap_err(),
        "invalid_timeout"
    );
    assert_eq!(
        WebLaunchOptions::parse(&["--max-llm-input".to_string(), "huge".to_string()]).unwrap_err(),
        "invalid_max_llm_input"
    );
    assert_eq!(
        WebLaunchOptions::parse(&["--max-llm-output".to_string()]).unwrap_err(),
        "missing_value:--max-llm-output"
    );
}

#[test]
fn generated_message_and_upload_ids_remain_unique_within_one_millisecond() {
    let ids = (0..2_000)
        .map(|_| unique_web_id("item"))
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 2_000);
}

#[test]
fn pending_upload_moves_into_the_submitted_user_entry_and_is_not_reinjected() {
    let state = routing_test_state();
    let session_id = "session_a";
    let attachment = WebAttachment {
        id: "upload_1".to_string(),
        name: "notes.md".to_string(),
        path: "/tmp/data/web_uploads/session_a/upload_1_notes.md".to_string(),
        bytes: 42,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut(session_id)
        .unwrap()
        .attachments
        .push(attachment.clone());

    let turn = start_web_turn(&state, session_id, "inspect this file").unwrap();

    assert_eq!(turn.user_entries[0].attachments, vec![attachment.clone()]);
    assert!(state.sessions.lock().unwrap()[session_id]
        .attachments
        .is_empty());
    assert!(
        session_context(&state, session_id, &turn.user_entries[0].attachments)
            .unwrap()
            .unwrap()
            .contains("upload_1_notes.md")
    );
    let empty_context = session_context(&state, session_id, &[]).unwrap();
    assert!(!empty_context
        .as_deref()
        .unwrap_or("")
        .contains("upload_1_notes.md"));

    rollback_web_turn(&state, session_id, &turn.turn_id, vec![attachment.clone()]);
    assert_eq!(
        state.sessions.lock().unwrap()[session_id].attachments,
        vec![attachment]
    );
}

#[test]
fn rolling_back_a_failed_new_turn_preserves_interrupted_session_state() {
    let state = routing_test_state();
    let session_id = "session_a";
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.get_mut(session_id).unwrap().state = "interrupted".to_string();
    }
    let turn = start_web_turn_with_command_id(
        &state,
        session_id,
        "new work that fails before Core accepts it",
        Some("failed_after_restart"),
    )
    .unwrap();

    rollback_web_turn(&state, session_id, &turn.turn_id, Vec::new());

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[session_id];
    assert_eq!(session.state, "interrupted");
    assert_eq!(session.pending_turn_id, None);
    assert!(session
        .turns
        .iter()
        .all(|item| item.turn_id != turn.turn_id));
}

#[test]
fn pending_attachment_removal_is_session_scoped_and_deletes_the_stored_file() {
    let state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_remove_attachment_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("upload_1_long-report-name.md");
    std::fs::write(&path, "test attachment").unwrap();
    let attachment = WebAttachment {
        id: "upload_1".to_string(),
        name: "long-report-name.md".to_string(),
        path: path.display().to_string(),
        bytes: 15,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .attachments
        .push(attachment);

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::AttachmentRemove {
            session_id: "session_a".to_string(),
            attachment_id: "upload_1".to_string(),
        },
    )
    .unwrap();

    assert!(matches!(
        event,
        Some(WireEvent::AttachmentRemoved {
            session_id,
            attachment_id,
        }) if session_id == "session_a" && attachment_id == "upload_1"
    ));
    assert!(state.sessions.lock().unwrap()["session_a"]
        .attachments
        .is_empty());
    assert!(!path.exists());
    assert_eq!(
        handle_command(
            &state,
            TEST_PORT,
            ClientCommand::AttachmentRemove {
                session_id: "session_b".to_string(),
                attachment_id: "upload_1".to_string(),
            },
        )
        .unwrap_err(),
        "pending_attachment_not_found"
    );
    let _ = std::fs::remove_dir(&root);
}

#[test]
fn duplicate_pending_attachment_removal_is_idempotent_for_the_same_session() {
    let state = routing_test_state();
    let root = std::env::temp_dir().join(format!(
        "timem_web_duplicate_remove_attachment_{}",
        now_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("upload_1_notes.md");
    std::fs::write(&path, "test attachment").unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .attachments
        .push(WebAttachment {
            id: "upload_1".to_string(),
            name: "notes.md".to_string(),
            path: path.display().to_string(),
            bytes: 15,
        });

    for _ in 0..5 {
        let event = handle_command(
            &state,
            TEST_PORT,
            ClientCommand::AttachmentRemove {
                session_id: "session_a".to_string(),
                attachment_id: "upload_1".to_string(),
            },
        )
        .unwrap();
        assert!(matches!(
            event,
            Some(WireEvent::AttachmentRemoved {
                session_id,
                attachment_id,
            }) if session_id == "session_a" && attachment_id == "upload_1"
        ));
    }
    assert!(state.sessions.lock().unwrap()["session_a"]
        .attachments
        .is_empty());
    assert!(!path.exists());
    let _ = std::fs::remove_dir(&root);
}

#[test]
fn failed_pending_attachment_file_removal_restores_the_session_entry() {
    let state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_restore_attachment_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let attachment = WebAttachment {
        id: "upload_restore".to_string(),
        name: "restore.md".to_string(),
        path: root.display().to_string(),
        bytes: 1,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .attachments
        .push(attachment.clone());

    assert_eq!(
        remove_pending_attachment(&state, "session_a", "upload_restore").unwrap_err(),
        "attachment_remove_failed"
    );
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"].attachments,
        vec![attachment]
    );
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn browser_commands_are_strictly_tagged_and_do_not_accept_unknown_variants() {
    let command = serde_json::from_str::<ClientCommand>(
        r#"{"type":"topic_reply","session_id":"session_1","topic_name":"core.request","request_id":"req_1","decision":"accept","payload":{}}"#,
    )
    .unwrap();
    assert!(matches!(command, ClientCommand::TopicReply { .. }));

    let rename = serde_json::from_str::<ClientCommand>(
        r#"{"type":"session_rename","session_id":"session_1","display_name":"Build agent"}"#,
    )
    .unwrap();
    assert!(matches!(rename, ClientCommand::SessionRename { .. }));

    let credential = serde_json::from_str::<ClientCommand>(
        r#"{"type":"session_api_key_update","session_id":"session_1","api_key":"secret"}"#,
    )
    .unwrap();
    assert!(matches!(
        credential,
        ClientCommand::SessionApiKeyUpdate { .. }
    ));
    let reveal = serde_json::from_str::<ClientCommand>(
        r#"{"type":"session_api_key_reveal","session_id":"session_1"}"#,
    )
    .unwrap();
    assert!(matches!(reveal, ClientCommand::SessionApiKeyReveal { .. }));

    let attachment_remove = serde_json::from_str::<ClientCommand>(
        r#"{"type":"attachment_remove","session_id":"session_1","attachment_id":"upload_1"}"#,
    )
    .unwrap();
    assert!(matches!(
        attachment_remove,
        ClientCommand::AttachmentRemove { .. }
    ));

    let mem_switch =
        serde_json::from_str::<ClientCommand>(r#"{"type":"mem_switch","path":"/tmp/.test_mem"}"#)
            .unwrap();
    assert!(matches!(
        mem_switch,
        ClientCommand::MemSwitch { ref path, stop_running: false } if path == "/tmp/.test_mem"
    ));
    let confirmed_mem_switch = serde_json::from_str::<ClientCommand>(
        r#"{"type":"mem_switch","path":"/tmp/.test_mem","stop_running":true}"#,
    )
    .unwrap();
    assert!(matches!(
        confirmed_mem_switch,
        ClientCommand::MemSwitch { ref path, stop_running: true } if path == "/tmp/.test_mem"
    ));
    let legacy_mem_switch =
        serde_json::from_str::<ClientCommand>(r#"{"type":"mem_switch","space":".test_mem"}"#)
            .unwrap();
    assert!(matches!(
        legacy_mem_switch,
        ClientCommand::MemSwitch { ref path, stop_running: false } if path == ".test_mem"
    ));

    assert!(serde_json::from_str::<ClientCommand>(r#"{"type":"shell_exec"}"#).is_err());
}

#[test]
fn always_allow_topic_reply_promotes_session_bash_approval() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "ALWAYS_ALLOW_DONE");
    let worker_id = state.sessions.lock().unwrap()[&session_id]
        .primary_worker_id
        .clone();
    start_web_turn(&state, &session_id, "needs bash approval").unwrap();

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TopicReply {
            session_id: session_id.clone(),
            worker_id: Some(worker_id),
            topic_name: CORE_TOPIC_USER_APPROVAL_REQUEST.to_string(),
            request_id: Some("approval_1".to_string()),
            decision: "always_allow".to_string(),
            payload: json!({ "summary": "always allow bash for this session" }),
        },
    )
    .unwrap();

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(
        session.runtime.settings.bash_approval_mode,
        BashApprovalMode::Approve
    );
    assert_eq!(session.runtime_profile.bash_approval, "approve");
}

#[test]
fn browser_open_uses_a_direct_argument_without_shell_interpolation() {
    let url = "http://127.0.0.1:12345/?token=test&name=a b";
    let (_program, args) = browser_command(url).expect("supported browser command");
    assert_eq!(args.last().and_then(|arg| arg.to_str()), Some(url));
}

#[test]
fn browser_auto_open_requires_a_local_graphical_session() {
    assert!(!browser_auto_open_allowed_for(true, true));
    assert!(!browser_auto_open_allowed_for(false, false));
    assert!(browser_auto_open_allowed_for(false, true));
}

#[test]
fn web_runtime_updates_only_accept_the_shared_runtime_config_keys() {
    assert!(matches!(
        runtime_config_field_from_key("TIMEM_MAX_LLM_OUTPUT"),
        Ok(agent_core::RuntimeConfigField::MaxOutput)
    ));
    assert_eq!(
        runtime_config_field_from_key("TIMEM_API_KEY").unwrap_err(),
        "unsupported_runtime_config_key"
    );
}

#[test]
fn session_model_service_config_allows_send_without_an_api_key() {
    let state = routing_test_state();
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions
            .get_mut("session_a")
            .unwrap()
            .runtime
            .settings
            .config
            .api_key
            .clear();
    }

    assert!(validate_session_model_service_config(&state, "session_a").is_ok());
}

#[test]
fn web_draft_model_service_config_allows_startup_without_an_api_key() {
    let config =
        model_service_config_for_web_launch(&WebLaunchOptions::default(), &HashMap::new()).unwrap();
    assert!(config.api_key.is_empty());
    assert_eq!(config.model, "qwen-plus");
    assert_eq!(
        config.interaction.tool_call_mode,
        agent_core::ToolCallMode::Auto
    );
}

#[test]
fn runtime_update_propagates_to_existing_sessions_and_new_session_defaults() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_runtime_update_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");
    let existing_session_id = state
        .sessions
        .lock()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let mut events = state.events.subscribe();

    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::RuntimeUpdate {
            key: "TIMEM_MODEL".to_string(),
            value: "future-session-model".to_string(),
        },
    )
    .unwrap()
    .is_none());

    let WireEvent::HostConfigUpdated {
        key,
        value,
        session_env_defaults,
    } = drain_wire_events(&mut events)
        .into_iter()
        .find(|event| matches!(event, WireEvent::HostConfigUpdated { .. }))
        .expect("runtime update must publish refreshed session defaults")
    else {
        unreachable!()
    };
    assert_eq!(key, "TIMEM_MODEL");
    assert_eq!(value, "future-session-model");
    assert_eq!(
        session_env_defaults.get("TIMEM_MODEL").map(String::as_str),
        Some("future-session-model")
    );
    assert!(!session_env_defaults.contains_key("TIMEM_API_KEY"));
    // After propagation, existing sessions should be updated too
    assert_eq!(
        state.sessions.lock().unwrap()[&existing_session_id]
            .runtime_profile
            .model,
        "future-session-model"
    );
    let cached = current_session_store(&state)
        .unwrap()
        .load_session(&existing_session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        cached.env.get("TIMEM_MODEL").map(String::as_str),
        Some("future-session-model")
    );

    let created = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionCreate {
            display_name: Some("Future defaults".to_string()),
            workspace_dir: None,
            env: BTreeMap::new(),
        },
    )
    .unwrap();
    let Some(WireEvent::SessionCreated { session }) = created else {
        panic!("runtime-updated defaults must be visible on the next created session")
    };
    assert_eq!(session.runtime_profile.model, "future-session-model");
}

#[test]
fn session_runtime_update_is_scoped_and_persisted_without_changing_host_defaults() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_scoped_runtime_update_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");
    let untouched_session_id = state
        .sessions
        .lock()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let target_session_id = create_session(
        &state,
        Some("Scoped runtime".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let host_model = state.template.settings.lock().unwrap().config.model.clone();
    let untouched_model = state.sessions.lock().unwrap()[&untouched_session_id]
        .runtime_profile
        .model
        .clone();
    let mut events = state.events.subscribe();

    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionRuntimeUpdate {
            session_id: target_session_id.clone(),
            key: "TIMEM_MODEL".to_string(),
            value: "session-only-model".to_string(),
        },
    )
    .unwrap()
    .is_none());

    let event = drain_wire_events(&mut events)
        .into_iter()
        .find(|event| matches!(event, WireEvent::SessionRuntimeConfigUpdated { .. }))
        .expect("scoped update should publish an event");
    assert!(matches!(
        event,
        WireEvent::SessionRuntimeConfigUpdated {
            ref session_id,
            ref key,
            ref value,
            ref runtime_profile,
        } if session_id == &target_session_id
            && key == "TIMEM_MODEL"
            && value == "session-only-model"
            && runtime_profile.model == "session-only-model"
    ));
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionRuntimeUpdate {
            session_id: target_session_id.clone(),
            key: "TIMEM_MAX_ROUNDS".to_string(),
            value: "200".to_string(),
        },
    )
    .unwrap()
    .is_none());
    let max_rounds_event = drain_wire_events(&mut events)
        .into_iter()
        .find(|event| {
            matches!(
                event,
                WireEvent::SessionRuntimeConfigUpdated { key, .. }
                    if key == "TIMEM_MAX_ROUNDS"
            )
        })
        .expect("max steps update should publish an event");
    assert!(matches!(
        max_rounds_event,
        WireEvent::SessionRuntimeConfigUpdated {
            ref session_id,
            ref value,
            ref runtime_profile,
            ..
        } if session_id == &target_session_id
            && value == "200"
            && runtime_profile.max_rounds == "200"
    ));
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(
        sessions[&target_session_id].runtime_profile.model,
        "session-only-model"
    );
    assert_eq!(
        sessions[&target_session_id].runtime_profile.max_rounds,
        "200"
    );
    assert_eq!(
        sessions[&untouched_session_id].runtime_profile.model,
        untouched_model
    );
    drop(sessions);
    assert_eq!(
        state.template.settings.lock().unwrap().config.model,
        host_model
    );
    let stored = current_session_store(&state)
        .unwrap()
        .load_session(&target_session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.env.get("TIMEM_MODEL").map(String::as_str),
        Some("session-only-model")
    );
    assert_eq!(
        stored.env.get("TIMEM_MAX_ROUNDS").map(String::as_str),
        Some("200")
    );

    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::take(&mut *guard)
    };
    manager.shutdown_all().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn web_startup_can_bootstrap_model_service_config_from_latest_session_cache() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_cached_bootstrap"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "cached_bootstrap_mem";
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = data_dir.join(space).display().to_string();
    state.template = Arc::new(template);
    set_test_mem(
        &state,
        data_dir.clone(),
        &data_dir.join(space).display().to_string(),
    );
    state.sessions.lock().unwrap().clear();

    create_session(
        &state,
        Some("Cached runtime".to_string()),
        Some(root.display().to_string()),
        BTreeMap::from([
            (
                "TIMEM_MODEL".to_string(),
                "cached-session-model".to_string(),
            ),
            (
                "TIMEM_API_KEY".to_string(),
                "cached-session-secret".to_string(),
            ),
        ]),
    )
    .unwrap();
    let store = current_session_store(&state).unwrap();
    let mut older = store.list_sessions().unwrap().remove(0);
    older.session_id = "older_cached_session".to_string();
    older.updated_at_ms = 1;
    older
        .env
        .insert("TIMEM_MODEL".to_string(), "older-model".to_string());
    older
        .env
        .insert("TIMEM_API_KEY".to_string(), "older-secret".to_string());
    store.upsert_session(&older).unwrap();

    let launch = WebLaunchOptions {
        space: Some(data_dir.join(space).display().to_string()),
        ..WebLaunchOptions::default()
    };
    let restored_template = WorkerTemplate::from_environment(&launch).unwrap();
    let settings = restored_template.settings.lock().unwrap();
    assert_eq!(settings.config.model, "cached-session-model");
    assert_eq!(settings.config.api_key, "cached-session-secret");
    assert_eq!(
        settings.config.interaction.tool_call_mode,
        agent_core::ToolCallMode::Auto
    );
}

#[test]
fn restored_interaction_cache_keeps_only_explicit_session_overrides() {
    let effective_env = BTreeMap::from([
        ("TIMEM_TOOL_CALL_MODE".to_string(), "inline".to_string()),
        ("TIMEM_PARALLEL_TOOL_CALLS".to_string(), "false".to_string()),
        ("TIMEM_MODEL".to_string(), "cached-model".to_string()),
    ]);
    let implicit = sanitize_restored_session_env(effective_env.clone(), &BTreeMap::new());
    assert!(!implicit.contains_key("TIMEM_TOOL_CALL_MODE"));
    assert!(!implicit.contains_key("TIMEM_PARALLEL_TOOL_CALLS"));
    assert_eq!(
        implicit.get("TIMEM_MODEL").map(String::as_str),
        Some("cached-model")
    );

    let explicit_overrides =
        BTreeMap::from([("TIMEM_TOOL_CALL_MODE".to_string(), "inline".to_string())]);
    let explicit = sanitize_restored_session_env(effective_env, &explicit_overrides);
    assert_eq!(
        explicit.get("TIMEM_TOOL_CALL_MODE").map(String::as_str),
        Some("inline")
    );
    assert!(!explicit.contains_key("TIMEM_PARALLEL_TOOL_CALLS"));
}

#[test]
fn generated_public_access_token_is_sixteen_hex_characters_and_rotates() {
    let tokens = (0..32)
        .map(|_| generate_token().unwrap())
        .collect::<std::collections::HashSet<_>>();

    assert_eq!(tokens.len(), 32);
    assert!(tokens.iter().all(|token| token.len() == 16));
    assert!(tokens
        .iter()
        .all(|token| token.chars().all(|character| character.is_ascii_hexdigit())));
}

#[test]
fn rejects_empty_turns_and_supplements_before_they_reach_core() {
    assert!(nonempty_text(" \n\t ".to_string(), "turn text").is_err());
    assert_eq!(
        nonempty_text("  retain text  ".to_string(), "supplement").unwrap(),
        "retain text"
    );
}

#[test]
fn embedded_frontend_assets_receive_browser_safe_content_types() {
    assert_eq!(mime_for_path("/index.html"), "text/html; charset=utf-8");
    assert_eq!(
        mime_for_path("/assets/index.js"),
        "application/javascript; charset=utf-8"
    );
    assert_eq!(mime_for_path("/timem_logo.png"), "image/png");
    assert!(embedded_web_asset("/timem_logo.png").is_some());
}

#[test]
fn embedded_frontend_toolrepo_browser_does_not_render_readme_body() {
    let index = std::str::from_utf8(embedded_web_asset("/index.html").unwrap()).unwrap();
    let js_path = index
        .split('"')
        .find(|part| part.starts_with("/assets/index-") && part.ends_with(".js"))
        .expect("embedded frontend index js asset");
    let css_path = index
        .split('"')
        .find(|part| part.starts_with("/assets/index-") && part.ends_with(".css"))
        .expect("embedded frontend index css asset");
    let js = std::str::from_utf8(embedded_web_asset(js_path).unwrap()).unwrap();
    let css = std::str::from_utf8(embedded_web_asset(css_path).unwrap()).unwrap();

    assert!(js.contains("Tool directory tree"));
    assert!(js.contains("Collapse tool detail"));
    assert!(!js.contains("toolrepo-readme"));
    assert!(!js.contains(".readme})"));
    assert!(css.contains(".toolrepo-item.selected .toolrepo-item-main>svg"));
    assert!(!css.contains(".toolrepo-readme"));
}

#[test]
fn browser_responses_disable_referrer_leaks_and_remote_active_content() {
    let mut response = Response::new(axum::body::Body::empty());
    apply_browser_security_headers(&mut response);
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let policy = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(policy.contains("img-src 'self' data:"));
    assert!(policy.contains("font-src 'self' data:"));
    assert!(policy.contains("script-src 'self'"));
    assert!(!policy.contains("script-src 'self' 'unsafe-inline'"));
    assert!(policy.contains("form-action 'none'"));
    assert!(policy.contains("object-src 'none'"));
    assert!(policy.contains("frame-ancestors 'none'"));
}

#[test]
fn workspace_snapshot_deduplicates_registered_current_directory() {
    let template = WorkerTemplate {
        settings: Arc::new(Mutex::new(RuntimeSettings {
            config: ModelServiceConfig {
                interaction: Default::default(),
                model: "test-model".to_string(),
                base_url: "http://127.0.0.1".to_string(),
                api_key: "test".to_string(),
                http_headers: Default::default(),
                request_fields: Default::default(),
                timeout_secs: 1,
                max_llm_output_tokens: 1_024,
                max_llm_input_tokens: 10_000,
                api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
                response_protocol: ResponseProtocolKind::default(),
                openai_compatible: agent_core::OpenAiCompatibleOptions::default(),
            },
            bash_approval_mode: BashApprovalMode::Ask,
            work_instruction_mode: WorkInstructionLoadMode::Off,
            max_rounds: agent_core::UNLIMITED_ROUND_BUDGET,
        })),
        data_dir: PathBuf::from("/tmp/data"),
        initial_space: ".test_mem".to_string(),
        env: BTreeMap::new(),
        current_dir: PathBuf::from("/work/a"),
        workspace_dirs: vec![PathBuf::from("/work/a"), PathBuf::from("/work/b")],
        reminder_tips_config: agent_core::ReminderTipsConfig::default(),
    };
    assert_eq!(web_workspace_dirs(&template), vec!["/work/a", "/work/b"]);
}

#[test]
fn upload_names_cannot_escape_the_session_upload_directory() {
    assert_eq!(
        sanitize_upload_name("../../report.txt").unwrap(),
        "report.txt"
    );
    assert_eq!(
        sanitize_upload_name("review notes?.md").unwrap(),
        "review_notes_.md"
    );
    assert!(sanitize_upload_name("..").is_err());
    assert_eq!(sanitize_upload_name(&"a".repeat(300)).unwrap().len(), 160);
}

#[test]
fn session_create_returns_the_complete_session_to_the_requesting_browser() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_create_session_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionCreate {
            display_name: Some("Review".to_string()),
            workspace_dir: Some(root.display().to_string()),
            env: BTreeMap::new(),
        },
    )
    .unwrap()
    .expect("session creation must return a direct browser event");

    let WireEvent::SessionCreated { session } = event else {
        panic!("unexpected session creation response")
    };
    assert_eq!(session.display_name, "Review");
    assert_eq!(
        PathBuf::from(&session.current_dir).canonicalize().unwrap(),
        root.canonicalize().unwrap()
    );
    assert!(state
        .sessions
        .lock()
        .unwrap()
        .contains_key(&session.session_id));
}

#[test]
fn unnamed_sessions_use_the_next_available_untitled_number() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_untitled_sessions"));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), "untitled_sessions_mem");
    state.sessions.lock().unwrap().clear();

    let reserved = create_session(
        &state,
        Some("Untitled 2".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let first = create_session(
        &state,
        None,
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let third = create_session(
        &state,
        Some("   ".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[&reserved].display_name, "Untitled 2");
    assert_eq!(sessions[&first].display_name, "Untitled 1");
    assert_eq!(sessions[&third].display_name, "Untitled 3");
    drop(sessions);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn new_session_does_not_inherit_mem_enabled_mcp_servers() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_new_session_mcp"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    state.template = Arc::new(template);
    set_test_mem(&state, data_dir, "new_session_mcp_mem");
    state.sessions.lock().unwrap().clear();
    {
        let mut mem = state.mem.lock().unwrap();
        mem.mcp_configs.push(McpServerConfig {
            id: "available".to_string(),
            name: "Available MCP".to_string(),
            enabled: true,
            transport: agent_core::mcp::McpTransportConfig::Stdio {
                command: "/bin/false".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
            request_timeout_ms: 10,
        });
        mem.mcp_store.save(&mem.mcp_configs).unwrap();
    }

    let session_id = create_session(
        &state,
        Some("No inherited MCP".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert!(session.mcp_server_ids.is_empty());
    assert_eq!(session.mcp_config_revision, 0);
    assert_eq!(session.applied_mcp_config_revision, 0);
    drop(sessions);

    let stored = current_session_store(&state)
        .unwrap()
        .load_session(&session_id)
        .unwrap()
        .unwrap();
    assert!(stored.mcp_server_ids.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_create_accepts_an_unregistered_absolute_workspace_directory() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_select_workspace_{}", now_ms()));
    let registered = root.join("registered");
    let selected = root.join("selected elsewhere");
    std::fs::create_dir_all(&registered).unwrap();
    std::fs::create_dir_all(&selected).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = registered.clone();
    template.workspace_dirs = vec![registered];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionCreate {
            display_name: Some("Selected workspace".to_string()),
            workspace_dir: Some(selected.display().to_string()),
            env: BTreeMap::new(),
        },
    )
    .unwrap()
    .expect("session creation must succeed for an explicitly selected directory");

    let WireEvent::SessionCreated { session } = event else {
        panic!("expected SessionCreated")
    };
    assert_eq!(
        PathBuf::from(session.current_dir).canonicalize().unwrap(),
        selected.canonicalize().unwrap()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_workspace_selection_rejects_relative_missing_and_file_paths() {
    let root = std::env::temp_dir().join(format!("timem_web_invalid_workspace_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("not-a-directory.txt");
    std::fs::write(&file, "test").unwrap();
    let state = routing_test_state();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs.clear();

    assert_eq!(
        template
            .resolve_workspace(Some("relative/path"))
            .unwrap_err(),
        "workspace_must_be_absolute"
    );
    assert_eq!(
        template
            .resolve_workspace(Some(root.join("missing").to_str().unwrap()))
            .unwrap_err(),
        "workspace_not_found"
    );
    assert_eq!(
        template
            .resolve_workspace(Some(file.to_str().unwrap()))
            .unwrap_err(),
        "workspace_not_directory"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_delete_stops_workers_and_removes_persisted_session() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_session_delete_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

    let created = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionCreate {
            display_name: Some("Disposable".to_string()),
            workspace_dir: Some(root.display().to_string()),
            env: BTreeMap::new(),
        },
    )
    .unwrap();
    let Some(WireEvent::SessionCreated { session }) = created else {
        panic!("expected SessionCreated")
    };
    let store = current_session_store(&state).unwrap();
    let session_dir = store
        .history_path_for_session(&session.session_id)
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(store.load_session(&session.session_id).unwrap().is_some());
    store
        .append_history_record(
            &session.session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: "turn_delete".to_string(),
                created_at_ms: 1,
                kind: None,
                command_id: None,
                delivery_state: None,
                content: "delete this history".to_string(),
            },
        )
        .unwrap();
    assert!(session_dir.exists());

    let deleted = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionDelete {
            session_id: session.session_id.clone(),
        },
    )
    .unwrap();

    assert!(matches!(
        deleted,
        Some(WireEvent::SessionDeleted { session_id }) if session_id == session.session_id
    ));
    assert!(!state
        .sessions
        .lock()
        .unwrap()
        .contains_key(&session.session_id));
    assert!(store.load_session(&session.session_id).unwrap().is_none());
    assert!(!session_dir.exists());
    assert_eq!(state.manager.lock().unwrap().worker_count(), 0);
}

#[test]
fn assistant_speaker_name_is_fixed() {
    assert_eq!(assistant_speaker_name(), "ASSISTANT");
}

#[test]
fn unnamed_web_session_uses_untitled_name_while_worker_keeps_core_identity() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_default_session_name_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

    let session_id = create_session(
        &state,
        None,
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(session.display_name, "Untitled 1");
    assert_eq!(session.workers.len(), 1);
    assert_eq!(session.workers[0].display_name, "ID0");
    drop(sessions);

    let renamed = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionRename {
            session_id: session_id.clone(),
            display_name: "Build session".to_string(),
        },
    )
    .unwrap();
    assert!(matches!(
        renamed,
        Some(WireEvent::SessionRenamed {
            session_id: ref renamed_id,
            display_name: ref name,
        }) if renamed_id == &session_id && name == "Build session"
    ));
    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[&session_id].display_name, "Build session");
    assert_eq!(
        sessions[&session_id].workers[0].display_name, "ID0",
        "Session rename must not rewrite the Web worker display name"
    );
    drop(sessions);
}

#[test]
fn existing_session_api_key_can_be_updated_without_exposing_the_secret() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_session_api_key_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");
    let session_id = create_session(
        &state,
        Some("Credential test".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();

    let mut events = state.events.subscribe();
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionApiKeyUpdate {
            session_id: session_id.clone(),
            api_key: "new-session-secret".to_string(),
        },
    )
    .unwrap()
    .is_none());
    let event = drain_wire_events(&mut events)
        .into_iter()
        .find(|event| matches!(event, WireEvent::SessionRuntimeUpdated { .. }))
        .expect("credential update should publish a scoped event");
    assert!(matches!(
        event,
        WireEvent::SessionRuntimeUpdated {
            session_id: ref event_session_id,
            ref runtime_profile,
        } if event_session_id == &session_id && runtime_profile.api_key_configured
    ));
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!serialized.contains("new-session-secret"));
    assert!(!serialized.contains("TIMEM_API_KEY"));
    assert_eq!(
        state.sessions.lock().unwrap()[&session_id]
            .runtime
            .settings
            .config
            .api_key,
        "new-session-secret"
    );
    let reveal = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionApiKeyReveal {
            session_id: session_id.clone(),
        },
    )
    .unwrap()
    .expect("credential reveal should reply to its requesting socket");
    assert!(matches!(
        reveal,
        WireEvent::SessionApiKeyRevealed {
            session_id: ref event_session_id,
            ref api_key,
        } if event_session_id == &session_id && api_key == "new-session-secret"
    ));
    assert!(
        events.try_recv().is_err(),
        "credential reveal must not be broadcast"
    );
    let stored = current_session_store(&state)
        .unwrap()
        .load_session(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.env.get("TIMEM_API_KEY").map(String::as_str),
        Some("new-session-secret")
    );

    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionApiKeyUpdate {
            session_id: session_id.clone(),
            api_key: String::new(),
        },
    )
    .unwrap()
    .is_none());
    let cleared = drain_wire_events(&mut events)
        .into_iter()
        .find(|event| matches!(event, WireEvent::SessionRuntimeUpdated { .. }))
        .expect("credential clearing should publish a scoped event");
    assert!(matches!(
        cleared,
        WireEvent::SessionRuntimeUpdated {
            runtime_profile: WebSessionRuntimeProfile {
                api_key_configured: false,
                ..
            },
            ..
        }
    ));

    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::take(&mut *guard)
    };
    manager.shutdown_all().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_runtime_update_is_allowed_during_an_active_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "ACTIVE_RUNTIME_UPDATE");
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.active_turn_id = Some("turn_active".to_string());
        session.state = "working".to_string();
        session.turns.push(WebTurn {
            turn_id: "turn_active".to_string(),
            state: "working".to_string(),
            created_at_ms: now_ms(),
            user_entries: Vec::new(),
            events: Vec::new(),
            sub_answers: Vec::new(),
            final_answer: None,
            completion: None,
        });
    }

    let mut events = state.events.subscribe();
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionRuntimeUpdate {
            session_id: session_id.clone(),
            key: "TIMEM_MODEL".to_string(),
            value: "active-turn-model".to_string(),
        },
    )
    .unwrap()
    .is_none());

    let event = drain_wire_events(&mut events)
        .into_iter()
        .find(|event| matches!(event, WireEvent::SessionRuntimeConfigUpdated { .. }))
        .expect("active-turn runtime update should publish a scoped event");
    assert!(matches!(
        event,
        WireEvent::SessionRuntimeConfigUpdated {
            session_id: ref event_session_id,
            ref key,
            ref value,
            ref runtime_profile,
        } if event_session_id == &session_id
            && key == "TIMEM_MODEL"
            && value == "active-turn-model"
            && runtime_profile.model == "active-turn-model"
    ));

    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert_eq!(session.state, "working");
        assert_eq!(session.active_turn_id.as_deref(), Some("turn_active"));
        assert_eq!(session.runtime_profile.model, "active-turn-model");
    }

    let manager = {
        let mut manager = state.manager.lock().unwrap();
        std::mem::take(&mut *manager)
    };
    manager.shutdown_all().unwrap();
}

#[test]
fn session_api_key_update_is_rejected_during_an_active_turn() {
    let state = routing_test_state();
    let mut sessions = state.sessions.lock().unwrap();
    let session = sessions.get_mut("session_a").unwrap();
    session.active_turn_id = Some("turn_active".to_string());
    session.turns.push(WebTurn {
        turn_id: "turn_active".to_string(),
        state: "working".to_string(),
        created_at_ms: now_ms(),
        user_entries: Vec::new(),
        events: Vec::new(),
        sub_answers: Vec::new(),
        final_answer: None,
        completion: None,
    });
    drop(sessions);
    let error = update_session_api_key(&state, "session_a", "new-secret".to_string()).unwrap_err();
    assert_eq!(error, "session_api_key_update_while_working");
}

#[test]
fn session_api_key_update_rejects_invalid_or_oversized_values_before_dispatch() {
    let state = routing_test_state();
    assert!(
        update_session_api_key(&state, "session_a", "bad key".to_string())
            .unwrap_err()
            .starts_with("invalid_session_api_key:")
    );
    assert_eq!(
        update_session_api_key(&state, "session_a", "x".repeat(8 * 1024 + 1)).unwrap_err(),
        "session_api_key_too_large"
    );
}

#[test]
fn session_creation_applies_independent_runtime_env_without_mutating_defaults() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_session_env_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

    let first_env = BTreeMap::from([
        ("TIMEM_MODEL".to_string(), "claude-session-a".to_string()),
        ("TIMEM_API_PROTOCOL".to_string(), "anthropic".to_string()),
        ("TIMEM_RESPONSE_PROTOCOL".to_string(), "json".to_string()),
        ("TIMEM_API_KEY".to_string(), "session-a-secret".to_string()),
        ("TIMEM_MAX_LLM_INPUT".to_string(), "128K".to_string()),
    ]);
    let second_env = BTreeMap::from([
        ("TIMEM_MODEL".to_string(), "qwen-session-b".to_string()),
        ("TIMEM_RESPONSE_PROTOCOL".to_string(), "xml".to_string()),
        ("TIMEM_MAX_LLM_INPUT".to_string(), "64K".to_string()),
        ("TIMEM_ENABLE_THINKING".to_string(), "true".to_string()),
        ("TIMEM_REASONING_EFFORT".to_string(), "max".to_string()),
        ("TIMEM_STREAM".to_string(), "true".to_string()),
    ]);
    let first = create_session(
        &state,
        Some("Session A".to_string()),
        Some(root.display().to_string()),
        first_env,
    )
    .unwrap();
    let second = create_session(
        &state,
        Some("Session B".to_string()),
        Some(root.display().to_string()),
        second_env,
    )
    .unwrap();

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[&first].runtime_profile.model, "claude-session-a");
    assert_eq!(sessions[&first].runtime_profile.api_protocol, "anthropic");
    assert_eq!(sessions[&first].runtime_profile.response_protocol, "json");
    assert_eq!(sessions[&first].max_llm_input_tokens, 128_000);
    assert_eq!(sessions[&second].runtime_profile.model, "qwen-session-b");
    assert_eq!(sessions[&second].runtime_profile.response_protocol, "xml");
    assert_eq!(sessions[&second].max_llm_input_tokens, 64_000);
    assert_eq!(
        sessions[&second]
            .runtime
            .settings
            .config
            .openai_compatible
            .enable_thinking,
        Some(true)
    );
    assert_eq!(
        sessions[&second]
            .runtime
            .settings
            .config
            .openai_compatible
            .reasoning_effort
            .as_deref(),
        Some("max")
    );
    assert!(
        sessions[&second]
            .runtime
            .settings
            .config
            .openai_compatible
            .stream
    );
    drop(sessions);

    let defaults = state.template.settings.lock().unwrap();
    assert_eq!(defaults.config.model, "test-model");
    assert_eq!(defaults.config.max_llm_input_tokens, 10_000);
    drop(defaults);

    let serialized = serde_json::to_string(&snapshot_for(&state, 12345)).unwrap();
    assert!(!serialized.contains("session-a-secret"));
    assert!(!serialized.contains("TIMEM_API_KEY"));
    let mut lifecycle_profiles = BTreeMap::new();
    let mut lifecycle_leaked_secret = false;
    for _ in 0..100 {
        for (session_id, _context_id, _worker_id, event) in drain_worker_events(&state) {
            if let CoreSessionWorkerEvent::Topics(topics) = event {
                lifecycle_leaked_secret |= topics
                    .iter()
                    .map(CoreTopicEvent::wire_payload)
                    .any(|topic| topic.to_string().contains("session-a-secret"));
                if let Some(lifecycle) = topics.first().and_then(CoreTopicEvent::as_lifecycle) {
                    lifecycle_profiles.insert(
                        session_id,
                        (
                            lifecycle.profile.model,
                            lifecycle.response_protocol,
                            lifecycle.max_llm_input_tokens,
                        ),
                    );
                }
            }
        }
        if lifecycle_profiles.contains_key(&first) && lifecycle_profiles.contains_key(&second) {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        lifecycle_profiles.get(&first),
        Some(&("claude-session-a".to_string(), "json".to_string(), 128_000,))
    );
    assert_eq!(
        lifecycle_profiles.get(&second),
        Some(&("qwen-session-b".to_string(), "xml".to_string(), 64_000,))
    );
    assert!(!lifecycle_leaked_secret);
}

#[test]
fn session_create_command_returns_session_with_runtime_overrides_applied() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_session_cmd_env_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionCreate {
            display_name: Some("Override session".to_string()),
            workspace_dir: Some(root.display().to_string()),
            env: BTreeMap::from([
                ("TIMEM_MODEL".to_string(), "model-from-dialog".to_string()),
                ("TIMEM_RESPONSE_PROTOCOL".to_string(), "xml".to_string()),
                ("TIMEM_MAX_LLM_INPUT".to_string(), "42K".to_string()),
                ("TIMEM_BASH_APPROVAL".to_string(), "approve".to_string()),
            ]),
        },
    )
    .unwrap();

    let session = match event {
        Some(WireEvent::SessionCreated { session }) => session,
        other => panic!("expected SessionCreated, got {other:?}"),
    };
    assert_eq!(session.display_name, "Override session");
    assert_eq!(session.runtime_profile.model, "model-from-dialog");
    assert_eq!(session.runtime_profile.response_protocol, "xml");
    assert_eq!(session.runtime_profile.max_llm_input_tokens, 42_000);
    assert_eq!(session.runtime_profile.bash_approval, "approve");

    let sessions = state.sessions.lock().unwrap();
    let stored = sessions.get(&session.session_id).unwrap();
    assert_eq!(stored.runtime.settings.config.model, "model-from-dialog");
    assert_eq!(
        stored.runtime.settings.config.response_protocol.name(),
        "xml"
    );
    assert_eq!(stored.runtime.settings.config.max_llm_input_tokens, 42_000);
    assert_eq!(
        stored.runtime.env.get("TIMEM_MODEL").map(String::as_str),
        Some("model-from-dialog")
    );
}

#[test]
fn missing_workspace_session_is_detached_once_without_deleting_history() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("missing_workspace_restore"));
    let data_dir = root.join("data");
    let space = "missing_workspace_mem";
    let missing_workspace = root.join("removed-workspace");
    let template_workspace = root.join("fallback-workspace");
    std::fs::create_dir_all(&template_workspace).unwrap();

    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = template_workspace.clone();
    template.workspace_dirs = vec![template_workspace];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template);
    state.sessions.lock().unwrap().clear();

    let store = current_session_store(&state).unwrap();
    let session_id = "session_missing_workspace";
    let history_path = store.history_path_for_session(session_id);
    store
        .append_history_record(
            session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: "turn_before_workspace_removal".to_string(),
                created_at_ms: 1,
                kind: Some("task".to_string()),
                command_id: None,
                delivery_state: None,
                content: "preserve this history".to_string(),
            },
        )
        .unwrap();
    store
        .upsert_session(&StoredSession {
            session_id: session_id.to_string(),
            display_name: "Missing workspace".to_string(),
            created_at_ms: 1,
            updated_at_ms: 2,
            current_dir: missing_workspace.display().to_string(),
            profile: StoredSessionProfile::default(),
            env: BTreeMap::new(),
            env_overrides: None,
            mcp_server_ids: Vec::new(),
            state: StoredSessionState::Ready,
            last_turn_id: Some("turn_before_workspace_removal".to_string()),
            raw_chat_history_path: history_path.display().to_string(),
            group_id: None,
        })
        .unwrap();

    assert_eq!(restore_stored_sessions(&state).unwrap(), 0);
    assert!(store.load_session(session_id).unwrap().is_none());
    assert!(history_path.exists());
    assert!(std::fs::read_to_string(&history_path)
        .unwrap()
        .contains("preserve this history"));

    // The repair is persistent: a later startup no longer retries this record.
    assert_eq!(restore_stored_sessions(&state).unwrap(), 0);
    assert!(history_path.exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_session_roles_migrate_into_the_mem_scoped_global_library() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_role_migration"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "role_migration_mem";

    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    set_test_mem(&state, data_dir.clone(), space);
    state.sessions.lock().unwrap().clear();

    let first_session = create_session(
        &state,
        Some("First legacy session".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let second_session = create_session(
        &state,
        Some("Second legacy session".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();

    let store = current_session_store(&state).unwrap();
    crate::worker_roles::save_roles(
        &roles_path_for_history(&store.history_path_for_session(&first_session)).unwrap(),
        &[
            WorkerRole {
                id: "legacy_shared_id".to_string(),
                name: "Reviewer".to_string(),
                description: "Review evidence before editing.".to_string(),
            },
            WorkerRole {
                id: "legacy_duplicate_name".to_string(),
                name: "Builder".to_string(),
                description: "Old duplicate that should not replace the first Builder.".to_string(),
            },
        ],
    )
    .unwrap();
    crate::worker_roles::save_roles(
        &roles_path_for_history(&store.history_path_for_session(&second_session)).unwrap(),
        &[
            WorkerRole {
                id: "legacy_shared_id".to_string(),
                name: "Builder".to_string(),
                description: "Implement and verify the requested change.".to_string(),
            },
            WorkerRole {
                id: "legacy_tester".to_string(),
                name: "Tester".to_string(),
                description: "Run focused and regression tests.".to_string(),
            },
        ],
    )
    .unwrap();

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);

    assert_eq!(restore_stored_sessions(&restarted).unwrap(), 2);

    let mem = current_mem_state(&restarted).unwrap();
    assert_eq!(mem.role_library.roles.len(), 3);
    assert_eq!(
        mem.role_library
            .roles
            .iter()
            .map(|role| role.name.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["Builder", "Reviewer", "Tester"])
    );
    assert_eq!(
        mem.role_library
            .roles
            .iter()
            .map(|role| role.id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        3,
        "legacy ID collisions must be reassigned instead of dropping a distinct role"
    );

    let persisted = load_role_library(&role_library_path(&mem.layout.memory_dir())).unwrap();
    assert_eq!(persisted, mem.role_library);

    let sessions = restarted.sessions.lock().unwrap();
    assert_eq!(sessions[&first_session].roles, persisted.roles);
    assert_eq!(sessions[&second_session].roles, persisted.roles);
    drop(sessions);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stored_session_restores_after_web_host_restart_with_fresh_worker() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_restore_session"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "restore_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();

    let session_id = create_session(
        &state,
        Some("Recovered work".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let turn = start_web_turn(&state, &session_id, "remember this after restart").unwrap();
    assert_eq!(turn.user_entries[0].text, "remember this after restart");

    // Simulate a session persisted by the previous Web host. The effective
    // Session environment remains authoritative even without separate override
    // provenance.
    let store = current_session_store(&state).unwrap();
    let mut legacy = store.load_session(&session_id).unwrap().unwrap();
    legacy.env_overrides = None;
    legacy
        .env
        .insert("TIMEM_MODEL".to_string(), "stale-model".to_string());
    legacy.env.insert(
        "TIMEM_GATEWAY_PROVIDER".to_string(),
        "retired-provider".to_string(),
    );
    store.upsert_session(&legacy).unwrap();

    // Changing a later host default must not silently replace a restored
    // Session's cached runtime environment.
    template.settings.lock().unwrap().config.model = "model-from-new-env".to_string();
    let expected_migrated_key = template.settings.lock().unwrap().config.api_key.clone();

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);
    let restored = restore_stored_sessions(&restarted).unwrap();
    assert_eq!(restored, 1);

    let sessions = restarted.sessions.lock().unwrap();
    let restored_session = sessions.get(&session_id).unwrap();
    assert_eq!(restored_session.display_name, "Recovered work");
    assert_eq!(
        std::fs::canonicalize(&restored_session.current_dir).unwrap(),
        std::fs::canonicalize(&root).unwrap()
    );
    assert_eq!(restored_session.workers.len(), 1);
    assert_eq!(restored_session.contexts.len(), 1);
    assert_eq!(restored_session.messages.len(), 1);
    assert_eq!(
        restored_session.messages[0].text,
        "remember this after restart"
    );
    assert!(restored_session.active_turn_id.is_none());
    assert!(restored_session.resume_notice_pending);
    assert_eq!(restored_session.runtime_profile.model, "stale-model");
    drop(sessions);

    let migrated = current_session_store(&restarted)
        .unwrap()
        .load_session(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(migrated.updated_at_ms, legacy.updated_at_ms);
    assert_eq!(
        migrated.env.get("TIMEM_MODEL").map(String::as_str),
        Some("stale-model")
    );
    assert_eq!(
        migrated.env.get("TIMEM_API_KEY").map(String::as_str),
        Some(expected_migrated_key.as_str())
    );
    assert!(!migrated.env.contains_key("TIMEM_GATEWAY_PROVIDER"));

    let context = session_context(&restarted, &session_id, &[])
        .unwrap()
        .expect("restored session should inject resume context");
    assert!(context.contains(
        "Runtime just restarted. Previous chat history's runtime info/tasks are invalid/outdated unless user asks to retrieve them."
    ));
    assert!(!context.contains("Previous audit chat history's runtime info are valid."));
    assert!(!context.contains("This session was restored"));
    assert!(context.contains("raw_chat_history.jsonl"));
    assert!(context.contains("format: JSONL, one record per line."));
    let context_after_first_use = session_context(&restarted, &session_id, &[])
        .unwrap()
        .unwrap_or_default();
    assert!(!context_after_first_use.contains("Runtime just restarted."));
    assert!(!context_after_first_use.contains("This session was restored"));
}

#[test]
fn web_host_startup_restore_appends_a_visible_raw_chat_restart_marker() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_restart_marker"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "restart_marker_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();

    let session_id = create_session(
        &state,
        Some("Restart marker".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    start_web_turn(&state, &session_id, "before restart").unwrap();
    let history_path = current_session_store(&state)
        .unwrap()
        .history_path_for_session(&session_id);

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);

    assert_eq!(
        restore_stored_sessions_after_runtime_restart(&restarted).unwrap(),
        1
    );

    let records = read_all_history_records(&history_path).unwrap();
    let markers = records
        .iter()
        .filter(|record| {
            matches!(
                record,
                ChatHistoryRecord::Message {
                    role: ChatHistoryRole::System,
                    kind: Some(kind),
                    content,
                    ..
                } if kind == RUNTIME_RESTART_HISTORY_KIND
                    && content == RUNTIME_RESTART_HISTORY_CONTENT
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1, "startup must append one raw-chat marker");

    let sessions = restarted.sessions.lock().unwrap();
    let restored = &sessions[&session_id];
    assert_eq!(
        restored.turns.len(),
        1,
        "marker must not create an empty turn"
    );
    assert_eq!(
        restored
            .messages
            .iter()
            .filter(|message| {
                message.role == "system"
                    && message.kind.as_deref() == Some(RUNTIME_RESTART_HISTORY_KIND)
            })
            .count(),
        1,
        "marker must be present in the restored Session chat timeline"
    );
    assert_eq!(
        restored
            .messages
            .last()
            .map(|message| message.text.as_str()),
        Some(RUNTIME_RESTART_HISTORY_CONTENT)
    );
}

#[test]
fn restore_keeps_multiple_legacy_sessions_when_retired_provider_cache_is_present() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_restore_legacy_sessions"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "legacy_sessions_mem";
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    set_test_mem(&state, data_dir.clone(), space);
    state.sessions.lock().unwrap().clear();

    for name in ["ADstart", "self-dev"] {
        let session_id = create_session(
            &state,
            Some(name.to_string()),
            Some(root.display().to_string()),
            BTreeMap::new(),
        )
        .unwrap();
        let store = current_session_store(&state).unwrap();
        let mut stored = store.load_session(&session_id).unwrap().unwrap();
        stored.env.insert(
            "TIMEM_GATEWAY_PROVIDER".to_string(),
            "retired-provider".to_string(),
        );
        store.upsert_session(&stored).unwrap();
    }

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);

    assert_eq!(restore_stored_sessions(&restarted).unwrap(), 2);
    let sessions = restarted.sessions.lock().unwrap();
    let names = sessions
        .values()
        .map(|session| session.display_name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names, BTreeSet::from(["ADstart", "self-dev"]));
    drop(sessions);

    for stored in current_session_store(&restarted)
        .unwrap()
        .list_sessions()
        .unwrap()
    {
        assert!(!stored.env.contains_key("TIMEM_GATEWAY_PROVIDER"));
    }
}

#[test]
fn session_create_and_restore_defer_unavailable_mcp_discovery_until_send() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_deferred_mcp_restore"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let marker = root.join("mcp_process_started");
    let space = "deferred_mcp_restore_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();
    {
        let mut mem = state.mem.lock().unwrap();
        mem.mcp_configs.push(McpServerConfig {
            id: "blocking-on-connect".to_string(),
            name: "Blocking on connect".to_string(),
            enabled: true,
            transport: agent_core::mcp::McpTransportConfig::Stdio {
                command: "/bin/sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    format!("touch '{}'; sleep 30", marker.display()),
                ],
                env: BTreeMap::new(),
            },
            request_timeout_ms: 100,
        });
        mem.mcp_store.save(&mem.mcp_configs).unwrap();
    }

    let started = Instant::now();
    let session_id = create_session(
        &state,
        Some("Deferred MCP".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!marker.exists(), "worker creation must not connect to MCP");
    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert!(session.mcp_server_ids.is_empty());
        assert_eq!(
            session.mcp_config_revision,
            session.applied_mcp_config_revision
        );
    }

    enable_mcp_for_session(&state, &session_id, true, Some("blocking-on-connect")).unwrap();
    persist_web_session(&state, &session_id).unwrap();
    assert!(
        !marker.exists(),
        "explicit Session enablement must not connect to MCP immediately"
    );
    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert_eq!(session.mcp_server_ids, vec!["blocking-on-connect"]);
        assert_ne!(
            session.mcp_config_revision,
            session.applied_mcp_config_revision
        );
    }

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);
    let started = Instant::now();
    assert_eq!(restore_stored_sessions(&restarted).unwrap(), 1);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        !marker.exists(),
        "session restore must not connect to an external MCP server"
    );
    let sessions = restarted.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_ne!(
        session.mcp_config_revision,
        session.applied_mcp_config_revision
    );
    drop(sessions);

    let started = Instant::now();
    assert_eq!(
        schedule_selected_session_mcp_refreshes(&restarted).unwrap(),
        1
    );
    assert_eq!(
        schedule_selected_session_mcp_refreshes(&restarted).unwrap(),
        0,
        "an in-flight MCP discovery must be deduplicated"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "background MCP prewarm must return immediately"
    );

    let started = Instant::now();
    assert!(apply_pending_session_mcp(&restarted, &session_id).unwrap());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "turn preparation must not wait for MCP discovery"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let report = mcp_reports(&restarted.mem.lock().unwrap())
            .into_iter()
            .next()
            .unwrap();
        if report.state == "error" {
            assert!(marker.exists(), "background discovery was not attempted");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "background MCP failure was not reported"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn restored_web_session_keeps_original_task_with_supplement_in_an_oversized_turn() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_restore_long_turn"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "restore_long_turn_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();

    let session_id = create_session(
        &state,
        Some("Milestone work".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let store = current_session_store(&state).unwrap();
    let turn_id = "turn_vla_milestone";
    let history_base_ms = now_ms_i64().saturating_sub(1_000);
    store
        .append_history_record(
            &session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: turn_id.to_string(),
                created_at_ms: history_base_ms,
                kind: Some("task".to_string()),
                command_id: None,
                delivery_state: None,
                content: "generate the VLA parking milestones".to_string(),
            },
        )
        .unwrap();
    for index in 0..203 {
        store
            .append_history_record(
                &session_id,
                &ChatHistoryRecord::Event {
                    role: ChatHistoryRole::System,
                    turn_id: turn_id.to_string(),
                    created_at_ms: history_base_ms + i64::from(index) + 1,
                    kind: ChatHistoryEventKind::Action,
                    content: format!("action {index}"),
                    extra: BTreeMap::new(),
                },
            )
            .unwrap();
    }
    store
        .append_history_record(
            &session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: turn_id.to_string(),
                created_at_ms: history_base_ms + 204,
                kind: Some("supplement".to_string()),
                command_id: None,
                delivery_state: None,
                content: "还有一个 tar_log，下面是 clp 压缩的日志".to_string(),
            },
        )
        .unwrap();

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);
    assert_eq!(restore_stored_sessions(&restarted).unwrap(), 1);

    let sessions = restarted.sessions.lock().unwrap();
    let restored = &sessions[&session_id];
    assert_eq!(restored.turns.len(), 1);
    assert_eq!(
        restored.turns[0]
            .user_entries
            .iter()
            .map(|entry| (entry.kind.as_str(), entry.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("task", "generate the VLA parking milestones"),
            ("supplement", "还有一个 tar_log，下面是 clp 压缩的日志"),
        ]
    );
}

#[test]
fn restored_session_keeps_cached_runtime_environment_without_exposing_it_to_web() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_restore_overrides"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "restore_overrides_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();

    let overrides = BTreeMap::from([
        ("TIMEM_MODEL".to_string(), "session-model".to_string()),
        ("TIMEM_STREAM".to_string(), "true".to_string()),
        ("TIMEM_OPENAI_CACHE_MODE".to_string(), "off".to_string()),
        (
            "TIMEM_API_KEY".to_string(),
            "session-only-secret".to_string(),
        ),
    ]);
    let session_id = create_session(
        &state,
        Some("Custom profile".to_string()),
        Some(root.display().to_string()),
        overrides,
    )
    .unwrap();
    let stored = current_session_store(&state)
        .unwrap()
        .load_session(&session_id)
        .unwrap()
        .unwrap();
    let persisted_overrides = stored.env_overrides.as_ref().unwrap();
    assert_eq!(
        persisted_overrides.get("TIMEM_MODEL").map(String::as_str),
        Some("session-model")
    );
    assert!(!persisted_overrides.contains_key("TIMEM_API_KEY"));
    assert_eq!(
        persisted_overrides.get("TIMEM_STREAM").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        stored.env.get("TIMEM_MODEL").map(String::as_str),
        Some("session-model")
    );
    assert_eq!(
        stored.env.get("TIMEM_API_KEY").map(String::as_str),
        Some("session-only-secret")
    );

    template.settings.lock().unwrap().config.model = "model-from-new-env".to_string();
    template.settings.lock().unwrap().config.api_key = "new-process-secret".to_string();
    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);
    assert_eq!(restore_stored_sessions(&restarted).unwrap(), 1);

    let sessions = restarted.sessions.lock().unwrap();
    let restored = sessions.get(&session_id).unwrap();
    assert_eq!(restored.runtime_profile.model, "session-model");
    assert_eq!(
        restored.runtime.settings.config.api_key,
        "session-only-secret"
    );
    assert!(restored.runtime.settings.config.openai_compatible.stream);
    assert_eq!(
        restored
            .runtime
            .settings
            .config
            .openai_compatible
            .cache_mode,
        agent_core::OpenAiCompatibleCacheMode::Off
    );
    assert!(!serde_json::to_string(restored)
        .unwrap()
        .contains("session-only-secret"));
}

#[test]
fn restored_web_turns_follow_history_time_not_turn_id_lexical_order() {
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_10".to_string(),
            created_at_ms: 10,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "first by time".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_10".to_string(),
            created_at_ms: 11,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "first answer".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_2".to_string(),
            created_at_ms: 20,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "second by time".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_2".to_string(),
            created_at_ms: 21,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "second answer".to_string(),
        },
    ];

    let turns = restored_turns_from_history_records(&records);
    assert_eq!(
        turns
            .iter()
            .map(|turn| turn.turn_id.as_str())
            .collect::<Vec<_>>(),
        vec!["turn_10", "turn_2"]
    );
    assert_eq!(turns[0].user_entries[0].text, "first by time");
    assert_eq!(turns[1].user_entries[0].text, "second by time");
}

#[test]
fn restored_web_turns_recover_terminal_completion_without_an_assistant_message() {
    let mut extra = BTreeMap::new();
    extra.insert(
        "completion".to_string(),
        json!({"stop_reason": "model_error", "rounds": 3}),
    );
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_completion_only".to_string(),
            created_at_ms: 10,
            kind: Some("task".to_string()),
            command_id: Some("accepted_command".to_string()),
            delivery_state: Some(ChatCommandDeliveryState::CoreAccepted),
            content: "task that ended without a final answer".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "turn_completion_only".to_string(),
            created_at_ms: 11,
            kind: ChatHistoryEventKind::Stats,
            content: "Turn completed.".to_string(),
            extra,
        },
    ];

    let turns = restored_turns_from_history_records(&records);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, "completed");
    assert_eq!(turns[0].final_answer, None);
    assert_eq!(
        turns[0].completion,
        Some(json!({"stop_reason": "model_error", "rounds": 3}))
    );
}

#[test]
fn restored_web_turns_preserve_user_entry_kinds() {
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 10,
            kind: Some("task".to_string()),
            command_id: None,
            delivery_state: None,
            content: "original task".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 11,
            kind: Some("supplement".to_string()),
            command_id: None,
            delivery_state: None,
            content: "mid-turn supplement".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 12,
            kind: Some("approval".to_string()),
            command_id: None,
            delivery_state: None,
            content: "approved request".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 13,
            kind: Some("unknown_legacy_kind".to_string()),
            command_id: None,
            delivery_state: None,
            content: "legacy text".to_string(),
        },
    ];

    let turns = restored_turns_from_history_records(&records);
    assert_eq!(
        turns[0]
            .user_entries
            .iter()
            .map(|entry| (entry.kind.as_str(), entry.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("task", "original task"),
            ("supplement", "mid-turn supplement"),
            ("approval", "approved request"),
            ("task", "legacy text"),
        ]
    );
}

#[test]
fn history_page_command_loads_older_records_by_cursor() {
    let state = routing_test_state();
    let session_id = "session_a";
    let store = current_session_store(&state).unwrap();
    for index in 0..450 {
        store
            .append_history_record(
                session_id,
                &ChatHistoryRecord::Message {
                    role: ChatHistoryRole::User,
                    turn_id: format!("turn_{index}"),
                    created_at_ms: index,
                    kind: None,
                    command_id: None,
                    delivery_state: None,
                    content: format!("line {index}"),
                },
            )
            .unwrap();
    }

    let first = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::HistoryPage {
            session_id: session_id.to_string(),
            before_cursor: None,
            limit: Some(200),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::HistoryPage {
        records,
        before_cursor,
        has_more,
        ..
    } = first
    else {
        panic!("expected history page")
    };
    assert_eq!(records.len(), 200);
    assert_eq!(records.first().unwrap().turn_id(), "turn_250");
    assert_eq!(before_cursor.as_deref(), Some("250"));
    assert!(has_more);

    let second = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::HistoryPage {
            session_id: session_id.to_string(),
            before_cursor,
            limit: Some(200),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::HistoryPage {
        records,
        before_cursor,
        has_more,
        ..
    } = second
    else {
        panic!("expected history page")
    };
    assert_eq!(records.len(), 200);
    assert_eq!(records.first().unwrap().turn_id(), "turn_50");
    assert_eq!(records.last().unwrap().turn_id(), "turn_249");
    assert_eq!(before_cursor.as_deref(), Some("50"));
    assert!(has_more);
}

#[test]
fn history_page_command_skips_malformed_records_without_breaking_cursor() {
    let state = routing_test_state();
    let session_id = "session_a";
    let history_path = current_session_store(&state)
        .unwrap()
        .history_path_for_session(session_id);
    std::fs::create_dir_all(history_path.parent().unwrap()).unwrap();
    let lines = [
        serde_json::to_string(&ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_0".to_string(),
            created_at_ms: 0,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "first valid".to_string(),
        })
        .unwrap(),
        "partial json from interrupted append".to_string(),
        serde_json::to_string(&ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_1".to_string(),
            created_at_ms: 1,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "second valid".to_string(),
        })
        .unwrap(),
        serde_json::to_string(&ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_2".to_string(),
            created_at_ms: 2,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "third valid".to_string(),
        })
        .unwrap(),
    ];
    std::fs::write(&history_path, format!("{}\n", lines.join("\n"))).unwrap();

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::HistoryPage {
            session_id: session_id.to_string(),
            before_cursor: None,
            limit: Some(2),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::HistoryPage {
        records,
        before_cursor,
        has_more,
        ..
    } = event
    else {
        panic!("expected history page")
    };

    assert_eq!(
        records
            .iter()
            .map(ChatHistoryRecord::turn_id)
            .collect::<Vec<_>>(),
        vec!["turn_1", "turn_2"]
    );
    assert_eq!(before_cursor.as_deref(), Some("1"));
    assert!(has_more);
}

#[test]
fn session_group_validation_rejects_case_insensitive_duplicate_names() {
    let groups = vec![
        SessionGroup {
            id: "group-a".to_string(),
            name: "Work".to_string(),
        },
        SessionGroup {
            id: "group-b".to_string(),
            name: "work".to_string(),
        },
    ];
    assert_eq!(
        crate::session_groups::validate_session_groups(&groups).unwrap_err(),
        "session_group_name_duplicate"
    );
}

#[test]
fn session_groups_create_move_persist_and_delete_without_deleting_sessions() {
    let state = routing_test_state();
    let workspace = std::env::temp_dir().join(unique_web_id("session_group_workspace"));
    std::fs::create_dir_all(&workspace).unwrap();
    let session_id = create_session(
        &state,
        Some("Grouped work".to_string()),
        Some(workspace.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();

    let Some(WireEvent::SessionGroupsUpdated { groups }) = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupCreate {
            name: "Projects".to_string(),
        },
    )
    .unwrap() else {
        panic!("expected session groups update");
    };
    assert_eq!(groups.len(), 1);
    let group_id = groups[0].id.clone();

    let moved = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupMove {
            session_id: session_id.clone(),
            group_id: Some(group_id.clone()),
        },
    )
    .unwrap();
    assert!(matches!(
        moved,
        Some(WireEvent::SessionGroupChanged {
            session_id: ref moved_session_id,
            group_id: Some(ref persisted_group_id),
        }) if moved_session_id == &session_id && persisted_group_id == &group_id
    ));
    assert_eq!(
        current_session_store(&state)
            .unwrap()
            .load_session(&session_id)
            .unwrap()
            .unwrap()
            .group_id
            .as_deref(),
        Some(group_id.as_str())
    );
    let snapshot = snapshot_for(&state, TEST_PORT);
    assert_eq!(snapshot.session_groups, groups);
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .unwrap()
            .group_id
            .as_deref(),
        Some(group_id.as_str())
    );

    let deleted = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupDelete {
            group_id: group_id.clone(),
        },
    )
    .unwrap();
    assert!(matches!(
        deleted,
        Some(WireEvent::SessionGroupsUpdated { ref groups }) if groups.is_empty()
    ));
    assert!(state.sessions.lock().unwrap().contains_key(&session_id));
    assert_eq!(
        current_session_store(&state)
            .unwrap()
            .load_session(&session_id)
            .unwrap()
            .unwrap()
            .group_id,
        None
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn session_group_move_persistence_failure_keeps_memory_and_disk_unchanged() {
    let state = routing_test_state();
    let Some(WireEvent::SessionGroupsUpdated { groups }) = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupCreate {
            name: "Failure test".to_string(),
        },
    )
    .unwrap() else {
        panic!("expected session groups update");
    };
    let group_id = groups[0].id.clone();
    let store = current_session_store(&state).unwrap();
    persist_web_session(&state, "session_a").unwrap();
    let metadata = store.metadata_path_for_session("session_a");
    let original = std::fs::read(&metadata).unwrap();
    std::fs::remove_file(&metadata).unwrap();
    std::fs::create_dir(&metadata).unwrap();

    let error = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupMove {
            session_id: "session_a".to_string(),
            group_id: Some(group_id),
        },
    )
    .unwrap_err();
    assert!(error.contains("session_metadata_write_failed"));
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"].group_id,
        None,
        "failed persistence must not mutate the in-memory session"
    );

    std::fs::remove_dir(&metadata).unwrap();
    std::fs::write(&metadata, original).unwrap();
    assert_eq!(
        store.load_session("session_a").unwrap().unwrap().group_id,
        None
    );
}

#[test]
fn session_group_delete_persistence_failure_rolls_back_prior_metadata_writes() {
    let state = routing_test_state();
    let Some(WireEvent::SessionGroupsUpdated { groups }) = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupCreate {
            name: "Rollback test".to_string(),
        },
    )
    .unwrap() else {
        panic!("expected session groups update");
    };
    let group_id = groups[0].id.clone();
    for session_id in ["session_a", "session_b"] {
        handle_command(
            &state,
            TEST_PORT,
            ClientCommand::SessionGroupMove {
                session_id: session_id.to_string(),
                group_id: Some(group_id.clone()),
            },
        )
        .unwrap();
    }
    let store = current_session_store(&state).unwrap();
    let blocked_metadata = store.metadata_path_for_session("session_b");
    let blocked_original = std::fs::read(&blocked_metadata).unwrap();
    std::fs::remove_file(&blocked_metadata).unwrap();
    std::fs::create_dir(&blocked_metadata).unwrap();

    let error = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionGroupDelete {
            group_id: group_id.clone(),
        },
    )
    .unwrap_err();
    assert!(error.contains("session_group_delete_persist_failed"));
    assert_eq!(
        current_mem_state(&state).unwrap().session_groups,
        groups,
        "failed deletion must keep the group definition"
    );
    for session_id in ["session_a", "session_b"] {
        assert_eq!(
            state.sessions.lock().unwrap()[session_id]
                .group_id
                .as_deref(),
            Some(group_id.as_str()),
            "failed deletion must keep every in-memory group assignment"
        );
    }
    assert_eq!(
        store
            .load_session("session_a")
            .unwrap()
            .unwrap()
            .group_id
            .as_deref(),
        Some(group_id.as_str()),
        "metadata written before the injected failure must be compensated"
    );

    std::fs::remove_dir(&blocked_metadata).unwrap();
    std::fs::write(&blocked_metadata, blocked_original).unwrap();
    assert_eq!(
        store
            .load_session("session_b")
            .unwrap()
            .unwrap()
            .group_id
            .as_deref(),
        Some(group_id.as_str())
    );
}

#[test]
fn snapshot_reports_worker_debug_ui_only_when_debug_store_is_enabled() {
    let mut state = routing_test_state();
    assert!(!snapshot_for(&state, TEST_PORT).server.debug_mode);

    state.debug = Some(Arc::new(DebugStore::create().unwrap()));
    assert!(snapshot_for(&state, TEST_PORT).server.debug_mode);
}

#[test]
fn snapshot_reports_the_active_mem_space_and_paths() {
    let state = routing_test_state();
    let snapshot = snapshot_for(&state, TEST_PORT);

    assert_eq!(snapshot.server.mem.space, ".test_mem");
    assert!(snapshot.server.mem.data_dir.contains("timem_web_data_test"));
    assert!(snapshot.server.mem.space_dir.ends_with(".test_mem"));
    assert!(snapshot.server.mem.memory_dir.ends_with(".test_mem/memory"));
    assert_eq!(snapshot.server.mem.temporary_retention_days, Some(5));
    assert_eq!(
        snapshot.server.mem.temporary_capacity_bytes,
        Some(MEM_CAPACITY_128_MB)
    );
    assert_eq!(
        snapshot.server.mem.conversation_capacity_bytes,
        Some(MEM_CAPACITY_128_MB)
    );
}

#[test]
fn web_mem_capacity_defaults_follow_launch_mode_without_overriding_saved_values() {
    let root = std::env::temp_dir().join(unique_web_id("mem_capacity_defaults"));
    std::fs::create_dir_all(&root).unwrap();

    let normal = load_web_mem_settings(&root, false).unwrap();
    assert_eq!(normal.temporary_retention_days, Some(5));
    assert_eq!(normal.temporary_capacity_bytes, Some(MEM_CAPACITY_128_MB));
    assert_eq!(
        normal.conversation_capacity_bytes,
        Some(MEM_CAPACITY_128_MB)
    );

    let debug = load_web_mem_settings(&root, true).unwrap();
    assert_eq!(debug.temporary_retention_days, Some(5));
    assert_eq!(debug.temporary_capacity_bytes, Some(MEM_CAPACITY_512_MB));
    assert_eq!(debug.conversation_capacity_bytes, Some(MEM_CAPACITY_128_MB));

    std::fs::write(
        web_mem_settings_path(&root),
        br#"{"temporary_retention_days":null,"temporary_capacity_bytes":null,"conversation_capacity_bytes":536870912}"#,
    )
    .unwrap();
    let saved = load_web_mem_settings(&root, true).unwrap();
    assert_eq!(saved.temporary_retention_days, None);
    assert_eq!(saved.temporary_capacity_bytes, None);
    assert_eq!(saved.conversation_capacity_bytes, Some(MEM_CAPACITY_512_MB));

    std::fs::write(
        web_mem_settings_path(&root),
        br#"{"temporary_retention_days":10}"#,
    )
    .unwrap();
    let migrated_normal = load_web_mem_settings(&root, false).unwrap();
    assert_eq!(migrated_normal.temporary_retention_days, Some(10));
    assert_eq!(
        migrated_normal.temporary_capacity_bytes,
        Some(MEM_CAPACITY_128_MB)
    );
    assert_eq!(
        migrated_normal.conversation_capacity_bytes,
        Some(MEM_CAPACITY_128_MB)
    );
    let migrated_debug = load_web_mem_settings(&root, true).unwrap();
    assert_eq!(
        migrated_debug.temporary_capacity_bytes,
        Some(MEM_CAPACITY_512_MB)
    );
    assert_eq!(
        migrated_debug.conversation_capacity_bytes,
        Some(MEM_CAPACITY_128_MB)
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opening_mem_does_not_run_temporary_retention_on_the_startup_path() {
    let state = routing_test_state();
    let (data_dir, space, store) = {
        let mem = state.mem.lock().unwrap();
        (
            state.template.data_dir.clone(),
            mem.space.clone(),
            mem.session_store.clone(),
        )
    };
    let session = state.sessions.lock().unwrap()["session_a"].clone();
    store
        .upsert_session(&stored_session_from_web_session_with_store(
            &store, &session,
        ))
        .unwrap();
    let expired = ChatHistoryRecord::Event {
        role: ChatHistoryRole::System,
        turn_id: "startup-retention-regression".to_string(),
        created_at_ms: now_ms_i64() - 6 * MILLIS_PER_DAY,
        kind: ChatHistoryEventKind::Action,
        content: "must remain until the background pass".to_string(),
        extra: BTreeMap::new(),
    };
    store.append_history_record("session_a", &expired).unwrap();

    WebMemState::new(data_dir, space).unwrap();

    assert!(
        read_all_history_records(&store.history_path_for_session("session_a"))
            .unwrap()
            .iter()
            .any(|record| matches!(
                record,
                ChatHistoryRecord::Event { content, .. }
                    if content == "must remain until the background pass"
            ))
    );
}

#[test]
fn mem_temporary_retention_is_mem_scoped_persisted_and_applies_to_all_temporary_data() {
    let state = routing_test_state();
    let (layout, store) = {
        let mem = state.mem.lock().unwrap();
        (mem.layout.clone(), mem.session_store.clone())
    };
    let session = state.sessions.lock().unwrap()["session_a"].clone();
    store
        .upsert_session(&stored_session_from_web_session_with_store(
            &store, &session,
        ))
        .unwrap();
    let now_ms = now_ms_i64();
    let old_ms = now_ms - 6 * MILLIS_PER_DAY;
    let recent_ms = now_ms - MILLIS_PER_DAY;
    for record in [
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "old-message".to_string(),
            created_at_ms: old_ms,
            kind: Some("task".to_string()),
            command_id: None,
            delivery_state: None,
            content: "old user message stays forever".to_string(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "old-action".to_string(),
            created_at_ms: old_ms,
            kind: ChatHistoryEventKind::Action,
            content: "expired temporary action".to_string(),
            extra: BTreeMap::new(),
        },
        ChatHistoryRecord::Event {
            role: ChatHistoryRole::System,
            turn_id: "recent-action".to_string(),
            created_at_ms: recent_ms,
            kind: ChatHistoryEventKind::ActionResult,
            content: "recent temporary action result".to_string(),
            extra: BTreeMap::new(),
        },
    ] {
        store.append_history_record("session_a", &record).unwrap();
    }

    std::fs::create_dir_all(layout.api_audit_file().parent().unwrap()).unwrap();
    std::fs::write(
        layout.api_audit_file(),
        serde_json::to_vec(&json!({
            "version": 1,
            "events": [
                {"type":"expired", "time_ms":old_ms},
                {"type":"recent", "time_ms":recent_ms}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .state = "working".to_string();
    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::MemTemporaryRetentionUpdate {
            days: Some(5),
            max_bytes: None,
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::MemSettingsUpdated {
        temporary_retention_days,
        temporary_capacity_bytes,
        conversation_capacity_bytes,
    } = event
    else {
        panic!("expected authoritative MEM settings event")
    };
    assert_eq!(temporary_retention_days, Some(5));
    assert_eq!(temporary_capacity_bytes, None);
    assert_eq!(conversation_capacity_bytes, Some(MEM_CAPACITY_128_MB));

    let retained = read_all_history_records(&store.history_path_for_session("session_a")).unwrap();
    assert!(retained.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Message { content, .. } if content == "old user message stays forever"
    )));
    assert!(!retained.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Event { content, .. } if content == "expired temporary action"
    )));
    assert!(retained.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Event { content, .. } if content == "recent temporary action result"
    )));
    let audit = agent_core::read_audit_doc(&layout.api_audit_file()).unwrap();
    assert_eq!(audit["events"].as_array().unwrap().len(), 1);
    assert_eq!(audit["events"][0]["type"], "recent");
    assert_eq!(
        load_web_mem_settings(store.root(), false)
            .unwrap()
            .temporary_retention_days,
        Some(5)
    );

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::MemTemporaryRetentionUpdate {
            days: None,
            max_bytes: None,
        },
    )
    .unwrap();
    assert_eq!(
        WebMemState::new(
            state.template.data_dir.clone(),
            state.mem.lock().unwrap().space.clone()
        )
        .unwrap()
        .settings
        .temporary_retention_days,
        None,
        "unlimited retention must survive reopening the same MEM"
    );
}

#[test]
fn ordinary_history_append_does_not_require_or_run_temporary_maintenance() {
    let state = routing_test_state();
    let session = state.sessions.lock().unwrap()["session_a"].clone();
    let store = current_session_store(&state).unwrap();
    store
        .upsert_session(&stored_session_from_web_session_with_store(
            &store, &session,
        ))
        .unwrap();

    append_history_record(
        &state,
        "session_a",
        &ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "ordinary_append".to_string(),
            created_at_ms: 1,
            kind: None,
            command_id: None,
            delivery_state: None,
            content: "hi".to_string(),
        },
    )
    .unwrap();

    let records = read_all_history_records(&store.history_path_for_session("session_a")).unwrap();
    assert!(records.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Message { content, .. } if content == "hi"
    )));
}

#[test]
fn idle_temporary_maintenance_is_skipped_while_any_session_has_live_work() {
    let state = routing_test_state();
    assert!(!has_live_mem_work(&state).unwrap());

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.state = "working".to_string();
        session.active_turn_id = Some("turn_live".to_string());
    }

    assert!(has_live_mem_work(&state).unwrap());
}

#[test]
fn idle_temporary_maintenance_is_allowed_only_after_work_is_clear() {
    let state = routing_test_state();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.pending_turn_id = Some("turn_pending".to_string());
    }
    assert!(has_live_mem_work(&state).unwrap());

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.pending_turn_id = None;
        session.active_turn_id = None;
        session.state = "ready".to_string();
    }
    assert!(!has_live_mem_work(&state).unwrap());
}

#[test]
fn temporary_maintenance_runtime_accumulates_across_restarts_without_counting_stopped_time() {
    let state = routing_test_state();
    let memory_dir = state.mem.lock().unwrap().layout.memory_dir();
    let first_now = tokio::time::Instant::now();
    {
        let mut mem = state.mem.lock().unwrap();
        mem.temporary_maintenance.checkpoint_started_at =
            first_now - Duration::from_secs(3 * 60 * 60);
        assert_eq!(
            checkpoint_temporary_maintenance_runtime(&mut mem, first_now).unwrap(),
            Duration::from_secs(3 * 60 * 60)
        );
    }

    // Reopening reads only the persisted accumulated duration. Wall-clock time
    // while no Timem process is running is not represented in this state.
    let mut reopened = load_temporary_maintenance_runtime_state(&memory_dir).unwrap();
    assert_eq!(
        reopened.accumulated_runtime,
        Duration::from_secs(3 * 60 * 60)
    );
    let second_now = tokio::time::Instant::now();
    reopened.checkpoint_started_at = second_now - Duration::from_secs(3 * 60 * 60);
    reopened.checkpoint(second_now);
    assert_eq!(reopened.accumulated_runtime, TEMPORARY_MAINTENANCE_INTERVAL);
}

#[test]
fn temporary_maintenance_trigger_persists_due_runtime_and_accepts_segment_hint() {
    let state = routing_test_state();
    let now = tokio::time::Instant::now();
    {
        let mut mem = state.mem.lock().unwrap();
        mem.temporary_maintenance.accumulated_runtime =
            TEMPORARY_MAINTENANCE_INTERVAL - Duration::from_secs(1);
        mem.temporary_maintenance.checkpoint_started_at = now - Duration::from_secs(1);
    }
    assert!(checkpoint_and_get_temporary_maintenance_trigger(&state, now).unwrap());

    let other = routing_test_state();
    let hint = {
        let mem = other.mem.lock().unwrap();
        agent_core::api_audit_maintenance_hint_path(&mem.layout.api_audit_file())
    };
    std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
    std::fs::write(&hint, b"audit_segment_rolled\n").unwrap();
    assert!(
        checkpoint_and_get_temporary_maintenance_trigger(&other, tokio::time::Instant::now())
            .unwrap()
    );
}

#[tokio::test]
async fn busy_temporary_maintenance_does_not_reset_due_runtime_or_hint() {
    let state = routing_test_state();
    let (memory_dir, hint) = {
        let mut mem = state.mem.lock().unwrap();
        mem.temporary_maintenance.accumulated_runtime = TEMPORARY_MAINTENANCE_INTERVAL;
        let memory_dir = mem.layout.memory_dir();
        save_temporary_maintenance_runtime_state(&memory_dir, &mem.temporary_maintenance).unwrap();
        (
            memory_dir,
            agent_core::api_audit_maintenance_hint_path(&mem.layout.api_audit_file()),
        )
    };
    std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
    std::fs::write(&hint, b"audit_segment_rolled\n").unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .state = "working".to_string();

    assert_eq!(
        run_idle_temporary_maintenance(state).await.unwrap(),
        TemporaryMaintenanceAttempt::Busy
    );
    assert!(hint.exists());
    assert_eq!(
        load_temporary_maintenance_runtime_state(&memory_dir)
            .unwrap()
            .accumulated_runtime,
        TEMPORARY_MAINTENANCE_INTERVAL
    );
}

#[test]
fn successful_temporary_maintenance_completion_resets_runtime_and_clears_hint() {
    let state = routing_test_state();
    let (memory_dir, hint) = {
        let mut mem = state.mem.lock().unwrap();
        mem.temporary_maintenance.accumulated_runtime = TEMPORARY_MAINTENANCE_INTERVAL;
        let memory_dir = mem.layout.memory_dir();
        save_temporary_maintenance_runtime_state(&memory_dir, &mem.temporary_maintenance).unwrap();
        (
            memory_dir,
            agent_core::api_audit_maintenance_hint_path(&mem.layout.api_audit_file()),
        )
    };
    std::fs::create_dir_all(hint.parent().unwrap()).unwrap();
    std::fs::write(&hint, b"audit_segment_rolled\n").unwrap();

    complete_temporary_maintenance(&state, tokio::time::Instant::now()).unwrap();
    assert!(!hint.exists());
    assert_eq!(
        load_temporary_maintenance_runtime_state(&memory_dir)
            .unwrap()
            .accumulated_runtime,
        Duration::ZERO
    );
}

#[tokio::test]
async fn idle_temporary_maintenance_waits_for_the_global_command_barrier() {
    let state = routing_test_state();
    let barrier = state.command_global_barrier.clone();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _read_guard = barrier.read().unwrap();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    locked_rx.recv().unwrap();

    let maintenance = tokio::spawn(run_idle_temporary_maintenance(state));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !maintenance.is_finished(),
        "maintenance must not overlap an accepted browser mutation"
    );

    release_tx.send(()).unwrap();
    holder.join().unwrap();
    maintenance.await.unwrap().unwrap();
}

#[test]
fn conversation_capacity_skips_running_session_and_prunes_another_session() {
    let state = routing_test_state();
    let store = state.mem.lock().unwrap().session_store.clone();
    let web_sessions = state.sessions.lock().unwrap().clone();
    for session in web_sessions.values() {
        store
            .upsert_session(&stored_session_from_web_session_with_store(&store, session))
            .unwrap();
        for turn in 0..2 {
            for role in [ChatHistoryRole::User, ChatHistoryRole::Assistant] {
                store
                    .append_history_record(
                        &session.session_id,
                        &ChatHistoryRecord::Message {
                            role,
                            turn_id: format!("{}_turn_{turn}", session.session_id),
                            created_at_ms: turn,
                            kind: None,
                            command_id: None,
                            delivery_state: None,
                            content: "capacity payload ".repeat(64),
                        },
                    )
                    .unwrap();
            }
        }
    }
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .active_turn_id = Some("session_a_turn_1".to_string());
    let active_path = store.history_path_for_session("session_a");
    let inactive_path = store.history_path_for_session("session_b");
    let active_before = std::fs::read(&active_path).unwrap();
    let inactive_before = std::fs::metadata(&inactive_path).unwrap().len();
    let total = active_before.len() as u64 + inactive_before;

    let removed = apply_conversation_stable_capacity(&state, &store, total - 1).unwrap();

    assert!(
        removed > 0,
        "another Session must still be pruned when the oldest candidate is active"
    );
    assert_eq!(std::fs::read(active_path).unwrap(), active_before);
    assert!(std::fs::metadata(inactive_path).unwrap().len() < inactive_before);
}

#[test]
fn temporary_capacity_evicts_only_complete_oldest_items() {
    let item = |id: &str, bytes: u64, modified_at_ms: i64| MemTemporaryItem {
        id: id.to_string(),
        path: format!("{id}.tmp"),
        kind: "temporary_file".to_string(),
        bytes,
        modified_at_ms,
        deletable: true,
        delete_reason: None,
    };
    let evicted = oldest_temporary_items_to_evict(
        vec![
            item("new", 6, 30),
            item("old", 7, 10),
            item("middle", 5, 20),
        ],
        11,
    );
    assert_eq!(evicted, vec!["old"]);
    assert_eq!(
        oldest_temporary_items_to_evict(vec![item("old", 5, 1), item("new", 6, 2)], 11,),
        Vec::<String>::new(),
        "content exactly at the stable capacity must be retained",
    );
    assert_eq!(
        oldest_temporary_items_to_evict(vec![item("old", 5, 1), item("new", 7, 2)], 11,),
        vec!["old"],
        "crossing capacity by one byte must evict the oldest complete item",
    );
    assert!(
        oldest_temporary_items_to_evict(vec![item("only", 12, 1)], 11)
            .iter()
            .eq(["only"])
    );
}

#[test]
fn temporary_retention_rolls_forward_is_idempotent_and_unlimited_skips_cleanup() {
    let state = routing_test_state();
    let (layout, store) = {
        let mem = state.mem.lock().unwrap();
        (mem.layout.clone(), mem.session_store.clone())
    };
    let session = state.sessions.lock().unwrap()["session_a"].clone();
    store
        .upsert_session(&stored_session_from_web_session_with_store(
            &store, &session,
        ))
        .unwrap();
    let day = MILLIS_PER_DAY;
    let base_now = 20 * day;
    for (created_at_ms, content) in [
        (base_now - 4 * day, "first expires on later tick"),
        (base_now - 5 * day, "exact cutoff remains"),
        (
            base_now - 5 * day - 1,
            "one millisecond before cutoff expires",
        ),
        (base_now - day, "second remains"),
    ] {
        store
            .append_history_record(
                "session_a",
                &ChatHistoryRecord::Event {
                    role: ChatHistoryRole::System,
                    turn_id: content.to_string(),
                    created_at_ms,
                    kind: ChatHistoryEventKind::Repair,
                    content: content.to_string(),
                    extra: BTreeMap::new(),
                },
            )
            .unwrap();
    }

    assert_eq!(
        apply_temporary_retention(&layout, &store, Some(5), None, base_now)
            .unwrap()
            .history_events,
        1,
        "age retention is strict: the exact cutoff remains and one millisecond older expires",
    );
    let after_boundary_pass =
        read_all_history_records(&store.history_path_for_session("session_a")).unwrap();
    assert!(after_boundary_pass.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Event { content, .. } if content == "exact cutoff remains"
    )));
    assert!(!after_boundary_pass.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Event { content, .. } if content == "one millisecond before cutoff expires"
    )));
    assert_eq!(
        apply_temporary_retention(&layout, &store, Some(5), None, base_now + 2 * day)
            .unwrap()
            .history_events,
        2,
        "the moving cutoff must expire both records that became old on a later periodic pass"
    );
    assert_eq!(
        apply_temporary_retention(&layout, &store, Some(5), None, base_now + 2 * day).unwrap(),
        TemporaryRetentionResult::default(),
        "repeating a periodic pass at the same cutoff must be idempotent"
    );
    assert_eq!(
        apply_temporary_retention(&layout, &store, None, None, i64::MAX).unwrap(),
        TemporaryRetentionResult::default()
    );
    let retained = read_all_history_records(&store.history_path_for_session("session_a")).unwrap();
    assert!(retained.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Event { content, .. } if content == "second remains"
    )));
}

#[test]
fn mem_temporary_items_are_bounded_sorted_safe_and_deletable() {
    let memory_dir = std::env::temp_dir().join(unique_web_id("mem_temporary_items"));
    std::fs::create_dir_all(memory_dir.join("nested")).unwrap();
    for index in 0..105_u64 {
        let path = memory_dir.join("nested").join(format!("item-{index}.tmp"));
        std::fs::write(path, vec![b'x'; usize::try_from(index + 1).unwrap()]).unwrap();
    }
    let persistent = memory_dir.join("persistent.json");
    std::fs::write(&persistent, vec![b'p'; 1000]).unwrap();
    let misleading = memory_dir.join("nested/not-temporary.txt");
    std::fs::write(&misleading, vec![b'n'; 2000]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&persistent, memory_dir.join("nested/link.tmp")).unwrap();

    let items = list_mem_temporary_items_at(&memory_dir).unwrap();
    assert_eq!(items.len(), 100);
    assert!(items.windows(2).all(|pair| pair[0].bytes >= pair[1].bytes));
    assert_eq!(items[0].bytes, 105);
    assert_eq!(items.last().unwrap().bytes, 6);
    assert!(items.iter().all(|item| item.kind == "temporary_file"));
    assert!(!items
        .iter()
        .any(|item| item.path.contains("persistent.json")));
    assert!(!items
        .iter()
        .any(|item| item.path.contains("not-temporary.txt")));
    assert!(!items.iter().any(|item| item.path.contains("link.tmp")));

    let selected = items[0].clone();
    assert_eq!(
        delete_mem_temporary_items_at(&memory_dir, std::slice::from_ref(&selected.id)).unwrap(),
        1
    );
    assert!(!memory_dir.join(&selected.path).exists());
    assert!(persistent.exists());
    assert!(
        delete_mem_temporary_items_at(&memory_dir, &["file:../escape.tmp".to_string()]).is_err()
    );
    assert!(
        delete_mem_temporary_items_at(&memory_dir, &["file:persistent.json".to_string()]).is_err()
    );
    let _ = std::fs::remove_dir_all(memory_dir);
}
#[test]
fn mem_temporary_items_ignore_legacy_shell_job_directories() {
    let memory_dir = std::env::temp_dir().join(unique_web_id("ignore_legacy_shell_jobs"));
    let shell_dir = memory_dir.join("shell_jobs");
    let nested_shell_dir = memory_dir.join("memory/shell_jobs");
    std::fs::create_dir_all(&shell_dir).unwrap();
    std::fs::create_dir_all(&nested_shell_dir).unwrap();
    std::fs::write(shell_dir.join("old.out"), "legacy").unwrap();
    std::fs::write(shell_dir.join("jobs.jsonl"), "legacy index").unwrap();
    std::fs::write(nested_shell_dir.join("old.err"), "legacy").unwrap();
    std::fs::write(memory_dir.join("visible.tmp"), "temporary").unwrap();

    let items = list_mem_temporary_items_at(&memory_dir).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "file:visible.tmp");
    assert!(!items.iter().any(|item| item.path.contains("shell_jobs")));
    let _ = std::fs::remove_dir_all(memory_dir);
}

#[test]
fn mem_temporary_retention_rejects_invalid_values() {
    let state = routing_test_state();
    assert_eq!(
        handle_command(
            &state,
            TEST_PORT,
            ClientCommand::MemTemporaryRetentionUpdate {
                days: Some(2),
                max_bytes: None
            },
        )
        .unwrap_err(),
        "mem_temporary_retention_days_invalid"
    );
}

#[test]
fn mem_switch_swaps_out_sessions_and_loads_the_selected_space() {
    let mut state = routing_test_state();
    let data_dir_raw = std::env::temp_dir().join(unique_web_id("timem_web_mem_switch"));
    std::fs::create_dir_all(&data_dir_raw).unwrap();
    let data_dir = std::fs::canonicalize(data_dir_raw).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = data_dir.clone();
    template.workspace_dirs = vec![data_dir.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = data_dir.join("alpha").display().to_string();
    state.template = Arc::new(template);
    set_test_mem(
        &state,
        data_dir.clone(),
        &data_dir.join("alpha").display().to_string(),
    );
    state.sessions.lock().unwrap().clear();

    let alpha_session = create_session(
        &state,
        Some("Alpha work".to_string()),
        None,
        BTreeMap::new(),
    )
    .unwrap();
    start_web_turn(&state, &alpha_session, "alpha task").unwrap();

    set_test_mem(
        &state,
        data_dir.clone(),
        &data_dir.join("beta").display().to_string(),
    );
    state.sessions.lock().unwrap().clear();
    let beta_session =
        create_session(&state, Some("Beta work".to_string()), None, BTreeMap::new()).unwrap();
    start_web_turn(&state, &beta_session, "beta task").unwrap();

    set_test_mem(
        &state,
        data_dir.clone(),
        &data_dir.join("alpha").display().to_string(),
    );
    state.sessions.lock().unwrap().clear();
    restore_stored_sessions(&state).unwrap();
    assert!(state.sessions.lock().unwrap().contains_key(&alpha_session));
    assert!(!state.sessions.lock().unwrap().contains_key(&beta_session));

    let mut events = state.events.subscribe();
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::MemSwitch {
            path: data_dir.join("beta").display().to_string(),
            stop_running: false,
        },
    )
    .unwrap()
    .is_none());

    let WireEvent::Hello { snapshot, .. } = events.try_recv().unwrap() else {
        panic!("expected hello snapshot after mem switch")
    };
    assert_eq!(
        snapshot.server.mem.space,
        std::fs::canonicalize(data_dir.join("beta"))
            .unwrap()
            .display()
            .to_string()
    );
    assert!(snapshot
        .sessions
        .iter()
        .any(|session| session.session_id == beta_session));
    assert!(!snapshot
        .sessions
        .iter()
        .any(|session| session.session_id == alpha_session));
    let beta_history_path = current_session_store(&state)
        .unwrap()
        .history_path_for_session(&beta_session);
    assert!(
        read_all_history_records(&beta_history_path)
            .unwrap()
            .iter()
            .all(|record| !matches!(
                record,
                ChatHistoryRecord::Message {
                    role: ChatHistoryRole::System,
                    kind: Some(kind),
                    ..
                } if kind == RUNTIME_RESTART_HISTORY_KIND
            )),
        "switching memory spaces inside one process is not a runtime restart"
    );
    let instance_path = state.web_instance.lock().unwrap().path().to_path_buf();
    assert_eq!(
        instance_path,
        data_dir.join("beta").join("web_instance.json"),
        "switching mem must also switch the per-MEM Web instance lease"
    );
}

#[test]
fn same_mem_switch_is_a_noop_even_when_work_is_live() {
    let state = routing_test_state();
    let current_path = current_mem_state(&state)
        .unwrap()
        .layout
        .space_dir()
        .display()
        .to_string();
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .active_turn_id = Some("active_turn".to_string());

    let snapshot = switch_mem_space(&state, TEST_PORT, &current_path, false).unwrap();
    assert_eq!(snapshot.server.mem.space_dir, current_path);
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"]
            .active_turn_id
            .as_deref(),
        Some("active_turn")
    );
}

#[test]
fn confirmed_mem_switch_stops_real_worker_persists_interruption_and_never_redrives() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("confirmed_mem_switch"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let alpha = data_dir.join("alpha");
    let beta = data_dir.join("beta");
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = alpha.display().to_string();
    state.template = Arc::new(template);
    set_test_mem(&state, data_dir.clone(), &alpha.display().to_string());
    state.sessions.lock().unwrap().clear();

    let entered = Arc::new(AtomicUsize::new(0));
    let session_id = register_cancellable_mem_switch_worker(&state, Arc::clone(&entered), &root);
    let turn = submit_turn(&state, &session_id, "keep running".to_string()).unwrap();
    let started = Instant::now();
    while entered.load(Ordering::SeqCst) == 0 {
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    let old_store = current_session_store(&state).unwrap();
    assert_eq!(
        switch_mem_space(&state, TEST_PORT, &beta.display().to_string(), false).unwrap_err(),
        "mem_switch_active_sessions_confirmation_required"
    );
    assert_eq!(state.manager.lock().unwrap().worker_count(), 1);
    assert_eq!(current_mem_state(&state).unwrap().layout.space_dir(), alpha);

    switch_mem_space(&state, TEST_PORT, &beta.display().to_string(), true).unwrap();
    assert!(state
        .manager
        .lock()
        .unwrap()
        .statuses()
        .iter()
        .all(|status| status.identity.session_id != session_id));
    let stored = old_store.load_session(&session_id).unwrap().unwrap();
    assert_eq!(stored.state, StoredSessionState::Interrupted);
    assert_eq!(stored.last_turn_id.as_deref(), Some(turn.turn_id.as_str()));

    let mut events = state.events.subscribe();
    switch_mem_space(&state, TEST_PORT, &alpha.display().to_string(), false).unwrap();
    let restored = state.sessions.lock().unwrap()[&session_id].clone();
    assert_eq!(restored.state, "interrupted");
    assert!(restored
        .workers
        .iter()
        .all(|worker| worker.state == "ready"));
    assert_eq!(
        restored
            .turns
            .iter()
            .find(|candidate| candidate.turn_id == turn.turn_id)
            .unwrap()
            .state,
        "interrupted"
    );
    thread::sleep(Duration::from_millis(30));
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, WireEvent::TurnStarted { .. }));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mem_switch_requires_a_safe_absolute_directory_path() {
    let state = routing_test_state();
    let too_long = format!("/tmp/{}", "a".repeat(4097));
    for path in [
        "",
        ".",
        "..",
        "../other",
        "alpha/beta",
        "/tmp/../other",
        too_long.as_str(),
    ] {
        assert!(handle_command(
            &state,
            TEST_PORT,
            ClientCommand::MemSwitch {
                path: path.to_string(),
                stop_running: false,
            },
        )
        .is_err());
    }
}

#[test]
fn mem_switch_rejects_active_sessions_before_touching_mem_scoped_instance_leases() {
    let state = routing_test_state();
    let original_mem = current_mem_state(&state).unwrap().space;
    let original_instance_lease = state.web_instance.lock().unwrap().path().to_path_buf();
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .active_turn_id = Some("active_turn".to_string());

    assert_eq!(
        switch_mem_space(&state, TEST_PORT, ".next_mem", false).unwrap_err(),
        "mem_switch_active_sessions_confirmation_required"
    );
    assert_eq!(current_mem_state(&state).unwrap().space, original_mem);
    assert_eq!(
        state.web_instance.lock().unwrap().path(),
        original_instance_lease
    );
}

#[test]
fn session_runtime_env_rejects_unknown_empty_and_invalid_values() {
    let state = routing_test_state();
    assert_eq!(
        state
            .template
            .session_settings(&BTreeMap::from([(
                "PATH".to_string(),
                "/tmp/bin".to_string(),
            )]))
            .err()
            .unwrap(),
        "unsupported_session_env_key:PATH"
    );
    assert_eq!(
        state
            .template
            .session_settings(&BTreeMap::from([(
                "TIMEM_MODEL".to_string(),
                "  ".to_string(),
            )]))
            .err()
            .unwrap(),
        "empty_session_env_value:TIMEM_MODEL"
    );
    assert_eq!(
        state
            .template
            .session_settings(&BTreeMap::from([(
                "TIMEM_TIMEOUT".to_string(),
                "0".to_string(),
            )]))
            .err()
            .unwrap(),
        "invalid_session_timeout"
    );
    assert_eq!(
        state
            .template
            .session_settings(&BTreeMap::from([(
                "TIMEM_RESPONSE_PROTOCOL".to_string(),
                "yaml".to_string(),
            )]))
            .err()
            .unwrap(),
        "invalid_session_response_protocol"
    );
    assert!(state
        .template
        .session_settings(&BTreeMap::from([(
            "TIMEM_STREAM".to_string(),
            "sometimes".to_string(),
        )]))
        .unwrap_err()
        .contains("invalid_TIMEM_STREAM"));
}

#[test]
fn session_runtime_env_accepts_an_explicitly_unconfigured_api_key_draft() {
    let state = routing_test_state();
    let settings = state
        .template
        .session_settings(&BTreeMap::from([(
            "TIMEM_API_KEY".to_string(),
            String::new(),
        )]))
        .unwrap();
    assert!(settings.config.api_key.is_empty());
}

#[test]
fn session_runtime_env_restores_protocol_base_url_and_model_as_one_configuration() {
    let state = routing_test_state();
    let settings = state
        .template
        .session_settings(&BTreeMap::from([
            (
                "TIMEM_BASE_URL".to_string(),
                "https://gateway.example.test/v1".to_string(),
            ),
            ("TIMEM_API_PROTOCOL".to_string(), "anthropic".to_string()),
            ("TIMEM_MODEL".to_string(), "custom-model".to_string()),
            ("TIMEM_API_KEY".to_string(), String::new()),
        ]))
        .unwrap();
    assert_eq!(settings.config.base_url, "https://gateway.example.test/v1");
    assert_eq!(
        settings.config.api_protocol,
        agent_core::ApiProtocol::Anthropic
    );
    assert_eq!(settings.config.model, "custom-model");
    assert!(settings.config.api_key.is_empty());
}

#[test]
fn ask_mode_does_not_announce_work_instructions_before_user_acceptance() {
    let state = routing_test_state();
    let session_id = "session_a";
    let current_dir = std::env::temp_dir().join(format!(
        "timem_web_work_instruction_notice_{}",
        unique_web_id("test")
    ));
    std::fs::create_dir_all(&current_dir).unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.work_instruction_mode = WorkInstructionLoadMode::Ask;
        session.current_dir = current_dir.display().to_string();
    }
    std::fs::write(current_dir.join("AGENTS.md"), "Wait for approval.").unwrap();

    assert!(work_instruction_notice_event(&state, session_id).is_none());
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut(session_id)
        .unwrap()
        .work_instruction_allowed = Some(true);
    assert!(work_instruction_notice_event(&state, session_id).is_some());
}

fn routing_test_state() -> AppState {
    let config = ModelServiceConfig {
        interaction: Default::default(),
        model: "test-model".to_string(),
        base_url: "http://127.0.0.1".to_string(),
        api_key: "test".to_string(),
        http_headers: Default::default(),
        request_fields: Default::default(),
        timeout_secs: 1,
        max_llm_output_tokens: 1_024,
        max_llm_input_tokens: 10_000,
        api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
        response_protocol: ResponseProtocolKind::Xml,
        openai_compatible: agent_core::OpenAiCompatibleOptions::default(),
    };
    let template = WorkerTemplate {
        settings: Arc::new(Mutex::new(RuntimeSettings {
            config,
            bash_approval_mode: BashApprovalMode::Ask,
            work_instruction_mode: WorkInstructionLoadMode::Off,
            max_rounds: agent_core::UNLIMITED_ROUND_BUDGET,
        })),
        data_dir: std::env::temp_dir().join(unique_web_id("timem_web_data_test")),
        initial_space: ".test_mem".to_string(),
        env: BTreeMap::new(),
        current_dir: PathBuf::from("/work"),
        workspace_dirs: vec![PathBuf::from("/work")],
        reminder_tips_config: agent_core::ReminderTipsConfig::default(),
    };
    let sessions = ["session_a", "session_b"]
        .into_iter()
        .enumerate()
        .map(|(ordinal, session_id)| {
            (
                session_id.to_string(),
                test_web_session(session_id, ordinal as u32, format!("Agent {ordinal}")),
            )
        })
        .collect();
    let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
    AppState {
        token: "test".to_string(),
        public_access: false,
        manager: Arc::new(Mutex::new(CoreSessionWorkerManager::new())),
        mem: Arc::new(Mutex::new(
            WebMemState::new(template.data_dir.clone(), template.initial_space.clone()).unwrap(),
        )),
        template: Arc::new(template),
        events: events.clone(),
        sessions: Arc::new(Mutex::new(sessions)),
        command_dedup: Arc::new(Mutex::new(CommandDedupCache::default())),
        semantic_delivery: Arc::new(SemanticEventDelivery::new(events.clone())),
        web_instance: Arc::new(Mutex::new(
            WebInstanceLease::acquire(
                &std::env::temp_dir().join(unique_web_id("routing_test_web_instance.json")),
            )
            .unwrap(),
        )),
        command_lanes: Arc::new(Mutex::new(HashMap::new())),
        command_global_barrier: Arc::new(RwLock::new(())),
        mem_epoch: Arc::new(RwLock::new(1)),
        debug: None,
        runtime_log: RuntimeLog::default(),
    }
}

#[test]
fn mem_temporary_commands_return_correlated_results_without_background_broadcast() {
    assert!(ClientCommand::MemTemporaryItemsList.result_is_direct());
    assert!(!ClientCommand::MemTemporaryItemsList.result_is_sensitive());
    assert!(ClientCommand::MemTemporaryItemsDelete { ids: Vec::new() }.result_is_direct());
    assert!(!ClientCommand::MemTemporaryItemsDelete { ids: Vec::new() }.result_is_sensitive());

    let state = routing_test_state();
    let memory_dir = state.mem.lock().unwrap().layout.memory_dir();
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("listed.tmp"), b"temporary payload").unwrap();

    let list_command_id = "mem_temporary_list_direct";
    assert!(reserve_command_dedup(&state, list_command_id)
        .unwrap()
        .is_none());
    let list = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: Some(list_command_id.to_string()),
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::MemTemporaryItemsList,
        },
    );
    assert!(matches!(
        list.ack,
        Some(WireEvent::CommandAck {
            status: CommandAckStatus::Committed,
            ..
        })
    ));
    let Some(WireEvent::MemTemporaryItems { items, error }) = list.event else {
        panic!("list command must return its result directly")
    };
    assert!(error.is_none());
    assert!(items.iter().any(|item| item.id == "file:listed.tmp"));
    assert!(matches!(
        reserve_command_dedup(&state, list_command_id).unwrap(),
        Some(CommandDedupState::Committed {
            serialized_event: Some(_),
            ..
        })
    ));

    let delete_command_id = "mem_temporary_delete_direct";
    assert!(reserve_command_dedup(&state, delete_command_id)
        .unwrap()
        .is_none());
    let delete = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: Some(delete_command_id.to_string()),
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::MemTemporaryItemsDelete {
                ids: vec!["file:listed.tmp".to_string()],
            },
        },
    );
    assert!(matches!(
        delete.ack,
        Some(WireEvent::CommandAck {
            status: CommandAckStatus::Committed,
            ..
        })
    ));
    let Some(WireEvent::MemTemporaryItems { items, error }) = delete.event else {
        panic!("delete command must return the refreshed list directly")
    };
    assert!(error.is_none());
    assert!(!items.iter().any(|item| item.id == "file:listed.tmp"));
    assert!(!memory_dir.join("listed.tmp").exists());
}

fn set_test_mem(state: &AppState, data_dir: PathBuf, space: &str) {
    *state.mem.lock().unwrap() = WebMemState::new(data_dir, space.to_string()).unwrap();
}

fn test_web_session(session_id: &str, ordinal: u32, display_name: String) -> WebSession {
    let context_id = test_context_id(session_id);
    let worker_id = test_worker_id(session_id);
    let settings = test_runtime_settings();
    WebSession {
        session_id: session_id.to_string(),
        group_id: None,
        display_name,
        ordinal,
        state: "ready".to_string(),
        current_dir: "/work".to_string(),
        debug_dir: None,
        max_llm_input_tokens: 10_000,
        tools: Vec::new(),
        mcp_server_ids: Vec::new(),
        mcp_config_revision: 0,
        applied_mcp_config_revision: 0,
        runtime_profile: test_runtime_profile(),
        contexts: vec![WebContext {
            context_id: context_id.clone(),
            current_dir: "/work".to_string(),
            worker_ids: vec![worker_id.clone()],
        }],
        workers: vec![WebWorker {
            worker_id: worker_id.clone(),
            context_id: context_id.clone(),
            display_name: format!("ID{ordinal}"),
            ordinal,
            state: "ready".to_string(),
            parent_worker_id: None,
        }],
        active_context_id: context_id,
        primary_worker_id: worker_id,
        attachments: Vec::new(),
        roles: Vec::new(),
        consumed_attachment_ids: BTreeSet::new(),
        messages: Vec::new(),
        turns: Vec::new(),
        history_before_cursor: None,
        history_has_more: false,
        resume_notice_pending: false,
        active_turn_id: None,
        cancelling_turn_id: None,
        pending_turn_id: None,
        turn_projection: TurnProjectionCache::default(),
        next_turn_intents: NextTurnIntentQueue::new(MAX_NEXT_TURN_INTENTS),
        pending_completion_message_id: None,
        pending_unconsumed_supplements: Vec::new(),
        reported_session_working_worker_count: None,
        work_instruction_mode: WorkInstructionLoadMode::Off,
        work_instruction_allowed: None,
        pending_work_instruction_turn: None,
        runtime: WebSessionRuntime {
            settings,
            env: BTreeMap::new(),
            env_overrides: BTreeMap::new(),
        },
    }
}

fn test_context_id(session_id: &str) -> String {
    format!("context_{session_id}")
}

fn test_worker_id(session_id: &str) -> String {
    format!("worker_{session_id}")
}

fn test_runtime_settings() -> RuntimeSettings {
    RuntimeSettings {
        config: ModelServiceConfig {
            interaction: Default::default(),
            model: "model".to_string(),
            base_url: "http://127.0.0.1:9".to_string(),
            api_key: "test".to_string(),
            http_headers: Default::default(),
            request_fields: Default::default(),
            timeout_secs: 1,
            max_llm_output_tokens: 1_024,
            max_llm_input_tokens: 10_000,
            api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
            response_protocol: ResponseProtocolKind::Xml,
            openai_compatible: agent_core::OpenAiCompatibleOptions::default(),
        },
        bash_approval_mode: BashApprovalMode::Ask,
        work_instruction_mode: WorkInstructionLoadMode::Off,
        max_rounds: agent_core::UNLIMITED_ROUND_BUDGET,
    }
}

fn test_runtime_profile() -> WebSessionRuntimeProfile {
    WebSessionRuntimeProfile {
        model: "model".to_string(),
        api_protocol: "openai-compatible".to_string(),
        response_protocol: "xml".to_string(),
        base_url: "http://127.0.0.1:9".to_string(),
        timeout_secs: 1,
        max_llm_input_tokens: 10_000,
        max_llm_output_tokens: 1_024,
        stream: false,
        max_rounds: "unlimited".to_string(),
        bash_approval: "ask".to_string(),
        work_instructions: "off".to_string(),
        api_key_configured: true,
    }
}

#[test]
fn round_budget_configuration_accepts_ui_choices_and_rejects_invalid_values() {
    assert_eq!(parse_round_budget("50").unwrap(), 50);
    assert_eq!(parse_round_budget("200").unwrap(), 200);
    assert_eq!(parse_round_budget("500").unwrap(), 500);
    assert_eq!(
        parse_round_budget("unlimited").unwrap(),
        agent_core::UNLIMITED_ROUND_BUDGET
    );
    assert_eq!(
        round_budget_value(agent_core::UNLIMITED_ROUND_BUDGET),
        "unlimited"
    );
    assert_eq!(
        parse_round_budget("0").unwrap_err(),
        "invalid_session_max_rounds"
    );
    assert_eq!(
        parse_round_budget("not-a-number").unwrap_err(),
        "invalid_session_max_rounds"
    );
}

#[test]
fn snapshot_exposes_unlimited_max_steps_as_the_session_default() {
    let state = routing_test_state();
    let snapshot = snapshot_for(&state, TEST_PORT);
    let option = snapshot
        .server
        .runtime_options
        .iter()
        .find(|option| option.key == "TIMEM_MAX_ROUNDS")
        .expect("max steps runtime option");
    assert_eq!(option.value, "unlimited");
    assert_eq!(
        snapshot
            .server
            .session_env_defaults
            .get("TIMEM_MAX_ROUNDS")
            .map(String::as_str),
        Some("unlimited")
    );
}

fn final_response_topic(session_id: &str, answer: String) -> CoreTopicEvent {
    CoreTopicEvent::new(
        session_id,
        CoreTopic::new(CORE_TOPIC_MODEL_RESPONSE, json!({})),
        CoreSessionState::Finished,
        json!({
            "status": "ALL_FINISHED",
            "final_answer": answer,
            "continue_work": false,
            "global": { "working_worker_count": 0 },
        }),
    )
    .with_worker_scope(test_context_id(session_id), test_worker_id(session_id))
}

fn handle_worker_event(state: &AppState, session_id: &str, event: CoreSessionWorkerEvent) {
    let event = match event {
        CoreSessionWorkerEvent::Topics(events) => CoreSessionWorkerEvent::Topics(
            events
                .into_iter()
                .map(|event| {
                    if event.context_id.is_none() || event.worker_id.is_none() {
                        event.with_worker_scope(
                            test_context_id(session_id),
                            test_worker_id(session_id),
                        )
                    } else {
                        event
                    }
                })
                .collect(),
        ),
        event => event,
    };
    handle_scoped_worker_event(
        state,
        session_id,
        &test_context_id(session_id),
        &test_worker_id(session_id),
        event,
    );
}

fn drain_wire_events(receiver: &mut broadcast::Receiver<WireEvent>) -> Vec<WireEvent> {
    let mut events = Vec::new();
    loop {
        match receiver.try_recv() {
            Ok(WireEvent::SemanticEvent { event, .. }) => events.push(*event),
            Ok(event) => events.push(event),
            Err(broadcast::error::TryRecvError::Empty)
            | Err(broadcast::error::TryRecvError::Closed) => return events,
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                panic!("topic routing test exceeded broadcast capacity: skipped {skipped} events")
            }
        }
    }
}

#[test]
fn primary_core_projection_is_cached_revisioned_and_not_rewritten_by_stale_or_subworker_events() {
    let state = routing_test_state();
    let session_id = "session_a";
    let token = agent_core::TurnToken {
        session_id: session_id.to_string(),
        turn_id: "turn_projection_test".to_string(),
        epoch: 7,
    };
    let active = agent_core::TurnProjection::Active(agent_core::ActiveTurnProjection {
        token: token.clone(),
        stop_requested: false,
        input_admission: agent_core::TurnInputAdmission::Open,
        activity: agent_core::TurnActivity::Running,
    });
    let finished = agent_core::TurnProjection::Finished(agent_core::FinishedTurnProjection {
        token: token.clone(),
        outcome: agent_core::TurnProjectionOutcome::Completed,
    });
    let mut events = state.events.subscribe();

    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::TurnProjection(active.clone()),
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::TurnProjection(active.clone()),
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &test_context_id(session_id),
        "subworker",
        CoreSessionWorkerEvent::TurnProjection(finished.clone()),
    );

    {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions.get(session_id).unwrap();
        let current = session.turn_projection.current().unwrap();
        assert_eq!(current.revision, 1);
        assert_eq!(current.projection, active);
        assert_eq!(session.state, "working");
    }

    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::TurnProjection(finished.clone()),
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::TurnProjection(agent_core::TurnProjection::Active(
            agent_core::ActiveTurnProjection {
                token,
                stop_requested: true,
                input_admission: agent_core::TurnInputAdmission::Closed,
                activity: agent_core::TurnActivity::WaitingUser,
            },
        )),
    );

    {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions.get(session_id).unwrap();
        let current = session.turn_projection.current().unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.projection, finished);
        assert_eq!(session.state, "ready");
    }

    let projected = drain_wire_events(&mut events)
        .into_iter()
        .filter_map(|event| match event {
            WireEvent::TurnProjection { projection, .. } => Some(projection),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        projected
            .iter()
            .map(|item| item.revision)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

fn wait_for_web_worker_event(
    state: &AppState,
    worker_id: &str,
    label: &str,
) -> CoreSessionWorkerEvent {
    let started = Instant::now();
    loop {
        if let Some(event) = state.manager.lock().unwrap().try_recv_event(worker_id) {
            return event;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{label} timed out waiting for worker event"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn concurrent_agent_topics_stay_in_their_own_session_and_wire_payload() {
    const EVENTS_PER_AGENT: usize = 60;
    let state = routing_test_state();
    let mut events = state.events.subscribe();
    let workers = ["session_a", "session_b"].map(|session_id| {
        let state = state.clone();
        thread::spawn(move || {
            for index in 0..EVENTS_PER_AGENT {
                let answer = format!("{session_id}:reply:{index}");
                handle_worker_event(
                    &state,
                    session_id,
                    CoreSessionWorkerEvent::Topics(vec![final_response_topic(session_id, answer)]),
                );
            }
        })
    });
    for worker in workers {
        worker.join().expect("topic routing worker must not panic");
    }

    let sessions = state.sessions.lock().unwrap();
    for session_id in ["session_a", "session_b"] {
        let session = sessions.get(session_id).unwrap();
        assert_eq!(session.messages.len(), EVENTS_PER_AGENT);
        assert!(session
            .messages
            .iter()
            .all(|message| message.text.starts_with(&format!("{session_id}:reply:"))));
        assert_eq!(session.state, "ready");
    }
    drop(sessions);

    let mut forwarded = BTreeMap::<String, usize>::new();
    for event in drain_wire_events(&mut events) {
        if let WireEvent::CoreTopic { event, .. } = event {
            let session_id = event["session_id"].as_str().unwrap().to_string();
            *forwarded.entry(session_id).or_default() += 1;
        }
    }
    assert_eq!(forwarded.get("session_a"), Some(&EVENTS_PER_AGENT));
    assert_eq!(forwarded.get("session_b"), Some(&EVENTS_PER_AGENT));
}

#[test]
fn barrier_synchronized_sessions_keep_request_action_final_and_completion_scoped() {
    const SESSIONS: usize = 8;
    let state = routing_test_state();
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.clear();
        for ordinal in 0..SESSIONS {
            let session_id = format!("barrier_session_{ordinal}");
            sessions.insert(
                session_id.clone(),
                test_web_session(&session_id, ordinal as u32, format!("Agent {ordinal}")),
            );
        }
    }
    for ordinal in 0..SESSIONS {
        start_web_turn(
            &state,
            &format!("barrier_session_{ordinal}"),
            &format!("task-{ordinal}"),
        )
        .unwrap();
    }
    let start = Arc::new(std::sync::Barrier::new(SESSIONS));
    let after_request = Arc::new(std::sync::Barrier::new(SESSIONS));
    let after_action = Arc::new(std::sync::Barrier::new(SESSIONS));
    let threads = (0..SESSIONS)
        .map(|ordinal| {
            let state = state.clone();
            let start = Arc::clone(&start);
            let after_request = Arc::clone(&after_request);
            let after_action = Arc::clone(&after_action);
            thread::spawn(move || {
                let session_id = format!("barrier_session_{ordinal}");
                start.wait();
                handle_worker_event(
                    &state,
                    &session_id,
                    CoreSessionWorkerEvent::ModelRequest {
                        round: 1,
                        prompt: String::new(),
                        interaction_profile: None,
                        interaction_request: None,
                        api_payload: None,
                    },
                );
                after_request.wait();
                let action = CoreTopicEvent::new(
                    &session_id,
                    CoreTopic::new(CORE_TOPIC_ACTION, json!({ "event": "finish" })),
                    CoreSessionState::Running,
                    json!({ "action": "run_bash", "status": "completed", "marker": session_id }),
                );
                handle_worker_event(
                    &state,
                    &session_id,
                    CoreSessionWorkerEvent::Topics(vec![action]),
                );
                after_action.wait();
                handle_worker_event(
                    &state,
                    &session_id,
                    CoreSessionWorkerEvent::Topics(vec![final_response_topic(
                        &session_id,
                        format!("final-{ordinal}"),
                    )]),
                );
                handle_worker_event(
                    &state,
                    &session_id,
                    CoreSessionWorkerEvent::TurnFinished {
                        outcome: TurnOutcome::final_response(
                            format!("final-{ordinal}"),
                            UsageStats::zero(),
                            None,
                            None,
                            Duration::ZERO,
                        ),
                    },
                );
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }

    let sessions = state.sessions.lock().unwrap();
    for ordinal in 0..SESSIONS {
        let session_id = format!("barrier_session_{ordinal}");
        let session = &sessions[&session_id];
        assert_eq!(session.state, "ready");
        assert!(session
            .messages
            .iter()
            .any(|message| message.text == format!("final-{ordinal}")));
        assert!(session.messages.iter().all(|message| {
            (0..SESSIONS).all(|other| other == ordinal || message.text != format!("final-{other}"))
        }));
        let turn = session.turns.last().unwrap();
        assert!(turn.events.iter().all(|event| {
            event.payload["session_id"]
                .as_str()
                .map(|id| id == session_id)
                .unwrap_or(true)
        }));
    }
    drop(sessions);

    assert!(
        state.semantic_delivery.baseline() > 0,
        "all sessions share one in-memory semantic delivery order"
    );
}

#[test]
fn one_session_aggregates_primary_and_subworker_state_without_cross_finishing() {
    let state = routing_test_state();
    let session_id = "session_a";
    start_web_turn(&state, session_id, "primary task").unwrap();
    let (primary_context_id, primary_worker_id) = primary_worker_scope(&state, session_id).unwrap();
    let sub_context_id = "context_session_a_sub".to_string();
    let sub_worker_id = "worker_session_a_sub".to_string();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.contexts.push(WebContext {
            context_id: sub_context_id.clone(),
            current_dir: "/work/subtask".to_string(),
            worker_ids: vec![sub_worker_id.clone()],
        });
        session.workers.push(WebWorker {
            worker_id: sub_worker_id.clone(),
            context_id: sub_context_id.clone(),
            display_name: "Subtask".to_string(),
            ordinal: 99,
            state: "ready".to_string(),
            parent_worker_id: Some(primary_worker_id.clone()),
        });
    }

    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &sub_context_id,
        &sub_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &sub_context_id,
        &sub_worker_id,
        CoreSessionWorkerEvent::TurnFinished {
            outcome: agent_core::TurnOutcome::final_response(
                "subtask done",
                agent_core::UsageStats::zero(),
                None,
                None,
                Duration::from_millis(1),
            ),
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.state, "working");
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == sub_worker_id)
            .unwrap()
            .state,
        "ready"
    );
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == primary_worker_id)
            .unwrap()
            .state,
        "working"
    );
    let turn = session.turns.last().unwrap();
    assert_eq!(turn.state, "working");
    assert!(turn.events.iter().any(|event| {
        event.source == "worker_activity"
            && event.payload["context_id"] == sub_context_id
            && event.payload["worker_id"] == sub_worker_id
    }));
}

#[test]
fn primary_turn_finish_clears_stale_working_workers_and_session_spinner() {
    let state = routing_test_state();
    let session_id = "session_a";
    start_web_turn(&state, session_id, "primary task").unwrap();
    let (primary_context_id, primary_worker_id) = primary_worker_scope(&state, session_id).unwrap();
    let sub_context_id = "context_session_a_stale_sub".to_string();
    let sub_worker_id = "worker_session_a_stale_sub".to_string();

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.contexts.push(WebContext {
            context_id: sub_context_id.clone(),
            current_dir: "/work/stale-subtask".to_string(),
            worker_ids: vec![sub_worker_id.clone()],
        });
        session.workers.push(WebWorker {
            worker_id: sub_worker_id.clone(),
            context_id: sub_context_id,
            display_name: "Stale subtask".to_string(),
            ordinal: 99,
            state: "working".to_string(),
            parent_worker_id: Some(primary_worker_id.clone()),
        });
    }

    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::TurnFinished {
            outcome: agent_core::TurnOutcome::final_response(
                "primary done",
                agent_core::UsageStats::zero(),
                None,
                None,
                Duration::from_millis(1),
            ),
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.active_turn_id, None);
    assert_eq!(session.state, "ready");
    assert!(session
        .workers
        .iter()
        .all(|worker| worker.state != "working"));
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == sub_worker_id)
            .unwrap()
            .state,
        "ready"
    );
    assert_eq!(session.turns.last().unwrap().state, "finished");
}

#[test]
fn final_answer_is_preserved_before_unconsumed_supplement_starts_a_new_turn() {
    let state = routing_test_state();
    let session_id = "session_a";
    let first = start_web_turn(&state, session_id, "Q1").unwrap();
    let (context_id, worker_id) = primary_worker_scope(&state, session_id).unwrap();

    handle_scoped_worker_event(
        &state,
        session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(CORE_TOPIC_MODEL_RESPONSE, json!({})),
            CoreSessionState::Finished,
            json!({
                "status": "ALL_FINISHED",
                "final_answer": "final_answer1",
                "continue_work": false,
                "global": {
                    "working_worker_count": 0,
                    "session_working_worker_count": 0
                }
            }),
        )
        .with_worker_scope(context_id.clone(), worker_id.clone())]),
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::UnconsumedSupplements {
            supplements: vec!["Q2".to_string()],
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &context_id,
        &worker_id,
        CoreSessionWorkerEvent::TurnFinished {
            outcome: TurnOutcome::final_response(
                "final_answer1",
                UsageStats::zero(),
                None,
                None,
                Duration::from_millis(1),
            ),
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.turns.len(), 2);
    assert_eq!(session.turns[0].turn_id, first.turn_id);
    assert_eq!(
        session.turns[0].final_answer.as_deref(),
        Some("final_answer1")
    );
    assert_eq!(session.turns[0].state, "finished");
    assert_ne!(session.turns[1].turn_id, first.turn_id);
    assert_eq!(session.turns[1].user_entries.len(), 1);
    assert_eq!(session.turns[1].user_entries[0].kind, "task");
    assert_eq!(session.turns[1].user_entries[0].text, "Q2");
    assert!(session.turns[1].final_answer.is_none());
    assert!(
        session.active_turn_id.as_deref() == Some(session.turns[1].turn_id.as_str())
            || session.pending_turn_id.as_deref() == Some(session.turns[1].turn_id.as_str()),
        "Q2 must have a distinct active or pending turn identity"
    );
    assert_eq!(
        session
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>(),
        vec!["final_answer1"],
        "starting Q2 must not replace Q1's assistant message"
    );
}

#[test]
fn stopped_primary_turn_preserves_unconsumed_supplements_without_resubmitting() {
    let state = routing_test_state();
    let session_id = "session_a";
    let supplements = vec!["follow-up one".to_string(), "follow-up two".to_string()];

    let turns_before = {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.pending_unconsumed_supplements = supplements.clone();
        session.turns.len()
    };

    let mut outcome =
        TurnOutcome::final_response("cancelled", UsageStats::zero(), None, None, Duration::ZERO);
    outcome.stop_reason = Some(agent_core::TurnStopReason::CancelledByUser);

    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::TurnFinished { outcome },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.pending_unconsumed_supplements, supplements);
    assert_eq!(session.turns.len(), turns_before);
}

#[test]
fn stopped_primary_turn_removes_its_stale_reported_working_count() {
    let state = routing_test_state();
    let session_id = "session_a";
    start_web_turn(&state, session_id, "primary task").unwrap();
    let (primary_context_id, primary_worker_id) = primary_worker_scope(&state, session_id).unwrap();

    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(CORE_TOPIC_MODEL_RESPONSE, json!({})),
            CoreSessionState::Running,
            json!({
                "status": "working",
                "continue_work": true,
                "global": {
                    "working_worker_count": 1,
                    "session_working_worker_count": 1
                }
            }),
        )
        .with_worker_scope(primary_context_id.clone(), primary_worker_id.clone())]),
    );

    let mut outcome = agent_core::TurnOutcome::final_response(
        "",
        agent_core::UsageStats::zero(),
        None,
        None,
        Duration::from_millis(1),
    );
    outcome.stop_reason = Some(agent_core::TurnStopReason::CancelledByUser);
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::TurnFinished { outcome },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.active_turn_id, None);
    assert_eq!(session.state, "ready");
    assert_eq!(session.reported_session_working_worker_count, None);
    assert!(session
        .workers
        .iter()
        .all(|worker| worker.state != "working"));
    assert_eq!(session.turns.last().unwrap().state, "finished");
}

#[test]
fn stopped_primary_turn_preserves_a_reported_active_subworker() {
    let state = routing_test_state();
    let session_id = "session_a";
    start_web_turn(&state, session_id, "primary task").unwrap();
    let (primary_context_id, primary_worker_id) = primary_worker_scope(&state, session_id).unwrap();
    let sub_worker_id = "worker_session_a_active_after_cancel".to_string();

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.workers.push(WebWorker {
            worker_id: sub_worker_id.clone(),
            context_id: "context_session_a_active_after_cancel".to_string(),
            display_name: "Active subtask".to_string(),
            ordinal: 99,
            state: "working".to_string(),
            parent_worker_id: Some(primary_worker_id.clone()),
        });
    }

    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(CORE_TOPIC_MODEL_RESPONSE, json!({})),
            CoreSessionState::Running,
            json!({
                "status": "working",
                "continue_work": true,
                "global": {
                    "working_worker_count": 2,
                    "session_working_worker_count": 2
                }
            }),
        )
        .with_worker_scope(primary_context_id.clone(), primary_worker_id.clone())]),
    );

    let mut outcome = agent_core::TurnOutcome::final_response(
        "",
        agent_core::UsageStats::zero(),
        None,
        None,
        Duration::from_millis(1),
    );
    outcome.stop_reason = Some(agent_core::TurnStopReason::CancelledByUser);
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::TurnFinished { outcome },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.active_turn_id, None);
    assert_eq!(session.state, "working");
    assert_eq!(session.reported_session_working_worker_count, None);
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == primary_worker_id)
            .unwrap()
            .state,
        "ready"
    );
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == sub_worker_id)
            .unwrap()
            .state,
        "working"
    );
}

#[test]
fn primary_turn_finish_preserves_explicitly_reported_active_subworker() {
    let state = routing_test_state();
    let session_id = "session_a";
    start_web_turn(&state, session_id, "primary task").unwrap();
    let (primary_context_id, primary_worker_id) = primary_worker_scope(&state, session_id).unwrap();
    let sub_context_id = "context_session_a_active_sub".to_string();
    let sub_worker_id = "worker_session_a_active_sub".to_string();

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.contexts.push(WebContext {
            context_id: sub_context_id.clone(),
            current_dir: "/work/active-subtask".to_string(),
            worker_ids: vec![sub_worker_id.clone()],
        });
        session.workers.push(WebWorker {
            worker_id: sub_worker_id.clone(),
            context_id: sub_context_id,
            display_name: "Active subtask".to_string(),
            ordinal: 99,
            state: "working".to_string(),
            parent_worker_id: Some(primary_worker_id.clone()),
        });
    }

    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(CORE_TOPIC_MODEL_RESPONSE, json!({})),
            CoreSessionState::Finished,
            json!({
                "status": "ALL_FINISHED",
                "final_answer": "primary done",
                "continue_work": false,
                "global": {
                    "working_worker_count": 1,
                    "session_working_worker_count": 1
                }
            }),
        )
        .with_worker_scope(primary_context_id.clone(), primary_worker_id.clone())]),
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::TurnFinished {
            outcome: agent_core::TurnOutcome::final_response(
                "primary done",
                agent_core::UsageStats::zero(),
                None,
                None,
                Duration::from_millis(1),
            ),
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.active_turn_id, None);
    assert_eq!(session.state, "working");
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == primary_worker_id)
            .unwrap()
            .state,
        "ready"
    );
    assert_eq!(
        session
            .workers
            .iter()
            .find(|worker| worker.worker_id == sub_worker_id)
            .unwrap()
            .state,
        "working"
    );
}

#[test]
fn primary_turn_finish_ignores_other_sessions_global_worker_count() {
    let state = routing_test_state();
    let session_id = "session_a";
    start_web_turn(&state, session_id, "primary task").unwrap();
    let (primary_context_id, primary_worker_id) = primary_worker_scope(&state, session_id).unwrap();

    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: String::new(),
            interaction_profile: None,
            interaction_request: None,
            api_payload: None,
        },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(CORE_TOPIC_MODEL_RESPONSE, json!({})),
            CoreSessionState::Finished,
            json!({
                "status": "ALL_FINISHED",
                "final_answer": "primary done",
                "continue_work": false,
                "global": {
                    "working_worker_count": 1,
                    "session_working_worker_count": 0
                }
            }),
        )
        .with_worker_scope(primary_context_id.clone(), primary_worker_id.clone())]),
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &primary_context_id,
        &primary_worker_id,
        CoreSessionWorkerEvent::TurnFinished {
            outcome: agent_core::TurnOutcome::final_response(
                "primary done",
                agent_core::UsageStats::zero(),
                None,
                None,
                Duration::from_millis(1),
            ),
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.active_turn_id, None);
    assert_eq!(session.state, "ready");
    assert!(session
        .workers
        .iter()
        .all(|worker| worker.state != "working"));
}

#[test]
fn child_context_worker_uses_its_owning_sessions_runtime_profile_and_env() {
    let state = routing_test_state();
    let session_id = "session_a";
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.runtime.settings.config.model = "session-owned-model".to_string();
        session
            .runtime
            .env
            .insert("SESSION_MARKER".to_string(), "owned".to_string());
        session.contexts.clear();
        session.workers.clear();
        session.active_context_id.clear();
        session.primary_worker_id.clear();
    }

    let primary_dir = std::env::temp_dir().join(unique_web_id("web_primary_context"));
    std::fs::create_dir_all(&primary_dir).unwrap();
    let (primary_context_id, parent_worker_id) = create_context_with_worker(
        &state,
        session_id,
        primary_dir,
        Some("Primary worker".to_string()),
        None,
        true,
    )
    .unwrap();

    let subtask_dir = std::env::temp_dir().join(unique_web_id("web_subtask_context"));
    std::fs::create_dir_all(&subtask_dir).unwrap();
    let (context_id, worker_id) = create_context_with_worker(
        &state,
        session_id,
        subtask_dir,
        Some("Subtask worker".to_string()),
        Some(parent_worker_id.clone()),
        false,
    )
    .unwrap();
    relay_topic_reply_to_requesting_worker(
        &state,
        session_id,
        Some(&worker_id),
        TopicReply::new(
            session_id,
            "core.test.child.request",
            HostDecision::Accept,
            json!({ "source": "primary_chat" }),
        ),
    )
    .unwrap();
    assert_eq!(
        relay_topic_reply_to_requesting_worker(
            &state,
            "session_b",
            Some(&worker_id),
            TopicReply::new(
                "session_b",
                "core.test.child.request",
                HostDecision::Accept,
                json!({}),
            ),
        )
        .err()
        .as_deref(),
        Some("session_worker_scope_mismatch")
    );
    let event = wait_for_web_worker_event(&state, &worker_id, "child lifecycle");
    let CoreSessionWorkerEvent::Topics(topics) = event else {
        panic!("expected child lifecycle topic");
    };
    let lifecycle = topics[0].as_lifecycle().unwrap();
    assert_eq!(lifecycle.profile.model, "session-owned-model");
    assert_eq!(
        lifecycle
            .workspace
            .unwrap()
            .env
            .get("SESSION_MARKER")
            .map(String::as_str),
        Some("owned")
    );
    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.contexts.len(), 2);
    assert_eq!(session.workers.len(), 2);
    assert_eq!(session.active_context_id, primary_context_id);
    let worker = session
        .workers
        .iter()
        .find(|worker| worker.worker_id == worker_id)
        .unwrap();
    assert_eq!(worker.context_id, context_id);
    assert_eq!(
        worker.parent_worker_id.as_deref(),
        Some(parent_worker_id.as_str())
    );
    drop(sessions);
    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
}

#[test]
fn web_runtime_shutdown_stops_all_session_workers() {
    let state = routing_test_state();
    let session_id = "session_a";
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.contexts.clear();
        session.workers.clear();
        session.active_context_id.clear();
        session.primary_worker_id.clear();
    }

    let primary_dir = std::env::temp_dir().join(unique_web_id("web_shutdown_primary"));
    let child_dir = std::env::temp_dir().join(unique_web_id("web_shutdown_child"));
    std::fs::create_dir_all(&primary_dir).unwrap();
    std::fs::create_dir_all(&child_dir).unwrap();
    let (_primary_context_id, primary_worker_id) = create_context_with_worker(
        &state,
        session_id,
        primary_dir,
        Some("Primary worker".to_string()),
        None,
        true,
    )
    .unwrap();
    let (_child_context_id, _child_worker_id) = create_context_with_worker(
        &state,
        session_id,
        child_dir,
        Some("Child worker".to_string()),
        Some(primary_worker_id),
        false,
    )
    .unwrap();

    let memory_dir = state.mem.lock().unwrap().layout.memory_dir();
    let tool_dir = memory_dir.join("shutdown-tool");
    std::fs::create_dir_all(&tool_dir).unwrap();
    let script = tool_dir.join("sleep.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\npython3 -c 'import time; time.sleep(30)'\n",
    )
    .unwrap();
    let tool_jobs = FileToolJobStore::new(&memory_dir);
    let started = tool_jobs.spawn("shutdown_sleep", &script, &serde_json::json!({"args": {}}));
    let job_id = started
        .lines()
        .find_map(|line| line.strip_prefix("job_id: "))
        .unwrap();
    assert_eq!(state.manager.lock().unwrap().worker_count(), 2);
    shutdown_web_runtime(&state).unwrap();
    assert_eq!(state.manager.lock().unwrap().worker_count(), 0);
    assert!(tool_jobs.status(job_id, 0).contains("state: cancelled"));
}

#[test]
fn web_runtime_cleanup_guard_runs_cleanup_once_on_scope_exit() {
    let state = routing_test_state();
    {
        let _guard = WebRuntimeCleanupGuard::new(&state);
    }

    assert_eq!(state.manager.lock().unwrap().worker_count(), 0);
}

struct CancellableMemSwitchModel {
    entered: Arc<AtomicUsize>,
}

impl ModelClient for CancellableMemSwitchModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        while !should_cancel() {
            if started.elapsed() > Duration::from_secs(5) {
                return Err("mem_switch_test_timeout".to_string());
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err("cancelled_for_mem_switch".to_string())
    }
}

fn register_cancellable_mem_switch_worker(
    state: &AppState,
    entered: Arc<AtomicUsize>,
    worker_dir: &Path,
) -> String {
    let session_id = unique_web_id("mem_switch_session");
    let context_id = test_context_id(&session_id);
    let core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        worker_dir,
    );
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            state.template.settings.lock().unwrap().config.clone(),
            CoreSessionWorkerWorkspace::new(
                worker_dir,
                worker_dir.join("audit.json"),
                "test-web",
                "local",
            ),
            session_id.clone(),
            context_id.clone(),
            Some("MEM switch worker".to_string()),
            None,
            CancellableMemSwitchModel { entered },
        )
        .unwrap();
    let mut session = test_web_session(&session_id, 0, "MEM switch worker".to_string());
    session.current_dir = worker_dir.display().to_string();
    session.contexts[0] = WebContext {
        context_id: context_id.clone(),
        current_dir: worker_dir.display().to_string(),
        worker_ids: vec![worker_id.clone()],
    };
    session.workers[0].worker_id = worker_id.clone();
    session.workers[0].context_id = context_id.clone();
    session.active_context_id = context_id;
    session.primary_worker_id = worker_id;
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);
    persist_web_session(state, &session_id).unwrap();
    session_id
}

struct BlockingShutdownModel {
    entered: Arc<AtomicUsize>,
}

impl ModelClient for BlockingShutdownModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_secs(2));
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content: confirmed_xml_response("<final_answer>late</final_answer>"),
            model_name: "test-model".to_string(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    }
}

#[test]
fn web_runtime_shutdown_detaches_active_workers_so_ctrl_c_can_exit() {
    let state = routing_test_state();
    let dir = std::env::temp_dir().join(unique_web_id("web_shutdown_active_worker"));
    std::fs::create_dir_all(&dir).unwrap();
    let entered = Arc::new(AtomicUsize::new(0));
    let core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        &dir,
    );
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            state.template.settings.lock().unwrap().config.clone(),
            CoreSessionWorkerWorkspace::new(&dir, dir.join("audit.json"), "test-web", "local"),
            "session_a",
            "context_active_shutdown",
            Some("Active".to_string()),
            None,
            BlockingShutdownModel {
                entered: Arc::clone(&entered),
            },
        )
        .unwrap();
    state
        .manager
        .lock()
        .unwrap()
        .handle(&worker_id)
        .unwrap()
        .run_turn("block", None)
        .unwrap();
    let started = Instant::now();
    while entered.load(Ordering::SeqCst) == 0 {
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }

    let shutdown_started = Instant::now();
    shutdown_web_runtime(&state).unwrap();
    assert!(
        shutdown_started.elapsed() < Duration::from_millis(250),
        "web Ctrl+C shutdown should not wait for an active model call to finish"
    );
    assert_eq!(state.manager.lock().unwrap().worker_count(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

struct CancelThenFinishModel {
    calls: Arc<AtomicUsize>,
    entered: Arc<AtomicUsize>,
    final_text: &'static str,
}

impl ModelClient for CancelThenFinishModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            while !should_cancel() {
                thread::sleep(Duration::from_millis(5));
            }
            return Err("cancelled_by_user".to_string());
        }
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content: confirmed_xml_response(&format!(
                "<final_answer>{}</final_answer>",
                self.final_text
            )),
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                total_tokens: 12,
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

#[test]
fn stop_then_immediate_submit_is_visible_once_and_runs_after_the_cancelled_turn() {
    let state = routing_test_state();
    let session_id = "session_a";
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicUsize::new(0));
    let context_id = "immediate_next_context".to_string();
    let worker_dir = std::env::temp_dir().join(unique_web_id("immediate_next_worker"));
    std::fs::create_dir_all(&worker_dir).unwrap();
    let core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        &worker_dir,
    );
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            state.template.settings.lock().unwrap().config.clone(),
            CoreSessionWorkerWorkspace::new(
                &worker_dir,
                worker_dir.join("audit.json"),
                "test-web",
                "local",
            ),
            session_id,
            context_id.clone(),
            Some("Primary".to_string()),
            None,
            CancelThenFinishModel {
                calls: Arc::clone(&calls),
                entered: Arc::clone(&entered),
                final_text: "NEXT_TURN_FINISHED",
            },
        )
        .unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.contexts = vec![WebContext {
            context_id: context_id.clone(),
            current_dir: worker_dir.display().to_string(),
            worker_ids: vec![worker_id.clone()],
        }];
        session.workers = vec![WebWorker {
            worker_id: worker_id.clone(),
            context_id: context_id.clone(),
            display_name: "Primary".to_string(),
            ordinal: 0,
            state: "ready".to_string(),
            parent_worker_id: None,
        }];
        session.active_context_id = context_id;
        session.primary_worker_id = worker_id.clone();
        session.current_dir = worker_dir.display().to_string();
    }

    handle_command_with_id(
        &state,
        TEST_PORT,
        Some("first-command"),
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "first turn blocks until Stop".to_string(),
            attachment_ids: None,
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap();
    let started = Instant::now();
    while entered.load(Ordering::SeqCst) == 0 {
        for (event_session_id, event_context_id, event_worker_id, event) in
            drain_worker_events(&state)
        {
            handle_scoped_worker_event(
                &state,
                &event_session_id,
                &event_context_id,
                &event_worker_id,
                event,
            );
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.to_string(),
            target_command_id: Some("first-command".to_string()),
        },
    )
    .unwrap();
    let first_reply = handle_command_with_id(
        &state,
        TEST_PORT,
        Some("next-command"),
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "second turn must appear immediately".to_string(),
            attachment_ids: None,
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("second command must be visible before cancellation settles");
    let WireEvent::TurnUpdated {
        turn: visible_next, ..
    } = first_reply
    else {
        panic!("second command must return a turn update")
    };
    assert_eq!(visible_next.state, "pending");
    assert_eq!(
        visible_next.user_entries[0].text,
        "second turn must appear immediately"
    );
    assert_eq!(load_next_turn_intents(&state, session_id).unwrap().len(), 1);
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions
            .get_mut(session_id)
            .unwrap()
            .attachments
            .push(WebAttachment {
                id: "duplicate-must-not-consume".to_string(),
                name: "duplicate.txt".to_string(),
                path: "/tmp/duplicate.txt".to_string(),
                bytes: 9,
            });
    }

    let duplicate_reply = handle_command_with_id(
        &state,
        TEST_PORT,
        Some("next-command"),
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "second turn must appear immediately".to_string(),
            attachment_ids: Some(vec!["duplicate-must-not-consume".to_string()]),
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("duplicate delivery must return the original visible turn");
    let WireEvent::TurnUpdated {
        turn: duplicate_turn,
        ..
    } = duplicate_reply
    else {
        panic!("duplicate delivery must remain a turn update")
    };
    assert_eq!(duplicate_turn.turn_id, visible_next.turn_id);
    {
        let sessions = state.sessions.lock().unwrap();
        let session = sessions.get(session_id).unwrap();
        assert_eq!(session.next_turn_intents.len(), 1);
        assert!(session
            .attachments
            .iter()
            .any(|file| file.id == "duplicate-must-not-consume"));
        assert_eq!(
            session
                .turns
                .iter()
                .filter(|turn| {
                    turn.user_entries
                        .iter()
                        .any(|entry| entry.command_id.as_deref() == Some("next-command"))
                })
                .count(),
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    let finished = Instant::now();
    loop {
        for (event_session_id, event_context_id, event_worker_id, event) in
            drain_worker_events(&state)
        {
            handle_scoped_worker_event(
                &state,
                &event_session_id,
                &event_context_id,
                &event_worker_id,
                event,
            );
        }
        let done = {
            let sessions = state.sessions.lock().unwrap();
            let session = sessions.get(session_id).unwrap();
            calls.load(Ordering::SeqCst) == 2
                && session.next_turn_intents.is_empty()
                && session
                    .turns
                    .iter()
                    .find(|turn| turn.turn_id == visible_next.turn_id)
                    .is_some_and(|turn| turn.state == "finished")
        };
        if done {
            break;
        }
        assert!(finished.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(load_next_turn_intents(&state, session_id)
        .unwrap()
        .is_empty());

    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
    let _ = std::fs::remove_dir_all(worker_dir);
}

#[test]
fn cancel_stops_all_session_workers_and_next_turn_runs_only_primary() {
    let state = routing_test_state();
    let session_id = "session_a";
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let primary_entered = Arc::new(AtomicUsize::new(0));
    let child_calls = Arc::new(AtomicUsize::new(0));
    let child_entered = Arc::new(AtomicUsize::new(0));
    let mut worker_specs = Vec::new();

    for (index, (calls, entered, final_text)) in [
        (
            Arc::clone(&primary_calls),
            Arc::clone(&primary_entered),
            "PRIMARY_CONTINUED",
        ),
        (
            Arc::clone(&child_calls),
            Arc::clone(&child_entered),
            "CHILD_SHOULD_NOT_CONTINUE",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let context_id = format!("cancel_context_{index}");
        let worker_dir = std::env::temp_dir().join(unique_web_id("cancel_worker"));
        std::fs::create_dir_all(&worker_dir).unwrap();
        let core = AgentCore::new(
            STATIC_PROMPT,
            CoreProfile {
                model: "test-model".to_string(),
            },
            &worker_dir,
        );
        let parent_worker_id = worker_specs
            .first()
            .map(|(_, worker_id, _): &(String, String, String)| worker_id.clone());
        let worker_id = state
            .manager
            .lock()
            .unwrap()
            .spawn_worker_in_session_with_model_client(
                core,
                state.template.settings.lock().unwrap().config.clone(),
                CoreSessionWorkerWorkspace::new(
                    &worker_dir,
                    worker_dir.join("audit.json"),
                    "test-web",
                    "local",
                ),
                session_id,
                context_id.clone(),
                Some(if index == 0 { "Primary" } else { "Child" }.to_string()),
                parent_worker_id,
                CancelThenFinishModel {
                    calls,
                    entered,
                    final_text,
                },
            )
            .unwrap();
        worker_specs.push((context_id, worker_id, worker_dir.display().to_string()));
    }
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(session_id).unwrap();
        session.contexts = worker_specs
            .iter()
            .map(|(context_id, worker_id, current_dir)| WebContext {
                context_id: context_id.clone(),
                current_dir: current_dir.clone(),
                worker_ids: vec![worker_id.clone()],
            })
            .collect();
        session.workers = worker_specs
            .iter()
            .enumerate()
            .map(|(index, (context_id, worker_id, _))| WebWorker {
                worker_id: worker_id.clone(),
                context_id: context_id.clone(),
                display_name: if index == 0 { "Primary" } else { "Child" }.to_string(),
                ordinal: index as u32,
                state: "ready".to_string(),
                parent_worker_id: (index == 1).then(|| worker_specs[0].1.clone()),
            })
            .collect();
        session.active_context_id = worker_specs[0].0.clone();
        session.primary_worker_id = worker_specs[0].1.clone();
        session.current_dir = worker_specs[0].2.clone();
    }
    for (_, worker_id, _) in &worker_specs {
        state
            .manager
            .lock()
            .unwrap()
            .handle(worker_id)
            .unwrap()
            .run_turn("start", None)
            .unwrap();
    }
    let started = Instant::now();
    while primary_entered.load(Ordering::SeqCst) == 0 || child_entered.load(Ordering::SeqCst) == 0 {
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.to_string(),
            target_command_id: None,
        },
    )
    .unwrap();
    let cancelled = Instant::now();
    while state.manager.lock().unwrap().working_worker_count() != 0 {
        for (event_session_id, context_id, worker_id, event) in drain_worker_events(&state) {
            handle_scoped_worker_event(&state, &event_session_id, &context_id, &worker_id, event);
        }
        assert!(cancelled.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(child_calls.load(Ordering::SeqCst), 1);

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "continue".to_string(),
            input_kind: None,
            source_turn_id: None,
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap();
    let continued = Instant::now();
    while primary_calls.load(Ordering::SeqCst) < 2 {
        assert!(continued.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(30));
    assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
    assert_eq!(child_calls.load(Ordering::SeqCst), 1);

    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
}

#[test]
fn five_agent_topic_burst_stays_isolated_and_bounded() {
    const AGENTS: usize = 5;
    const RESPONSES_PER_AGENT: usize = 240;
    let state = routing_test_state();
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.clear();
        for ordinal in 0..AGENTS {
            let session_id = format!("stress_{ordinal}");
            sessions.insert(
                session_id.clone(),
                test_web_session(&session_id, ordinal as u32, format!("Stress {ordinal}")),
            );
        }
    }
    let workers = (0..AGENTS)
        .map(|ordinal| {
            let state = state.clone();
            thread::spawn(move || {
                let session_id = format!("stress_{ordinal}");
                for index in 0..RESPONSES_PER_AGENT {
                    handle_worker_event(
                        &state,
                        &session_id,
                        CoreSessionWorkerEvent::Topics(vec![final_response_topic(
                            &session_id,
                            format!("{session_id}:response:{index}"),
                        )]),
                    );
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let sessions = state.sessions.lock().unwrap();
    for ordinal in 0..AGENTS {
        let session_id = format!("stress_{ordinal}");
        let messages = &sessions[&session_id].messages;
        assert_eq!(messages.len(), RESPONSES_PER_AGENT);
        assert!(messages
            .iter()
            .all(|message| message.text.starts_with(&format!("{session_id}:"))));
    }
}

#[test]
fn concurrent_cancel_supplement_and_final_are_isolated_by_session() {
    let state = routing_test_state();
    let cancel_session = register_real_worker(&state, "CONCURRENT_CANCEL");
    let supplement_session = register_real_worker(&state, "CONCURRENT_SUPPLEMENT");
    let final_session = register_real_worker(&state, "CONCURRENT_FINAL");
    let cancel_turn = start_web_turn(&state, &cancel_session, "cancel only this session").unwrap();
    let supplement_turn =
        start_web_turn(&state, &supplement_session, "supplement only this session").unwrap();
    let final_turn = start_web_turn(&state, &final_session, "finish only this session").unwrap();
    let (final_context_id, final_worker_id) = primary_worker_scope(&state, &final_session).unwrap();

    let barrier = Arc::new(std::sync::Barrier::new(3));
    let cancel_thread = {
        let state = state.clone();
        let barrier = Arc::clone(&barrier);
        let session_id = cancel_session.clone();
        thread::spawn(move || {
            barrier.wait();
            handle_command(
                &state,
                TEST_PORT,
                ClientCommand::TurnCancel {
                    session_id,
                    target_command_id: None,
                },
            )
            .unwrap();
        })
    };
    let supplement_thread = {
        let state = state.clone();
        let barrier = Arc::clone(&barrier);
        let session_id = supplement_session.clone();
        thread::spawn(move || {
            barrier.wait();
            append_turn_supplement_with_pending_attachments(
                &state,
                &session_id,
                "only session B sees this".to_string(),
                None,
            )
            .unwrap();
        })
    };
    let final_thread = {
        let state = state.clone();
        let barrier = Arc::clone(&barrier);
        let session_id = final_session.clone();
        let context_id = final_context_id.clone();
        let worker_id = final_worker_id.clone();
        thread::spawn(move || {
            barrier.wait();
            handle_scoped_worker_event(
                &state,
                &session_id,
                &context_id,
                &worker_id,
                CoreSessionWorkerEvent::Topics(vec![final_response_topic(
                    &session_id,
                    "only session C sees this".to_string(),
                )
                .with_worker_scope(context_id.clone(), worker_id.clone())]),
            );
            handle_scoped_worker_event(
                &state,
                &session_id,
                &context_id,
                &worker_id,
                CoreSessionWorkerEvent::TurnFinished {
                    outcome: TurnOutcome::final_response(
                        "only session C sees this",
                        UsageStats::zero(),
                        None,
                        None,
                        Duration::ZERO,
                    ),
                },
            );
        })
    };
    cancel_thread.join().unwrap();
    supplement_thread.join().unwrap();
    final_thread.join().unwrap();

    let sessions = state.sessions.lock().unwrap();
    let cancelled = &sessions[&cancel_session];
    assert_eq!(
        cancelled.active_turn_id.as_deref(),
        Some(cancel_turn.turn_id.as_str())
    );
    assert!(cancelled
        .messages
        .iter()
        .all(|message| message.text != "only session C sees this"));
    let supplemented = &sessions[&supplement_session];
    assert_eq!(
        supplemented.active_turn_id.as_deref(),
        Some(supplement_turn.turn_id.as_str())
    );
    assert_eq!(
        supplemented
            .turns
            .last()
            .unwrap()
            .user_entries
            .last()
            .unwrap()
            .text,
        "only session B sees this"
    );
    assert!(supplemented
        .messages
        .iter()
        .all(|message| message.text != "only session C sees this"));
    let finished = &sessions[&final_session];
    assert!(finished.active_turn_id.is_none());
    assert_eq!(finished.turns.last().unwrap().turn_id, final_turn.turn_id);
    assert_eq!(
        finished.turns.last().unwrap().final_answer.as_deref(),
        Some("only session C sees this")
    );
    assert!(finished
        .turns
        .last()
        .unwrap()
        .user_entries
        .iter()
        .all(|entry| entry.text != "only session B sees this"));
    drop(sessions);

    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
}

#[test]
fn eight_working_sessions_keep_mixed_cancel_supplement_final_and_request_scoped() {
    const SESSION_COUNT: usize = 8;
    let state = routing_test_state();
    let sessions = (0..SESSION_COUNT)
        .map(|ordinal| {
            let session_id = register_real_worker(
                &state,
                Box::leak(format!("MIXED_{ordinal}").into_boxed_str()),
            );
            let turn = start_web_turn(&state, &session_id, &format!("task-{ordinal}")).unwrap();
            let (context_id, worker_id) = primary_worker_scope(&state, &session_id).unwrap();
            (session_id, turn.turn_id, context_id, worker_id)
        })
        .collect::<Vec<_>>();
    let barrier = Arc::new(std::sync::Barrier::new(SESSION_COUNT));
    let workers = sessions
        .iter()
        .cloned()
        .enumerate()
        .map(|(ordinal, (session_id, _, context_id, worker_id))| {
            let state = state.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                match ordinal % 4 {
                    0 => {
                        handle_command(
                            &state,
                            TEST_PORT,
                            ClientCommand::TurnCancel {
                                session_id,
                                target_command_id: None,
                            },
                        )
                        .unwrap();
                    }
                    1 => {
                        append_turn_supplement_with_pending_attachments(
                            &state,
                            &session_id,
                            format!("supplement-{ordinal}"),
                            None,
                        )
                        .unwrap();
                    }
                    2 => {
                        let answer = format!("final-{ordinal}");
                        handle_scoped_worker_event(
                            &state,
                            &session_id,
                            &context_id,
                            &worker_id,
                            CoreSessionWorkerEvent::Topics(vec![final_response_topic(
                                &session_id,
                                answer.clone(),
                            )
                            .with_worker_scope(context_id.clone(), worker_id.clone())]),
                        );
                        handle_scoped_worker_event(
                            &state,
                            &session_id,
                            &context_id,
                            &worker_id,
                            CoreSessionWorkerEvent::TurnFinished {
                                outcome: TurnOutcome::final_response(
                                    answer,
                                    UsageStats::zero(),
                                    None,
                                    None,
                                    Duration::ZERO,
                                ),
                            },
                        );
                    }
                    _ => {
                        let request = CoreTopicEvent::new(
                            session_id.clone(),
                            CoreTopic::new(CORE_TOPIC_USER_APPROVAL_REQUEST, json!({})),
                            CoreSessionState::Running,
                            json!({
                                "request_id": format!("request-{ordinal}"),
                                "summary": format!("approval-{ordinal}"),
                            }),
                        )
                        .with_worker_scope(context_id.clone(), worker_id.clone());
                        handle_scoped_worker_event(
                            &state,
                            &session_id,
                            &context_id,
                            &worker_id,
                            CoreSessionWorkerEvent::Topics(vec![request]),
                        );
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let stored = state.sessions.lock().unwrap();
    for (ordinal, (session_id, turn_id, _, _)) in sessions.iter().enumerate() {
        let session = &stored[session_id];
        let turn = session
            .turns
            .iter()
            .find(|turn| turn.turn_id == *turn_id)
            .unwrap();
        match ordinal % 4 {
            0 => assert!(turn.final_answer.is_none()),
            1 => assert_eq!(
                turn.user_entries.last().unwrap().text,
                format!("supplement-{ordinal}")
            ),
            2 => {
                assert_eq!(
                    turn.final_answer.as_deref(),
                    Some(format!("final-{ordinal}").as_str())
                );
                assert!(session.active_turn_id.is_none());
            }
            _ => assert!(turn.events.iter().any(|event| {
                let wire = event.payload.to_string();
                wire.contains(CORE_TOPIC_USER_APPROVAL_REQUEST)
                    && wire.contains(&format!("request-{ordinal}"))
            })),
        }
        for other in 0..SESSION_COUNT {
            if other != ordinal {
                assert!(turn
                    .user_entries
                    .iter()
                    .all(|entry| entry.text != format!("supplement-{other}")));
                assert_ne!(
                    turn.final_answer.as_deref(),
                    Some(format!("final-{other}").as_str())
                );
            }
        }
    }
    drop(stored);
    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
}

#[test]
fn deleting_idle_or_host_working_session_does_not_remove_another_sessions_worker() {
    let state = routing_test_state();
    let idle_session = register_real_worker(&state, "DELETE_IDLE");
    let working_session = register_real_worker(&state, "DELETE_WORKING");
    let survivor_session = register_real_worker(&state, "DELETE_SURVIVOR");
    let survivor_worker = state.sessions.lock().unwrap()[&survivor_session]
        .primary_worker_id
        .clone();
    start_web_turn(&state, &working_session, "host has an active turn").unwrap();
    for session_id in [&idle_session, &working_session, &survivor_session] {
        persist_web_session(&state, session_id).unwrap();
    }

    for session_id in [idle_session, working_session] {
        let event = handle_command(
            &state,
            TEST_PORT,
            ClientCommand::SessionDelete {
                session_id: session_id.clone(),
            },
        )
        .unwrap();
        assert!(matches!(
            event,
            Some(WireEvent::SessionDeleted { session_id: deleted }) if deleted == session_id
        ));
        assert!(!state.sessions.lock().unwrap().contains_key(&session_id));
        assert!(state
            .manager
            .lock()
            .unwrap()
            .handle(&survivor_worker)
            .is_some());
    }

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::SessionRename {
            session_id: survivor_session.clone(),
            display_name: "survivor still routable".to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        state.sessions.lock().unwrap()[&survivor_session].display_name,
        "survivor still routable"
    );

    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
}

#[test]
fn host_chat_history_drops_only_the_oldest_entries_at_its_memory_bound() {
    let state = routing_test_state();
    for index in 0..(MAX_SESSION_MESSAGES + 25) {
        append_message(&state, "session_a", "assistant", format!("message-{index}")).unwrap();
    }
    let sessions = state.sessions.lock().unwrap();
    let messages = &sessions["session_a"].messages;
    assert_eq!(messages.len(), MAX_SESSION_MESSAGES);
    assert_eq!(messages.first().unwrap().text, "message-25");
    assert_eq!(messages.last().unwrap().text, "message-2024");
}

#[test]
fn mismatched_core_topic_is_not_forwarded_or_written_to_another_agent() {
    let state = routing_test_state();
    let mut events = state.events.subscribe();
    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::Topics(vec![final_response_topic(
            "session_b",
            "must not leak".to_string(),
        )]),
    );
    let sessions = state.sessions.lock().unwrap();
    assert!(sessions["session_a"].messages.is_empty());
    assert!(sessions["session_b"].messages.is_empty());
    drop(sessions);

    let events = drain_wire_events(&mut events);
    assert!(events
        .iter()
        .all(|event| !matches!(event, WireEvent::CoreTopic { .. })));
    assert!(events.iter().any(|event| matches!(
        event,
        WireEvent::WorkerActivity { session_id, event, .. }
            if session_id == "session_a" && event["kind"] == "topic_scope_mismatch"
    )));
}

#[test]
fn debug_worker_event_pipeline_persists_native_dumps_metrics_and_repair_history() {
    let mut state = routing_test_state();
    let debug = Arc::new(DebugStore::create().unwrap());
    let debug_root = debug.root().to_path_buf();
    state.debug = Some(debug);

    let session_id = "session_a";
    let turn = start_web_turn(&state, session_id, "repair malformed XML").unwrap();
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 1,
            prompt: "debug prompt".to_string(),
            interaction_profile: Some(agent_core::InteractionProfile {
                api_protocol: "openai_compatible".to_string(),
                model: "qwen-plus".to_string(),
                gateway: "https://gateway.example.test/v1".to_string(),
                requested_mode: agent_core::ToolCallMode::Auto,
                resolved_mode: agent_core::ToolCallMode::Native,
                active_prompt_protocol: "json".to_string(),
                parallel_supported: true,
                parallel_enabled: true,
                source: agent_core::CapabilityProbeSource::Probe,
                reason: "native_and_parallel_probe_succeeded".to_string(),
                probe_latency_ms: Some(10),
                observed_tool_calls: 2,
            }),
            interaction_request: Some(Box::new(agent_core::ModelInteractionRequest {
                rendered_prompt: "debug prompt".to_string(),
                static_tool_count: 1,
                tools: vec![agent_core::ToolDefinition {
                    name: "self_tool".to_string(),
                    description: "Inspect runtime state".to_string(),
                    input_schema: json!({"type": "object"}),
                }],
                native_exchanges: vec![agent_core::NativeExchange {
                    delta_id: "pd_1".to_string(),
                    assistant_text: "previous".to_string(),
                    calls: vec![agent_core::NativeToolCall {
                        id: "call_previous".to_string(),
                        name: "self_tool".to_string(),
                        arguments: json!({"type": "cwd"}),
                        raw_arguments: r#"{"type":"cwd"}"#.to_string(),
                    }],
                    results: vec![agent_core::NativeToolResult {
                        call_id: "call_previous".to_string(),
                        name: "self_tool".to_string(),
                        content: "CWD: /work".to_string(),
                        is_error: false,
                    }],
                }],
                resolved_mode: agent_core::ToolCallMode::Native,
                parallel_tool_calls: true,
                tool_choice: agent_core::NativeToolChoice::Auto,
            })),
            api_payload: Some(Box::new(json!({
                "messages": [
                    {"role": "user", "content": "debug prompt"},
                    {"role": "assistant", "content": "previous", "tool_calls": [{"id": "call_previous", "type": "function", "function": {"name": "self_tool", "arguments": "{\"type\":\"cwd\"}"}}]},
                    {"role": "tool", "tool_call_id": "call_previous", "content": "CWD: /work"}
                ]
            }))),
        },
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::ModelRequestCompleted {
            latency: Duration::from_millis(425),
        },
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::ModelResponseParsed { tool_count: 1 },
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::ModelResponse {
            round: 1,
            usage: UsageStats::zero(),
            content: "I will inspect the workspace.".to_string(),
            tool_calls: vec![agent_core::NativeToolCall {
                id: "call_current".to_string(),
                name: "self_tool".to_string(),
                arguments: json!({"type": "cwd"}),
                raw_arguments: r#"{"type":"cwd"}"#.to_string(),
            }],
            truncated: true,
            runtime_phase: None,
        },
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(CORE_TOPIC_ACTION, json!({})),
            CoreSessionState::Running,
            json!({
                "event": "finish",
                "cpu_time_available": true,
                "cpu_time_ns": 25_000_000_u64,
            }),
        )]),
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::Topics(vec![CoreTopicEvent::new(
            session_id,
            CoreTopic::new(
                CORE_TOPIC_MODEL_REPAIR,
                json!({ "name": CORE_TOPIC_MODEL_REPAIR }),
            ),
            CoreSessionState::WaitingModel,
            json!({
                "issue": "xml_response_root_missing",
                "reason": "required response root missing",
                "attempt": 1,
                "max_attempts": 3,
            }),
        )]),
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::Topics(vec![agent_core::runtime_root_repair_help_topic_event(
            session_id,
        )]),
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::ModelRequest {
            round: 2,
            prompt: "inline retry prompt".to_string(),
            interaction_profile: Some(agent_core::InteractionProfile {
                api_protocol: "anthropic".to_string(),
                model: "claude-inline".to_string(),
                gateway: "https://inline.example/v1".to_string(),
                requested_mode: agent_core::ToolCallMode::Inline,
                resolved_mode: agent_core::ToolCallMode::Inline,
                active_prompt_protocol: "xml".to_string(),
                parallel_supported: false,
                parallel_enabled: false,
                source: agent_core::CapabilityProbeSource::Explicit,
                reason: "explicit_inline".to_string(),
                probe_latency_ms: None,
                observed_tool_calls: 0,
            }),
            interaction_request: Some(Box::new(agent_core::ModelInteractionRequest::inline(
                "inline retry prompt",
            ))),
            api_payload: Some(Box::new(json!({
                "system": "inline system",
                "messages": [{"role": "user", "content": "inline retry prompt"}]
            }))),
        },
    );
    handle_worker_event(
        &state,
        session_id,
        CoreSessionWorkerEvent::ModelError {
            error: "synthetic terminal failure".to_string(),
        },
    );

    let statistics =
        std::fs::read_to_string(debug_root.join(session_id).join("statistics.html")).unwrap();
    assert!(
        statistics.contains("Repair error categories"),
        "{statistics}"
    );
    assert!(
        statistics.contains("missing_or_invalid_response_root"),
        "{statistics}"
    );
    assert!(
        statistics.contains("Runtime root repair help: 1"),
        "{statistics}"
    );
    assert!(statistics.contains("badge-native"), "{statistics}");
    assert!(statistics.contains("Action on-CPU time"), "{statistics}");
    assert!(statistics.contains("LLM API latency"), "{statistics}");
    assert!(statistics.contains("Finish reason: length"), "{statistics}");
    assert!(
        statistics.contains("<span>Count</span><strong>1</strong>"),
        "{statistics}"
    );
    assert!(
        statistics.contains("<span>Min bytes</span><strong>29</strong>"),
        "{statistics}"
    );
    assert!(
        statistics.contains("<span>Avg bytes</span><strong>29.0</strong>"),
        "{statistics}"
    );
    assert!(
        statistics.contains("<span>Max bytes</span><strong>29</strong>"),
        "{statistics}"
    );
    assert!(statistics.contains("100.0%"), "{statistics}");
    assert!(statistics.contains("claude-inline"), "{statistics}");
    assert!(statistics.contains("badge-inline"), "{statistics}");

    let prompt_dump =
        std::fs::read_to_string(debug_root.join(session_id).join("llm_prompt.html")).unwrap();
    assert!(prompt_dump.contains("inline system"), "{prompt_dump}");
    assert!(prompt_dump.contains("inline retry prompt"), "{prompt_dump}");
    assert!(
        prompt_dump.find("inline system").unwrap()
            < prompt_dump.find("inline retry prompt").unwrap(),
        "{prompt_dump}"
    );
    assert!(!prompt_dump.contains("call_previous"), "{prompt_dump}");
    assert!(!prompt_dump.contains("CWD: /work"), "{prompt_dump}");
    assert!(!prompt_dump.contains("worker_id"), "{prompt_dump}");

    let tool_schema_dump =
        std::fs::read_to_string(debug_root.join(session_id).join("tool_schema.dump")).unwrap();
    assert!(
        tool_schema_dump.contains("scope: latest_request_only"),
        "{tool_schema_dump}"
    );
    assert!(
        tool_schema_dump.contains("tool_call_mode: inline"),
        "{tool_schema_dump}"
    );
    assert!(
        tool_schema_dump.contains("no native API tools field"),
        "{tool_schema_dump}"
    );
    assert!(
        !tool_schema_dump.contains("self_tool"),
        "{tool_schema_dump}"
    );

    let response_dump =
        std::fs::read_to_string(debug_root.join(session_id).join("llm_response.dump")).unwrap();
    assert!(
        response_dump.contains("request_sequence: 1"),
        "{response_dump}"
    );
    assert!(
        response_dump.contains("tool_call_count: 1"),
        "{response_dump}"
    );
    assert!(response_dump.contains("call_current"), "{response_dump}");
    assert!(
        response_dump.contains("I will inspect the workspace."),
        "{response_dump}"
    );

    let records = read_all_history_records(
        &current_session_store(&state)
            .unwrap()
            .history_path_for_session(session_id),
    )
    .unwrap();
    assert!(records.iter().any(|record| matches!(
        record,
        ChatHistoryRecord::Event {
            turn_id,
            kind: ChatHistoryEventKind::Repair,
            extra,
            ..
        } if turn_id == &turn.turn_id
            && extra
                .get("payload")
                .and_then(|payload| payload.get("topic"))
                .and_then(|topic| topic.get("name"))
                .and_then(Value::as_str)
                == Some(CORE_TOPIC_MODEL_REPAIR)
    )));
}

#[test]
fn browser_reconnect_snapshot_preserves_live_core_working_state() {
    let state = routing_test_state();
    let turn = start_web_turn(
        &state,
        "session_a",
        "keep running while the browser reconnects",
    )
    .unwrap();

    let first_snapshot = snapshot_for(&state, TEST_PORT);
    let first = first_snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == "session_a")
        .unwrap();
    assert_eq!(first.state, "working");
    assert_eq!(first.active_turn_id.as_deref(), Some(turn.turn_id.as_str()));
    assert_eq!(
        first
            .turns
            .iter()
            .find(|candidate| candidate.turn_id == turn.turn_id)
            .unwrap()
            .state,
        "working"
    );
    assert!(first.workers.iter().any(|worker| worker.state == "working"));

    // A new WebSocket connection receives another snapshot from the same live
    // Core-owned in-memory state. Snapshot construction must not run persisted
    // restart recovery or manufacture an interrupted state.
    let reconnect_snapshot = snapshot_for(&state, TEST_PORT);
    let reconnected = reconnect_snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == "session_a")
        .unwrap();
    assert_eq!(reconnected.state, "working");
    assert_eq!(
        reconnected.active_turn_id.as_deref(),
        Some(turn.turn_id.as_str())
    );
    assert_eq!(
        reconnected
            .turns
            .iter()
            .find(|candidate| candidate.turn_id == turn.turn_id)
            .unwrap()
            .state,
        "working"
    );
    assert!(reconnected
        .workers
        .iter()
        .any(|worker| worker.state == "working"));
    assert!(reconnect_snapshot
        .sessions
        .iter()
        .all(|session| session.state != "interrupted"));
}

#[test]
fn successful_cwd_action_updates_only_its_session_and_reconnect_snapshot() {
    let state = routing_test_state();
    let mut events = state.events.subscribe();
    let cwd = "/work/session-a/new-location";
    let event = CoreTopicEvent::new(
        "session_a",
        CoreTopic::new(CORE_TOPIC_ACTION, json!({ "event": "finish" })),
        CoreSessionState::Running,
        json!({
            "action": "self_tool",
            "event": "finish",
            "status": "completed",
            "context_state": { "cwd": cwd },
        }),
    );

    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::Topics(vec![event]),
    );

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions["session_a"].current_dir, cwd);
    assert_eq!(sessions["session_b"].current_dir, "/work");
    drop(sessions);
    let snapshot = snapshot_for(&state, 12345);
    assert_eq!(
        snapshot
            .sessions
            .iter()
            .find(|session| session.session_id == "session_a")
            .unwrap()
            .current_dir,
        cwd
    );
    assert!(drain_wire_events(&mut events).iter().any(|wire| matches!(
        wire,
        WireEvent::CoreTopic { event, .. }
            if event["session_id"] == "session_a"
                && event["payload"]["context_state"]["cwd"] == cwd
    )));
}

#[test]
fn turn_completion_stats_are_attached_to_the_matching_final_answer() {
    let state = routing_test_state();
    let mut events = state.events.subscribe();
    let turn = start_web_turn(&state, "session_a", "complete this task").unwrap();
    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::Topics(vec![final_response_topic(
            "session_a",
            "final answer".to_string(),
        )]),
    );
    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::TurnFinished {
            outcome: TurnOutcome::final_response(
                "final answer",
                UsageStats {
                    llm_calls: 3,
                    prompt_tokens: 12_000,
                    completion_tokens: 450,
                    cached_tokens: 8_000,
                    tool_calls: 2,
                    ..UsageStats::zero()
                },
                None,
                None,
                Duration::from_millis(2_400),
            ),
        },
    );
    let sessions = state.sessions.lock().unwrap();
    let message = sessions["session_a"].messages.last().unwrap();
    assert_eq!(message.text, "final answer");
    assert_eq!(
        message.completion.as_ref().unwrap()["stats"]["prompt_tokens"],
        12_000
    );
    assert_eq!(message.completion.as_ref().unwrap()["elapsed_ms"], 2_400);
    let completed_turn = sessions["session_a"]
        .turns
        .iter()
        .find(|candidate| candidate.turn_id == turn.turn_id)
        .unwrap();
    assert_eq!(completed_turn.state, "finished");
    assert_eq!(completed_turn.final_answer.as_deref(), Some("final answer"));
    assert_eq!(
        completed_turn.completion.as_ref().unwrap()["stats"]["completion_tokens"],
        450
    );
    assert!(sessions["session_b"].messages.is_empty());
    drop(sessions);

    let events = drain_wire_events(&mut events);
    let response_id = events.iter().find_map(|event| match event {
        WireEvent::CoreTopic {
            turn_id,
            turn_event_id,
            event,
        } => {
            assert_eq!(turn_id.as_deref(), Some(turn.turn_id.as_str()));
            assert!(turn_event_id.as_deref().is_some_and(|id| !id.is_empty()));
            event["payload"]["ui_message_id"].as_str()
        }
        _ => None,
    });
    let completion = events
        .iter()
        .find_map(|event| match event {
            WireEvent::TurnFinished {
                turn_id, outcome, ..
            } if turn_id.as_deref() == Some(turn.turn_id.as_str()) => Some(outcome),
            _ => None,
        })
        .unwrap();
    assert_eq!(completion["message_id"].as_str(), response_id);
    assert_eq!(completion["completion"]["stats"]["cached_tokens"], 8_000);
}

#[test]
fn live_model_usage_is_retained_in_the_active_turn_and_correct_session() {
    let state = routing_test_state();
    let mut events = state.events.subscribe();
    let turn = start_web_turn(&state, "session_a", "measure this task").unwrap();
    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::ModelResponse {
            round: 2,
            content: String::new(),
            tool_calls: Vec::new(),
            truncated: false,
            runtime_phase: None,
            usage: UsageStats {
                prompt_tokens: 8_200,
                completion_tokens: 123,
                cached_tokens: 6_400,
                ..UsageStats::zero()
            },
        },
    );
    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::ModelResponse {
            round: 3,
            content: String::new(),
            tool_calls: Vec::new(),
            truncated: false,
            runtime_phase: Some("toolgen".to_string()),
            usage: UsageStats {
                prompt_tokens: 3_100,
                completion_tokens: 80,
                ..UsageStats::zero()
            },
        },
    );

    let sessions = state.sessions.lock().unwrap();
    let active = sessions["session_a"]
        .turns
        .iter()
        .find(|candidate| candidate.turn_id == turn.turn_id)
        .unwrap();
    assert_eq!(active.events.len(), 2);
    assert_eq!(active.events[0].payload["kind"], "model_response");
    assert_eq!(active.events[0].payload["usage"]["prompt_tokens"], 8_200);
    assert_eq!(active.events[1].payload["runtime_phase"], "toolgen");
    assert!(sessions["session_b"].turns.is_empty());
    drop(sessions);

    assert!(drain_wire_events(&mut events).iter().any(|event| matches!(
        event,
        WireEvent::WorkerActivity { session_id, turn_id, event, .. }
            if session_id == "session_a"
                && turn_id.as_deref() == Some(turn.turn_id.as_str())
                && event["usage"]["completion_tokens"] == 123
    )));
}

#[test]
fn lifecycle_updates_the_session_specific_context_limit() {
    let state = routing_test_state();
    let lifecycle = core_initialized_topic_event(
        "session_a",
        &CoreProfile {
            model: "model".to_string(),
        },
        "xml",
        131_072,
        50,
        6,
        0,
    );
    handle_worker_event(
        &state,
        "session_a",
        CoreSessionWorkerEvent::Topics(vec![lifecycle]),
    );

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions["session_a"].max_llm_input_tokens, 131_072);
    assert_eq!(sessions["session_b"].max_llm_input_tokens, 10_000);
}

#[test]
fn user_supplement_is_retained_in_the_authoritative_web_session_snapshot() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "SUPPLEMENT_DONE");
    let turn = start_web_turn(&state, &session_id, "Inspect the project").unwrap();
    append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "Use the second verification path".to_string(),
        None,
    )
    .unwrap();
    let sessions = state.sessions.lock().unwrap();
    let retained = sessions[&session_id]
        .turns
        .iter()
        .find(|candidate| candidate.turn_id == turn.turn_id)
        .unwrap();
    assert_eq!(retained.user_entries.len(), 2);
    assert_eq!(retained.user_entries[0].kind, "task");
    assert_eq!(retained.user_entries[1].kind, "supplement");
    assert_eq!(
        retained.user_entries[1].text,
        "Use the second verification path"
    );
}

#[test]
fn active_turn_supplement_consumes_pending_attachments_into_the_same_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "SUPPLEMENT_ATTACHMENT");
    let turn = start_web_turn(&state, &session_id, "inspect initial state").unwrap();
    let attachment = WebAttachment {
        id: "upload_supplement".to_string(),
        name: "extra-context.md".to_string(),
        path: "/tmp/timem-web/extra-context.md".to_string(),
        bytes: 128,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
        .unwrap()
        .attachments
        .push(attachment.clone());

    let updated = append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "also use this attached context".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(updated.turn_id, turn.turn_id);
    assert_eq!(updated.user_entries.len(), 2);
    assert_eq!(updated.user_entries[1].kind, "supplement");
    assert_eq!(
        updated.user_entries[1].attachments,
        vec![attachment.clone()]
    );
    assert!(state.sessions.lock().unwrap()[&session_id]
        .attachments
        .is_empty());
    assert!(uploaded_files_context(
        &updated.user_entries[1].attachments,
        &agent_core::response_protocol::XML_PROMPT_BOUNDARIES
    )
    .unwrap()
    .contains("extra-context.md (/tmp/timem-web/extra-context.md)"));
}

#[test]
fn failed_active_turn_supplement_does_not_drop_pending_attachments() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "SUPPLEMENT_ATTACHMENT_ROLLBACK");
    let turn = start_web_turn(&state, &session_id, "inspect initial state").unwrap();
    let attachment = WebAttachment {
        id: "upload_race".to_string(),
        name: "race-context.md".to_string(),
        path: "/tmp/timem-web/race-context.md".to_string(),
        bytes: 128,
    };
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.attachments.push(attachment.clone());
        session
            .turns
            .retain(|candidate| candidate.turn_id != turn.turn_id);
    }

    assert_eq!(
        append_turn_supplement_with_pending_attachments(
            &state,
            &session_id,
            "supplement during stale active turn".to_string(),
            None,
        )
        .unwrap_err(),
        "active_turn_not_found"
    );

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[&session_id].attachments, vec![attachment]);
}

#[test]
fn stale_supplement_after_cancel_consumes_pending_attachments_as_a_new_task() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "STALE_SUPPLEMENT_ATTACHMENT");
    let cancelled = start_web_turn(&state, &session_id, "cancel this").unwrap();
    let attachment = WebAttachment {
        id: "upload_after_cancel".to_string(),
        name: "new-task.md".to_string(),
        path: "/tmp/timem-web/new-task.md".to_string(),
        bytes: 64,
    };
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.active_turn_id = None;
        session.state = "ready".to_string();
        session.attachments.push(attachment.clone());
        session
            .turns
            .iter_mut()
            .find(|turn| turn.turn_id == cancelled.turn_id)
            .unwrap()
            .state = "finished".to_string();
    }

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSupplement {
            session_id: session_id.clone(),
            text: "new task with file".to_string(),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("stale supplement should become a new turn with attachments");

    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("expected turn update")
    };
    assert_ne!(turn.turn_id, cancelled.turn_id);
    assert_eq!(turn.user_entries[0].kind, "task");
    assert_eq!(turn.user_entries[0].attachments, vec![attachment]);
    assert!(state.sessions.lock().unwrap()[&session_id]
        .attachments
        .is_empty());
}

#[test]
fn selected_attachment_ids_stay_bound_to_their_messages() {
    let state = routing_test_state();
    let first = WebAttachment {
        id: "upload_first".to_string(),
        name: "first.png".to_string(),
        path: "/tmp/timem-web/first.png".to_string(),
        bytes: 10,
    };
    let second = WebAttachment {
        id: "upload_second".to_string(),
        name: "second.png".to_string(),
        path: "/tmp/timem-web/second.png".to_string(),
        bytes: 20,
    };
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.attachments = vec![first.clone(), second.clone()];
    }

    let first_ids = vec![first.id.clone()];
    let first_turn = start_web_turn_with_selected_attachments(
        &state,
        "session_a",
        "first queued message",
        Some(&first_ids),
        Some("queued_first"),
    )
    .unwrap();

    assert_eq!(first_turn.user_entries[0].attachments, vec![first.clone()]);
    {
        let sessions = state.sessions.lock().unwrap();
        assert_eq!(sessions["session_a"].attachments, vec![second.clone()]);
        assert!(sessions["session_a"]
            .consumed_attachment_ids
            .contains(&first.id));
        assert!(!sessions["session_a"]
            .consumed_attachment_ids
            .contains(&second.id));
    }

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.active_turn_id = None;
        session.pending_turn_id = None;
        session.state = "ready".to_string();
    }
    let second_ids = vec![second.id.clone()];
    let second_turn = start_web_turn_with_selected_attachments(
        &state,
        "session_a",
        "second queued message",
        Some(&second_ids),
        Some("queued_second"),
    )
    .unwrap();

    assert_eq!(
        second_turn.user_entries[0].attachments,
        vec![second.clone()]
    );
    assert!(state.sessions.lock().unwrap()["session_a"]
        .attachments
        .is_empty());
}

#[test]
fn explicit_empty_attachment_ids_leave_other_queued_files_pending() {
    let state = routing_test_state();
    let attachment = WebAttachment {
        id: "upload_reserved_for_later".to_string(),
        name: "later.png".to_string(),
        path: "/tmp/timem-web/later.png".to_string(),
        bytes: 30,
    };
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut("session_a")
        .unwrap()
        .attachments
        .push(attachment.clone());

    let turn = start_web_turn_with_selected_attachments(
        &state,
        "session_a",
        "message without files",
        Some(&[]),
        Some("queued_without_files"),
    )
    .unwrap();

    assert!(turn.user_entries[0].attachments.is_empty());
    assert_eq!(
        state.sessions.lock().unwrap()["session_a"].attachments,
        vec![attachment]
    );
}

#[test]
fn immediate_message_after_core_finalization_starts_a_new_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "FINAL_RACE");
    let first = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "finish immediately".to_string(),
            input_kind: None,
            source_turn_id: None,
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("first task should start");
    let WireEvent::TurnUpdated { turn: first, .. } = first else {
        panic!("expected first turn update")
    };
    let handle = primary_worker_handle(&state, &session_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while handle.is_accepting_user_supplements() {
        assert!(
            Instant::now() < deadline,
            "worker should close its supplement window"
        );
        thread::sleep(Duration::from_millis(2));
    }
    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        if session.active_turn_id.is_none() && session.pending_turn_id.is_none() {
            assert_eq!(
                session.state, "ready",
                "when Core completion is already consumed, UI must be ready"
            );
        } else {
            assert!(
                session.active_turn_id.as_deref() == Some(first.turn_id.as_str())
                    || session.pending_turn_id.as_deref() == Some(first.turn_id.as_str()),
                "an unconsumed Core turn must remain correlated to the first task"
            );
        }
    }

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSupplement {
            session_id: session_id.clone(),
            text: "run this after the final answer".to_string(),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("late immediate message should be resubmitted as a new task");

    let WireEvent::TurnUpdated { turn: second, .. } = event else {
        panic!("expected second turn update")
    };
    assert_ne!(second.turn_id, first.turn_id);
    assert_eq!(second.user_entries[0].kind, "task");
    assert_eq!(
        second.user_entries[0].text,
        "run this after the final answer"
    );
    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert!(
            session.active_turn_id.as_deref() == Some(second.turn_id.as_str())
                || session.pending_turn_id.as_deref() == Some(second.turn_id.as_str()),
            "the new task must be correlated before or after Core TurnStarted"
        );
        if session.active_turn_id.is_none() {
            assert_eq!(
                session.state, "ready",
                "a queued task must not manufacture Core working state"
            );
        }
    }
}

#[test]
fn turn_user_entries_are_persisted_with_raw_text_and_semantic_kind() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "HISTORY_KIND_WRITE");
    let turn = start_web_turn(&state, &session_id, "initial task").unwrap();

    append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "mid-turn correction".to_string(),
        None,
    )
    .unwrap();
    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TopicReply {
            session_id: session_id.clone(),
            worker_id: None,
            topic_name: "core.request.test".to_string(),
            request_id: Some("request_1".to_string()),
            decision: "accept".to_string(),
            payload: json!({ "summary": "approved local command" }),
        },
    )
    .unwrap();

    let records = read_all_history_records(
        &current_session_store(&state)
            .unwrap()
            .history_path_for_session(&session_id),
    )
    .unwrap();
    let user_messages = records
        .into_iter()
        .filter_map(|record| match record {
            ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id,
                kind,
                content,
                ..
            } => Some((turn_id, kind, content)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        user_messages,
        vec![
            (
                turn.turn_id.clone(),
                Some("task".to_string()),
                "initial task".to_string()
            ),
            (
                turn.turn_id.clone(),
                Some("supplement".to_string()),
                "mid-turn correction".to_string()
            ),
            (
                turn.turn_id,
                Some("approval".to_string()),
                "Accepted: approved local command".to_string()
            ),
        ]
    );
}

#[test]
fn duplicate_cancel_commands_are_idempotent_for_one_active_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "CANCEL_SPAM");
    start_web_turn(&state, &session_id, "transfer a large file").unwrap();

    for _ in 0..5 {
        assert!(handle_command(
            &state,
            TEST_PORT,
            ClientCommand::TurnCancel {
                session_id: session_id.clone(),
                target_command_id: None,
            },
        )
        .unwrap()
        .is_none());
    }

    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.active_turn_id = None;
        session.state = "ready".to_string();
    }
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.clone(),
            target_command_id: None,
        },
    )
    .unwrap()
    .is_none());
}

#[test]
fn turn_submit_during_an_active_turn_cannot_merge_into_the_current_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "QUEUED_NEXT_TURN");
    let first = start_web_turn(&state, &session_id, "Q1").unwrap();

    let error = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "Q2".to_string(),
            input_kind: None,
            source_turn_id: None,
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap_err();

    assert_eq!(error, "turn_already_active_use_supplement");
    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].turn_id, first.turn_id);
    assert_eq!(session.turns[0].user_entries.len(), 1);
    assert_eq!(session.turns[0].user_entries[0].text, "Q1");
}

#[test]
fn explicit_supplement_during_an_active_turn_stays_in_the_current_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "SUBMIT_RACE");
    let first = start_web_turn(&state, &session_id, "initial upload task").unwrap();

    let turn = append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "stop if this is still running".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(turn.turn_id, first.turn_id);
    assert_eq!(turn.user_entries.len(), 2);
    assert_eq!(turn.user_entries[1].kind, "supplement");
    assert_eq!(turn.user_entries[1].text, "stop if this is still running");
}

#[test]
fn repeated_user_sends_during_an_active_turn_are_ordered_supplements() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "MULTI_SUPPLEMENT_RACE");
    let first = start_web_turn(&state, &session_id, "initial long task").unwrap();

    for text in [
        "first correction while still working",
        "second correction after seeing output",
        "third correction from a rapid send click",
    ] {
        append_turn_supplement_with_pending_attachments(
            &state,
            &session_id,
            text.to_string(),
            None,
        )
        .unwrap();
    }
    append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "explicit supplement command stays in the same turn".to_string(),
        None,
    )
    .unwrap();

    let sessions = state.sessions.lock().unwrap();
    let retained = sessions[&session_id]
        .turns
        .iter()
        .find(|turn| turn.turn_id == first.turn_id)
        .unwrap();
    assert_eq!(
        retained
            .user_entries
            .iter()
            .map(|entry| (entry.kind.as_str(), entry.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("task", "initial long task"),
            ("supplement", "first correction while still working"),
            ("supplement", "second correction after seeing output"),
            ("supplement", "third correction from a rapid send click"),
            (
                "supplement",
                "explicit supplement command stays in the same turn"
            ),
        ]
    );
}

fn cancelled_test_outcome() -> TurnOutcome {
    let mut outcome =
        TurnOutcome::final_response("cancelled", UsageStats::zero(), None, None, Duration::ZERO);
    outcome.stop_reason = Some(agent_core::TurnStopReason::CancelledByUser);
    outcome
}

fn finished_projection_for(session_id: &str, turn_id: &str, epoch: u64) -> CoreSessionWorkerEvent {
    CoreSessionWorkerEvent::TurnProjection(agent_core::TurnProjection::Finished(
        agent_core::FinishedTurnProjection {
            token: agent_core::TurnToken {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                epoch,
            },
            outcome: agent_core::TurnProjectionOutcome::Cancelled,
        },
    ))
}

fn queue_turn_while_stopping(
    state: &AppState,
    session_id: &str,
    command_id: &str,
    text: &str,
) -> WebTurn {
    let event = handle_command_with_id(
        state,
        TEST_PORT,
        Some(command_id),
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: text.to_string(),
            attachment_ids: None,
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("queued user command must be visible immediately");
    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("queued user command must return a turn update")
    };
    turn
}

#[test]
fn runtime_restart_interrupts_and_discards_persisted_next_turn_without_redrive() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("restart_discards_next_turn"));
    std::fs::create_dir_all(&root).unwrap();
    let data_dir = root.join("data");
    let space = "restart_discards_next_turn_mem";
    set_test_mem(&state, data_dir.clone(), space);
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = data_dir.clone();
    template.initial_space = space.to_string();
    state.template = Arc::new(template.clone());
    state.sessions.lock().unwrap().clear();

    let session_id = create_session(
        &state,
        Some("Discard queued next turn on restart".to_string()),
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let old_turn = start_web_turn_with_command_id(
        &state,
        &session_id,
        "old turn interrupted by restart",
        Some("old-command"),
    )
    .unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
        .unwrap()
        .cancelling_turn_id = Some(old_turn.turn_id);
    let queued = queue_turn_while_stopping(
        &state,
        &session_id,
        "discarded-command",
        "queued turn must stop at restart",
    );
    assert_eq!(
        load_next_turn_intents(&state, &session_id).unwrap().len(),
        1
    );

    let original_manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    original_manager.shutdown_all().unwrap();

    let mut restarted = routing_test_state();
    restarted.sessions.lock().unwrap().clear();
    restarted.template = Arc::new(template);
    set_test_mem(&restarted, data_dir, space);
    assert_eq!(
        restore_stored_sessions_after_runtime_restart(&restarted).unwrap(),
        1
    );

    thread::sleep(Duration::from_millis(30));
    let startup_events = drain_worker_events(&restarted);
    assert!(startup_events.iter().all(|(_, _, _, event)| {
        !matches!(
            event,
            CoreSessionWorkerEvent::CommandAccepted { command_id }
                if command_id == "discarded-command"
        ) && !matches!(
            event,
            CoreSessionWorkerEvent::TurnStarted { command_id: Some(command_id) }
                if command_id == "discarded-command"
        )
    }));
    for (event_session_id, event_context_id, event_worker_id, event) in startup_events {
        handle_scoped_worker_event(
            &restarted,
            &event_session_id,
            &event_context_id,
            &event_worker_id,
            event,
        );
    }
    {
        let sessions = restarted.sessions.lock().unwrap();
        let restored = &sessions[&session_id];
        assert_eq!(restored.turn_projection.current(), None);
        assert_eq!(restored.active_turn_id, None);
        assert_eq!(restored.pending_turn_id, None);
        assert!(restored.next_turn_intents.is_empty());
        assert_eq!(
            restored
                .turns
                .iter()
                .find(|turn| turn.turn_id == queued.turn_id)
                .unwrap()
                .state,
            "interrupted"
        );
    }
    assert!(load_next_turn_intents(&restarted, &session_id)
        .unwrap()
        .is_empty());

    restarted
        .sessions
        .lock()
        .unwrap()
        .get_mut(&session_id)
        .unwrap()
        .attachments
        .push(WebAttachment {
            id: "old-redelivery-must-not-consume".to_string(),
            name: "old.txt".to_string(),
            path: "/tmp/old.txt".to_string(),
            bytes: 3,
        });
    let old_redelivery = handle_command_with_id(
        &restarted,
        TEST_PORT,
        Some("discarded-command"),
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "queued turn must stop at restart".to_string(),
            attachment_ids: Some(vec!["old-redelivery-must-not-consume".to_string()]),
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("old command ID may only return its interrupted turn");
    let WireEvent::TurnUpdated { turn, .. } = old_redelivery else {
        panic!("old command ID must return a turn update")
    };
    assert_eq!(turn.turn_id, queued.turn_id);
    assert_eq!(turn.state, "interrupted");
    assert!(restarted.sessions.lock().unwrap()[&session_id]
        .attachments
        .iter()
        .any(|file| file.id == "old-redelivery-must-not-consume"));
    thread::sleep(Duration::from_millis(30));
    assert!(drain_worker_events(&restarted)
        .iter()
        .all(|(_, _, _, event)| {
            !matches!(
                event,
                CoreSessionWorkerEvent::CommandAccepted { command_id }
                    if command_id == "discarded-command"
            )
        }));

    let new_turn = handle_command_with_id(
        &restarted,
        TEST_PORT,
        Some("new-command-after-restart"),
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "new work after restart".to_string(),
            attachment_ids: None,
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("a new command ID may create a new turn after restart");
    let WireEvent::TurnUpdated { turn: new_turn, .. } = new_turn else {
        panic!("new command must return a turn update")
    };
    assert_ne!(new_turn.turn_id, queued.turn_id);

    let started = Instant::now();
    let mut accepted = 0usize;
    while accepted == 0 {
        for (event_session_id, event_context_id, event_worker_id, event) in
            drain_worker_events(&restarted)
        {
            if matches!(
                &event,
                CoreSessionWorkerEvent::CommandAccepted { command_id }
                    if command_id == "new-command-after-restart"
            ) {
                accepted += 1;
            }
            assert!(!matches!(
                &event,
                CoreSessionWorkerEvent::CommandAccepted { command_id }
                    if command_id == "discarded-command"
            ));
            handle_scoped_worker_event(
                &restarted,
                &event_session_id,
                &event_context_id,
                &event_worker_id,
                event,
            );
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(accepted, 1);

    let manager = {
        let mut guard = restarted.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn next_turn_dispatch_waits_for_both_core_and_host_terminal_facts_in_either_order() {
    for projection_first in [true, false] {
        let state = routing_test_state();
        let session_id = "session_a";
        let current = start_web_turn(&state, session_id, "old turn").unwrap();
        {
            let mut sessions = state.sessions.lock().unwrap();
            sessions.get_mut(session_id).unwrap().cancelling_turn_id =
                Some(current.turn_id.clone());
        }
        let queued = queue_turn_while_stopping(
            &state,
            session_id,
            if projection_first {
                "projection-first"
            } else {
                "turn-finished-first"
            },
            "new turn",
        );
        let mut events = state.events.subscribe();

        if projection_first {
            handle_worker_event(
                &state,
                session_id,
                finished_projection_for(session_id, &current.turn_id, 1),
            );
        } else {
            handle_worker_event(
                &state,
                session_id,
                CoreSessionWorkerEvent::TurnFinished {
                    outcome: cancelled_test_outcome(),
                },
            );
        }

        assert!(
            drain_wire_events(&mut events)
                .iter()
                .all(|event| !matches!(event, WireEvent::HostError { message } if message.starts_with("next_turn_intent_"))),
            "one terminal fact must not be enough to dispatch"
        );
        {
            let sessions = state.sessions.lock().unwrap();
            let session = sessions.get(session_id).unwrap();
            assert_eq!(session.next_turn_intents.len(), 1);
            assert_ne!(
                session.pending_turn_id.as_deref(),
                Some(queued.turn_id.as_str())
            );
        }

        if projection_first {
            handle_worker_event(
                &state,
                session_id,
                CoreSessionWorkerEvent::TurnFinished {
                    outcome: cancelled_test_outcome(),
                },
            );
        } else {
            handle_worker_event(
                &state,
                session_id,
                finished_projection_for(session_id, &current.turn_id, 1),
            );
        }

        let dispatch_errors = drain_wire_events(&mut events)
            .into_iter()
            .filter(|event| {
                matches!(event, WireEvent::HostError { message } if message.starts_with("next_turn_intent_core_send_failed:"))
            })
            .count();
        assert_eq!(
            dispatch_errors, 1,
            "the second independent terminal fact must unlock exactly one dispatch attempt"
        );
        let sessions = state.sessions.lock().unwrap();
        let session = sessions.get(session_id).unwrap();
        assert_eq!(session.next_turn_intents.len(), 1);
        assert_eq!(session.pending_turn_id, None);
    }
}

#[test]
fn full_next_turn_queue_rejects_without_consuming_attachment_or_creating_turn() {
    let state = routing_test_state();
    let session_id = "session_a";
    let current = start_web_turn(&state, session_id, "old turn").unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.get_mut(session_id).unwrap().cancelling_turn_id = Some(current.turn_id);
    }
    for index in 0..MAX_NEXT_TURN_INTENTS {
        queue_turn_while_stopping(
            &state,
            session_id,
            &format!("queued-{index}"),
            &format!("queued text {index}"),
        );
    }
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions
            .get_mut(session_id)
            .unwrap()
            .attachments
            .push(WebAttachment {
                id: "keep-me".to_string(),
                name: "evidence.txt".to_string(),
                path: "/tmp/evidence.txt".to_string(),
                bytes: 8,
            });
    }
    let turns_before = state.sessions.lock().unwrap()[session_id].turns.len();

    let error = handle_command_with_id(
        &state,
        TEST_PORT,
        Some("queue-overflow"),
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "must be rejected".to_string(),
            attachment_ids: Some(vec!["keep-me".to_string()]),
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap_err();
    assert_eq!(error, "next_turn_intent_queue_full");

    let sessions = state.sessions.lock().unwrap();
    let session = sessions.get(session_id).unwrap();
    assert_eq!(session.next_turn_intents.len(), MAX_NEXT_TURN_INTENTS);
    assert_eq!(session.turns.len(), turns_before);
    assert!(session.attachments.iter().any(|file| file.id == "keep-me"));
    assert!(session.turns.iter().all(|turn| {
        turn.user_entries
            .iter()
            .all(|entry| entry.command_id.as_deref() != Some("queue-overflow"))
    }));
}

#[test]
fn rapid_stop_and_send_clicks_during_active_turn_do_not_break_the_session() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "STOP_AND_SEND_RACE");
    let first = start_web_turn(&state, &session_id, "copy a large artifact").unwrap();

    let turn = append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "first correction before stopping".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(turn.turn_id, first.turn_id);

    for _ in 0..3 {
        assert!(handle_command(
            &state,
            TEST_PORT,
            ClientCommand::TurnCancel {
                session_id: session_id.clone(),
                target_command_id: None,
            },
        )
        .unwrap()
        .is_none());
    }

    let turn = append_turn_supplement_with_pending_attachments(
        &state,
        &session_id,
        "late correction from another rapid send click".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(turn.turn_id, first.turn_id);

    let sessions = state.sessions.lock().unwrap();
    let retained = sessions[&session_id]
        .turns
        .iter()
        .find(|turn| turn.turn_id == first.turn_id)
        .unwrap();
    assert_eq!(
        retained
            .user_entries
            .iter()
            .map(|entry| (entry.kind.as_str(), entry.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("task", "copy a large artifact"),
            ("supplement", "first correction before stopping"),
            (
                "supplement",
                "late correction from another rapid send click"
            ),
        ]
    );
}

#[test]
fn stale_supplement_after_cancel_completion_starts_a_new_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "STALE_SUPPLEMENT");
    let cancelled = start_web_turn(&state, &session_id, "cancel this").unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.active_turn_id = None;
        session.state = "ready".to_string();
        session
            .turns
            .iter_mut()
            .find(|turn| turn.turn_id == cancelled.turn_id)
            .unwrap()
            .state = "finished".to_string();
    }

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSupplement {
            session_id: session_id.clone(),
            text: "new instruction after stop".to_string(),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .expect("stale supplement should become a new turn");

    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("expected turn update")
    };
    assert_ne!(turn.turn_id, cancelled.turn_id);
    assert_eq!(turn.user_entries[0].kind, "task");
    assert_eq!(turn.user_entries[0].text, "new instruction after stop");
}

#[test]
fn stale_topic_reply_after_turn_completion_is_ignored_without_host_error() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "STALE_REPLY");
    start_web_turn(&state, &session_id, "needs approval").unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.active_turn_id = None;
        session.state = "ready".to_string();
    }

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TopicReply {
            session_id,
            worker_id: None,
            topic_name: "core.request.test".to_string(),
            request_id: Some("request_1".to_string()),
            decision: "accept".to_string(),
            payload: json!({ "summary": "duplicate click" }),
        },
    )
    .unwrap();

    assert!(event.is_none());
}

#[test]
fn stale_work_instruction_reply_during_new_active_turn_is_ignored() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "STALE_WORK_INSTRUCTION_REPLY");
    let active = start_web_turn(&state, &session_id, "new active task").unwrap();

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TopicReply {
            session_id: session_id.clone(),
            worker_id: None,
            topic_name: CORE_TOPIC_WORK_INSTRUCTION_LOAD.to_string(),
            request_id: Some("old_work_instruction_request".to_string()),
            decision: "accept".to_string(),
            payload: json!({ "summary": "stale AGENTS.md approval" }),
        },
    )
    .unwrap();

    assert!(event.is_none());
    let sessions = state.sessions.lock().unwrap();
    let turn = sessions[&session_id]
        .turns
        .iter()
        .find(|turn| turn.turn_id == active.turn_id)
        .unwrap();
    assert_eq!(turn.user_entries.len(), 1);
    assert_eq!(turn.user_entries[0].kind, "task");
    assert!(sessions[&session_id]
        .pending_work_instruction_turn
        .is_none());
    assert_eq!(sessions[&session_id].work_instruction_allowed, None);
}

#[test]
fn background_exit_event_is_appended_to_its_original_turn() {
    let state = routing_test_state();
    let first = start_web_turn(&state, "session_a", "start background work").unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut("session_a").unwrap();
        session.active_turn_id = None;
        session.turns.push(WebTurn {
            turn_id: "turn_newer".to_string(),
            state: "working".to_string(),
            created_at_ms: now_ms(),
            user_entries: Vec::new(),
            events: Vec::new(),
            sub_answers: Vec::new(),
            final_answer: None,
            completion: None,
        });
        session.active_turn_id = Some("turn_newer".to_string());
    }

    append_turn_event(
        &state,
        "session_a",
        Some(&first.turn_id),
        "core_topic",
        json!({
            "topic": {"name": "core.action"},
            "payload": {
                "action_id": "call_bg",
                "event": "finish",
                "status": "background_running"
            }
        }),
    )
    .expect("background launch event");
    let terminal = json!({
        "topic": {"name": "core.action"},
        "payload": {
            "action_id": "call_bg",
            "event": "finish",
            "status": "completed",
            "turn_id": "action_turn_internal_id"
        }
    });
    let target = core_topic_target_turn_id(&state, "session_a", &terminal)
        .expect("action id should recover the original web turn");
    assert_eq!(target, first.turn_id);
    let reference = append_turn_event(&state, "session_a", Some(&target), "core_topic", terminal)
        .expect("original turn terminal event");
    assert_eq!(reference.turn_id, first.turn_id);

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions["session_a"];
    assert_eq!(session.active_turn_id.as_deref(), Some("turn_newer"));
    assert_eq!(
        session
            .turns
            .iter()
            .find(|turn| turn.turn_id == first.turn_id)
            .unwrap()
            .events
            .len(),
        2
    );
    assert!(session
        .turns
        .iter()
        .find(|turn| turn.turn_id == "turn_newer")
        .unwrap()
        .events
        .is_empty());
}

#[test]
fn polling_action_progress_reaches_web_event_stream_before_loop_finishes() {
    let state = routing_test_state();
    let marker = std::env::temp_dir().join(unique_web_id("polling_progress_marker"));
    let _ = std::fs::remove_file(&marker);
    let session_id = register_polling_progress_worker(&state, marker.clone());
    let mut wire_events = state.events.subscribe();
    submit_turn(&state, &session_id, "wait for marker".to_string()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_thought_while_polling = false;
    let mut saw_action_start_while_polling = false;
    while Instant::now() < deadline
        && !(saw_thought_while_polling && saw_action_start_while_polling)
    {
        for (event_session_id, context_id, worker_id, event) in drain_worker_events(&state) {
            handle_scoped_worker_event(&state, &event_session_id, &context_id, &worker_id, event);
        }
        let loop_finished = marker.exists();
        for event in drain_wire_events(&mut wire_events) {
            if let WireEvent::CoreTopic { event, .. } = event {
                if event["topic"]["name"] == CORE_TOPIC_MODEL_RESPONSE
                    && event["payload"]["free_talk"] == "Waiting for the polling marker."
                    && !loop_finished
                {
                    saw_thought_while_polling = true;
                }
                if event["topic"]["name"] == CORE_TOPIC_ACTION
                    && event["payload"]["event"] == "start"
                    && event["payload"]["kind"]["mode"] == "poll"
                    && !loop_finished
                {
                    saw_action_start_while_polling = true;
                }
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        saw_thought_while_polling,
        "free-talk should reach Web while the polling action is still blocked"
    );
    assert!(
        saw_action_start_while_polling,
        "poll action start should reach Web while polling is still blocked"
    );
    // Release the polling action only after both progress events were observed.
    // This makes the ordering deterministic instead of racing a fixed-delay writer.
    std::fs::write(&marker, b"ready").unwrap();
}

#[test]
fn active_turn_event_windows_are_bounded_and_session_isolated() {
    const SESSION_COUNT: usize = 5;
    const EVENTS_PER_SESSION: usize = 575;
    let state = routing_test_state();
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.clear();
        for ordinal in 0..SESSION_COUNT {
            let session_id = format!("bounded_{ordinal}");
            sessions.insert(
                session_id.clone(),
                test_web_session(&session_id, ordinal as u32, format!("Agent {ordinal}")),
            );
        }
    }

    let workers = (0..SESSION_COUNT)
        .map(|ordinal| {
            let state = state.clone();
            thread::spawn(move || {
                let session_id = format!("bounded_{ordinal}");
                start_web_turn(&state, &session_id, "stress this turn").unwrap();
                for sequence in 0..EVENTS_PER_SESSION {
                    let reference = append_active_turn_event(
                        &state,
                        &session_id,
                        "worker_activity",
                        json!({ "session": session_id, "sequence": sequence }),
                    )
                    .unwrap();
                    assert!(reference.event_id.starts_with("turn_event_"));
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    let sessions = state.sessions.lock().unwrap();
    for ordinal in 0..SESSION_COUNT {
        let session_id = format!("bounded_{ordinal}");
        let turn = sessions[&session_id].turns.last().unwrap();
        assert_eq!(turn.events.len(), EVENTS_PER_SESSION);
        assert_eq!(turn.events.first().unwrap().payload["sequence"], 0);
        assert_eq!(
            turn.events.last().unwrap().payload["sequence"],
            EVENTS_PER_SESSION - 1
        );
        assert!(turn
            .events
            .iter()
            .all(|event| event.payload["session"] == session_id));
    }
}

#[test]
fn active_turn_user_entries_drop_only_the_oldest_entries_at_the_bound() {
    let state = routing_test_state();
    start_web_turn(&state, "session_a", "initial task").unwrap();
    for sequence in 0..(MAX_TURN_USER_ENTRIES + 5) {
        append_turn_user_entry(
            &state,
            "session_a",
            "supplement",
            format!("supplement-{sequence}"),
        )
        .unwrap();
    }
    let sessions = state.sessions.lock().unwrap();
    let entries = &sessions["session_a"].turns.last().unwrap().user_entries;
    assert_eq!(entries.len(), MAX_TURN_USER_ENTRIES);
    assert_eq!(entries.first().unwrap().text, "supplement-5");
    assert_eq!(
        entries.last().unwrap().text,
        format!("supplement-{}", MAX_TURN_USER_ENTRIES + 4)
    );
}

struct PollingProgressModel {
    calls: u8,
    marker: PathBuf,
}

impl ModelClient for PollingProgressModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        let content = if self.calls == 1 {
            format!(
                "<ASSISTANT><free_talk>Waiting for the polling marker.</free_talk><actions><run_bash name=\"wait for polling marker\" interval_ms=\"20\" loop_timeout_ms=\"2000\" once_timeout_ms=\"500\"><loop_cmd>test -f {}</loop_cmd></run_bash></actions></ASSISTANT>",
                self.marker.display()
            )
        } else {
            confirmed_xml_response("<final_answer>Polling complete.</final_answer>")
        };
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content,
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                total_tokens: 12,
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

struct TaggedFinalModel(&'static str);

impl ModelClient for TaggedFinalModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content: confirmed_xml_response(&format!("<final_answer>{}</final_answer>", self.0)),
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                total_tokens: 12,
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

struct ToolGenPromptCaptureModel {
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ModelClient for ToolGenPromptCaptureModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        Ok(LlmResponse {
        tool_calls: Vec::new(),
            content: confirmed_xml_response("<toolgen_retrospect>No reusable tool was published.</toolgen_retrospect><final_answer>ToolGen review complete.</final_answer>"),
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                total_tokens: 12,
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

struct ToolGenPublishModel {
    calls: u8,
}

impl ModelClient for ToolGenPublishModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        let content = if self.calls == 1 {
            "<ASSISTANT><free_talk>Checking the reusable workflow.</free_talk><actions><run_bash name=\"check reusable workflow\" timeout_ms=\"5000\"><cmd>printf toolgen-host-check</cmd></run_bash></actions></ASSISTANT>".to_string()
        } else if prompt.contains(r#"<action_result><toolgen name="publish verified draft">"#) {
            confirmed_xml_response("<toolgen_retrospect>Published host-tool after runtime validation.</toolgen_retrospect><final_answer>ToolGen host workflow completed.</final_answer>")
        } else {
            let marker = "Write the new tool files only in this temporary staging directory:\n";
            let draft = prompt
                .split_once(marker)
                .and_then(|(_, rest)| rest.lines().next())
                .expect("ToolGen prompt must provide a draft path");
            std::fs::write(
                std::path::Path::new(draft).join("README.md"),
                "# host-tool\n\nPurpose: verify the Web host ToolGen event chain.\nSynopsis: `host-tool --self-test`\nInput: optional self-test flag. Output: ready.\nExample: `./tool.sh --self-test`\n",
            )
            .unwrap();
            std::fs::write(
                std::path::Path::new(draft).join("tool.sh"),
                "#!/bin/bash\nset -euo pipefail\n[[ ${1:-} == --self-test ]] && { echo ready; exit 0; }\necho ready\n",
            )
            .unwrap();
            std::fs::write(
                std::path::Path::new(draft).join(".timem-tool.json"),
                serde_json::json!({
                    "name": "host-tool",
                    "type": "test-automation",
                    "language": "bash",
                    "entrypoint": "tool.sh",
                    "synopsis": "host-tool [--self-test]",
                    "self_test": {"args": ["--self-test"], "timeout_ms": 2000}
                })
                .to_string(),
            )
            .unwrap();
            format!(
                "<ASSISTANT><free_talk>Publishing the verified draft.</free_talk><actions><toolgen name=\"publish verified draft\" op=\"publish\"><draft_path>{}</draft_path></toolgen></actions></ASSISTANT>",
                draft
            )
        };
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content,
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 100 + u32::from(self.calls),
                completion_tokens: 20,
                total_tokens: 120 + u32::from(self.calls),
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

struct InspectPathModel {
    round: u8,
}

impl ModelClient for InspectPathModel {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.round += 1;
        let content = if self.round == 1 {
            "<ASSISTANT><actions><self_tool name=\"inspect runtime paths\" type=\"path\"/></actions></ASSISTANT>".to_string()
        } else {
            confirmed_xml_response("<final_answer>paths inspected</final_answer>")
        };
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content,
            model_name: "test-model".to_string(),
            usage: UsageStats {
                llm_calls: 1,
                prompt_tokens: 10,
                completion_tokens: 2,
                total_tokens: 12,
                ..UsageStats::zero()
            },
            truncated: false,
        })
    }
}

fn register_polling_progress_worker(state: &AppState, marker: PathBuf) -> String {
    let ordinal = state.sessions.lock().unwrap().len() as u32;
    let session_id = unique_web_id("polling_progress_session");
    let context_id = test_context_id(&session_id);
    let worker_dir = std::env::temp_dir().join(format!("timem_web_polling_progress_{}", now_ms()));
    std::fs::create_dir_all(&worker_dir).unwrap();
    let mut core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        &worker_dir,
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let config = state.template.settings.lock().unwrap().config.clone();
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            config,
            CoreSessionWorkerWorkspace::new(
                &worker_dir,
                worker_dir.join("audit.json"),
                "test-web",
                "local",
            ),
            session_id.clone(),
            context_id.clone(),
            Some("Polling progress test".to_string()),
            None,
            PollingProgressModel { calls: 0, marker },
        )
        .unwrap();
    let mut session = test_web_session(&session_id, ordinal, "Polling progress test".to_string());
    session.current_dir = worker_dir.display().to_string();
    session.contexts[0] = WebContext {
        context_id: context_id.clone(),
        current_dir: worker_dir.display().to_string(),
        worker_ids: vec![worker_id.clone()],
    };
    session.workers[0].worker_id = worker_id.clone();
    session.workers[0].context_id = context_id.clone();
    session.active_context_id = context_id;
    session.primary_worker_id = worker_id;
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);
    session_id
}

fn register_real_worker(state: &AppState, name: &'static str) -> String {
    let ordinal = state.sessions.lock().unwrap().len() as u32;
    let session_id = unique_web_id("test_session");
    let context_id = test_context_id(&session_id);
    let worker_dir =
        std::env::temp_dir().join(format!("timem_web_topic_route_{name}_{}", now_ms()));
    std::fs::create_dir_all(&worker_dir).unwrap();
    let core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        &worker_dir,
    );
    let config = state.template.settings.lock().unwrap().config.clone();
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            config,
            CoreSessionWorkerWorkspace::new(
                &worker_dir,
                worker_dir.join("audit.json"),
                "test-web",
                "local",
            ),
            session_id.clone(),
            context_id.clone(),
            Some(name.to_string()),
            None,
            TaggedFinalModel(name),
        )
        .unwrap();
    let mut session = test_web_session(&session_id, ordinal, name.to_string());
    session.current_dir = worker_dir.display().to_string();
    session.contexts[0] = WebContext {
        context_id: context_id.clone(),
        current_dir: worker_dir.display().to_string(),
        worker_ids: vec![worker_id.clone()],
    };
    session.workers[0].worker_id = worker_id.clone();
    session.workers[0].context_id = context_id;
    session.active_context_id = session.contexts[0].context_id.clone();
    session.primary_worker_id = worker_id;
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);
    session_id
}

fn register_toolgen_capture_worker(state: &AppState, prompts: Arc<Mutex<Vec<String>>>) -> String {
    let ordinal = state.sessions.lock().unwrap().len() as u32;
    let session_id = unique_web_id("toolgen_session");
    let context_id = test_context_id(&session_id);
    let worker_dir = std::env::temp_dir().join(format!("timem_web_toolgen_{}", now_ms()));
    std::fs::create_dir_all(&worker_dir).unwrap();
    let core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        &worker_dir,
    );
    let config = state.template.settings.lock().unwrap().config.clone();
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            config,
            CoreSessionWorkerWorkspace::new(
                &worker_dir,
                worker_dir.join("audit.json"),
                "test-web",
                "local",
            ),
            session_id.clone(),
            context_id.clone(),
            Some("ToolGen test".to_string()),
            None,
            ToolGenPromptCaptureModel { prompts },
        )
        .unwrap();
    let mut session = test_web_session(&session_id, ordinal, "ToolGen test".to_string());
    session.current_dir = worker_dir.display().to_string();
    session.contexts[0] = WebContext {
        context_id: context_id.clone(),
        current_dir: worker_dir.display().to_string(),
        worker_ids: vec![worker_id.clone()],
    };
    session.workers[0].worker_id = worker_id.clone();
    session.workers[0].context_id = context_id;
    session.active_context_id = session.contexts[0].context_id.clone();
    session.primary_worker_id = worker_id;
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);
    session_id
}

fn register_toolgen_publish_worker(state: &AppState) -> String {
    let ordinal = state.sessions.lock().unwrap().len() as u32;
    let session_id = unique_web_id("toolgen_publish_session");
    let context_id = test_context_id(&session_id);
    let worker_dir = std::env::temp_dir().join(format!("timem_web_toolgen_publish_{}", now_ms()));
    std::fs::create_dir_all(&worker_dir).unwrap();
    let memory_dir = current_mem_state(state).unwrap().layout.memory_dir();
    let mut core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        &memory_dir,
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    let config = state.template.settings.lock().unwrap().config.clone();
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            config,
            CoreSessionWorkerWorkspace::new(
                &worker_dir,
                worker_dir.join("audit.json"),
                "test-web",
                "local",
            ),
            session_id.clone(),
            context_id.clone(),
            Some("ToolGen publish test".to_string()),
            None,
            ToolGenPublishModel { calls: 0 },
        )
        .unwrap();
    let mut session = test_web_session(&session_id, ordinal, "ToolGen publish test".to_string());
    session.current_dir = worker_dir.display().to_string();
    session.contexts[0] = WebContext {
        context_id: context_id.clone(),
        current_dir: worker_dir.display().to_string(),
        worker_ids: vec![worker_id.clone()],
    };
    session.workers[0].worker_id = worker_id.clone();
    session.workers[0].context_id = context_id;
    session.active_context_id = session.contexts[0].context_id.clone();
    session.primary_worker_id = worker_id;
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);
    session_id
}

fn add_completed_toolgen_source_turn(state: &AppState, session_id: &str) -> String {
    let source = start_web_turn(state, session_id, "extract reusable timing data").unwrap();
    let mut sessions = state.sessions.lock().unwrap();
    let session = sessions.get_mut(session_id).unwrap();
    let turn = session
        .turns
        .iter_mut()
        .find(|turn| turn.turn_id == source.turn_id)
        .unwrap();
    turn.state = "completed".to_string();
    turn.final_answer = Some("source final answer must remain visible".to_string());
    turn.completion = Some(json!({"stop_reason": "finished"}));
    session.state = "ready".to_string();
    session.active_turn_id = None;
    source.turn_id
}

fn drive_worker_until_session_ready(
    state: &AppState,
    session_id: &str,
    prompts: &Arc<Mutex<Vec<String>>>,
) {
    let started = Instant::now();
    loop {
        for (event_session_id, context_id, worker_id, event) in drain_worker_events(state) {
            handle_scoped_worker_event(state, &event_session_id, &context_id, &worker_id, event);
        }
        if !prompts.lock().unwrap().is_empty()
            && state.sessions.lock().unwrap()[session_id].state == "ready"
        {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "ToolGen worker did not finish"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn manual_toolgen_uses_system_only_without_optional_user_guidance() {
    let state = routing_test_state();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let session_id = register_toolgen_capture_worker(&state, Arc::clone(&prompts));
    let source_turn_id = add_completed_toolgen_source_turn(&state, &session_id);

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: String::new(),
            input_kind: Some("toolgen".to_string()),
            source_turn_id: Some(source_turn_id.clone()),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("manual ToolGen must create a Web turn");
    };
    assert_eq!(turn.state, "pending");
    assert!(turn.turn_id.starts_with("web_toolgen_turn_"));
    assert!(turn.user_entries.is_empty());
    drive_worker_until_session_ready(&state, &session_id, &prompts);

    let prompt = prompts.lock().unwrap().last().unwrap().clone();
    assert!(prompt.contains("[TOOL_GEN_TASK] Please extract the reusable function"));
    assert!(!prompt.contains("Referenced completed turn id:"));
    assert!(!prompt.contains("Completed task:"));
    assert!(!prompt.contains("Completed task result:"));
    assert!(!prompt.contains("Observed action evidence:"));
    assert!(prompt.contains("<ASSISTANT>"));
    assert!(!prompt.contains("<ASSISTANT id="));
    assert!(!prompt.contains("ToolGen test_TOOLGEN"));
    let marker = "[TOOL_GEN_TASK] Please extract the reusable function";
    let marker_at = prompt.find(marker).unwrap();
    let delta_start = prompt[..marker_at].rfind("<prompt_delta ").unwrap();
    let delta_end =
        prompt[marker_at..].find("</prompt_delta>").unwrap() + marker_at + "</prompt_delta>".len();
    let toolgen_delta = &prompt[delta_start..delta_end];
    assert!(toolgen_delta.contains("<RUNTIME>"));
    assert!(!toolgen_delta.contains("<USER>"));
    assert!(!toolgen_delta.contains("## RUNTIME"));
    assert!(!toolgen_delta.contains("## USER"));

    let sessions = state.sessions.lock().unwrap();
    let source = sessions[&session_id]
        .turns
        .iter()
        .find(|turn| turn.turn_id == source_turn_id)
        .unwrap();
    assert_eq!(
        source.final_answer.as_deref(),
        Some("source final answer must remain visible")
    );
}

#[test]
fn manual_toolgen_adds_optional_guidance_as_user_component() {
    let state = routing_test_state();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let session_id = register_toolgen_capture_worker(&state, Arc::clone(&prompts));
    let source_turn_id = add_completed_toolgen_source_turn(&state, &session_id);

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "Prefer a Python CLI with JSON output.".to_string(),
            input_kind: Some("toolgen".to_string()),
            source_turn_id: Some(source_turn_id),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("manual ToolGen must create a Web turn");
    };
    assert_eq!(turn.user_entries.len(), 1);
    assert_eq!(turn.user_entries[0].kind, "toolgen_instruction");
    drive_worker_until_session_ready(&state, &session_id, &prompts);

    let prompt = prompts.lock().unwrap().last().unwrap().clone();
    let guidance_at = prompt
        .find("Prefer a Python CLI with JSON output.")
        .expect("optional ToolGen guidance must reach the model");
    let delta_start = prompt[..guidance_at].rfind("<prompt_delta ").unwrap();
    let delta_end = prompt[guidance_at..].find("</prompt_delta>").unwrap()
        + guidance_at
        + "</prompt_delta>".len();
    let toolgen_delta = &prompt[delta_start..delta_end];
    assert!(toolgen_delta.contains("<USER>"));
    let runtime_at = toolgen_delta.find("<RUNTIME>").unwrap();
    let user_at = toolgen_delta.find("<USER>").unwrap();
    assert!(
        runtime_at < user_at,
        "the fixed ToolGen RUNTIME instruction must precede optional USER guidance"
    );
    assert!(!toolgen_delta.contains("## RUNTIME"));
    assert!(!toolgen_delta.contains("## USER"));
}

#[test]
fn manual_toolgen_publishes_tool_and_retains_the_complete_web_event_chain() {
    let state = routing_test_state();
    let session_id = register_toolgen_publish_worker(&state);
    let source_turn_id = add_completed_toolgen_source_turn(&state, &session_id);

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "Keep the generated CLI deterministic.".to_string(),
            input_kind: Some("toolgen".to_string()),
            source_turn_id: Some(source_turn_id.clone()),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap();

    let started = Instant::now();
    loop {
        for (event_session_id, context_id, worker_id, event) in drain_worker_events(&state) {
            handle_scoped_worker_event(&state, &event_session_id, &context_id, &worker_id, event);
        }
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        let finished = session.state == "ready" && session.tools.len() == 1;
        drop(sessions);
        if finished {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "ToolGen publish workflow did not finish: {}",
            serde_json::to_string(&state.sessions.lock().unwrap()[&session_id]).unwrap()
        );
        thread::sleep(Duration::from_millis(5));
    }

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    let source = session
        .turns
        .iter()
        .find(|turn| turn.turn_id == source_turn_id)
        .unwrap();
    assert_eq!(
        source.final_answer.as_deref(),
        Some("source final answer must remain visible")
    );
    let toolgen_turn = session.turns.last().unwrap();
    assert_eq!(toolgen_turn.state, "finished");
    assert!(toolgen_turn.final_answer.is_none());
    assert_eq!(session.tools[0].name, "host-tool");
    assert_eq!(
        toolgen_turn.completion.as_ref().unwrap()["stats"]["llm_calls"],
        3
    );

    let serialized_events = serde_json::to_string(&toolgen_turn.events).unwrap();
    assert!(serialized_events.contains("Checking the reusable workflow"));
    assert!(serialized_events.contains("Publishing the verified draft"));
    assert!(serialized_events.contains("run_bash"));
    assert!(serialized_events.contains("toolgen"));
    assert!(serialized_events.contains("published"));
    assert!(serialized_events.contains("model_response"));
    assert!(serialized_events.contains("runtime_phase"));
    assert!(!serialized_events.contains("model_error"));
}

#[test]
fn manual_toolgen_rejects_bad_source_state_and_duplicate_clicks() {
    let state = routing_test_state();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let session_id = register_toolgen_capture_worker(&state, Arc::clone(&prompts));

    let missing = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: String::new(),
            input_kind: Some("toolgen".to_string()),
            source_turn_id: Some("missing_turn".to_string()),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap_err();
    assert_eq!(missing, "toolgen_source_turn_not_found");

    let unfinished = start_web_turn(&state, &session_id, "unfinished task").unwrap();
    {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.active_turn_id = None;
        session.state = "ready".to_string();
    }
    let incomplete = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: String::new(),
            input_kind: Some("toolgen".to_string()),
            source_turn_id: Some(unfinished.turn_id),
            attachment_ids: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap_err();
    assert_eq!(incomplete, "toolgen_source_turn_not_completed");

    let source_turn_id = add_completed_toolgen_source_turn(&state, &session_id);
    let request = || ClientCommand::TurnSubmit {
        session_id: session_id.clone(),
        text: String::new(),
        input_kind: Some("toolgen".to_string()),
        source_turn_id: Some(source_turn_id.clone()),
        attachment_ids: None,
        role_id: None,
        role_ids: Vec::new(),
    };
    assert!(handle_command(&state, TEST_PORT, request())
        .unwrap()
        .is_some());
    assert_eq!(
        handle_command(&state, TEST_PORT, request()).unwrap_err(),
        "turn_already_active"
    );
    drive_worker_until_session_ready(&state, &session_id, &prompts);
    assert_eq!(prompts.lock().unwrap().len(), 1);
}

#[test]
fn real_concurrent_workers_route_final_topics_to_matching_web_sessions() {
    let state = routing_test_state();
    let mut events = state.events.subscribe();
    let alpha = register_real_worker(&state, "ALPHA_DONE");
    let beta = register_real_worker(&state, "BETA_DONE");
    primary_worker_handle(&state, &alpha)
        .unwrap()
        .run_turn("alpha", None)
        .unwrap();
    primary_worker_handle(&state, &beta)
        .unwrap()
        .run_turn("beta", None)
        .unwrap();

    for _ in 0..200 {
        for (session_id, context_id, worker_id, event) in drain_worker_events(&state) {
            handle_scoped_worker_event(&state, &session_id, &context_id, &worker_id, event);
        }
        let sessions = state.sessions.lock().unwrap();
        let complete = sessions[&alpha]
            .messages
            .iter()
            .any(|message| message.text == "ALPHA_DONE")
            && sessions[&beta]
                .messages
                .iter()
                .any(|message| message.text == "BETA_DONE");
        drop(sessions);
        if complete {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(
        sessions[&alpha]
            .messages
            .last()
            .map(|message| message.text.as_str()),
        Some("ALPHA_DONE"),
        "alpha worker did not publish a final response: {:#?}",
        sessions[&alpha]
    );
    assert_eq!(
        sessions[&beta]
            .messages
            .last()
            .map(|message| message.text.as_str()),
        Some("BETA_DONE"),
        "beta worker did not publish a final response: {:#?}",
        sessions[&beta]
    );
    assert!(sessions[&alpha]
        .messages
        .iter()
        .all(|message| message.text != "BETA_DONE"));
    assert!(sessions[&beta]
        .messages
        .iter()
        .all(|message| message.text != "ALPHA_DONE"));
    drop(sessions);

    let topic_session_ids = drain_wire_events(&mut events)
        .into_iter()
        .filter_map(|event| match event {
            WireEvent::CoreTopic { event, .. }
                if event["topic"]["name"] == CORE_TOPIC_MODEL_RESPONSE =>
            {
                event["session_id"].as_str().map(str::to_string)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(topic_session_ids.contains(&alpha));
    assert!(topic_session_ids.contains(&beta));
}

#[test]
fn real_worker_self_tool_path_call_is_read_only_for_web_session_cwd() {
    let state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_cwd_e2e_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let mut core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            model: "test-model".to_string(),
        },
        root.join("memory"),
    );
    core.change_prompt_cwd(root.display().to_string()).unwrap();
    let config = state.template.settings.lock().unwrap().config.clone();
    let session_id = unique_web_id("cwd_session");
    let context_id = test_context_id(&session_id);
    let worker_id = state
        .manager
        .lock()
        .unwrap()
        .spawn_worker_in_session_with_model_client(
            core,
            config,
            CoreSessionWorkerWorkspace::new(&root, root.join("audit.json"), "test-web", "local"),
            session_id.clone(),
            context_id.clone(),
            Some("CWD_TEST".to_string()),
            None,
            InspectPathModel { round: 0 },
        )
        .unwrap();
    let mut session = test_web_session(&session_id, 0, "CWD_TEST".to_string());
    session.current_dir = root.display().to_string();
    session.contexts[0] = WebContext {
        context_id: context_id.clone(),
        current_dir: root.display().to_string(),
        worker_ids: vec![worker_id.clone()],
    };
    session.workers[0].worker_id = worker_id.clone();
    session.workers[0].context_id = context_id;
    session.active_context_id = session.contexts[0].context_id.clone();
    session.primary_worker_id = worker_id;
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), session);
    submit_turn(&state, &session_id, "change cwd".to_string()).unwrap();

    for _ in 0..200 {
        for (event_session_id, event_context_id, event_worker_id, event) in
            drain_worker_events(&state)
        {
            handle_scoped_worker_event(
                &state,
                &event_session_id,
                &event_context_id,
                &event_worker_id,
                event,
            );
        }
        if state.sessions.lock().unwrap()[&session_id]
            .active_turn_id
            .is_none()
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        state.sessions.lock().unwrap()[&session_id].current_dir,
        root.display().to_string()
    );
}

#[tokio::test]
async fn targeted_cancel_finishes_a_turn_waiting_for_work_instruction_permission() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "CANCEL_PENDING_GUIDE");
    let current_dir = {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.work_instruction_mode = WorkInstructionLoadMode::Ask;
        PathBuf::from(&session.current_dir)
    };
    std::fs::write(
        current_dir.join("AGENTS.md"),
        "Never reaches Core after Stop.",
    )
    .unwrap();
    let mut wire_events = state.events.subscribe();

    handle_command_with_id(
        &state,
        TEST_PORT,
        Some("submit-guide"),
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "wait for permission".to_string(),
            attachment_ids: None,
            input_kind: None,
            source_turn_id: None,
            role_id: None,
            role_ids: Vec::new(),
        },
    )
    .unwrap();
    let pending_turn_id = state.sessions.lock().unwrap()[&session_id]
        .pending_turn_id
        .clone()
        .unwrap();

    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnCancel {
            session_id: session_id.clone(),
            target_command_id: Some("submit-guide".to_string()),
        },
    )
    .unwrap()
    .is_none());

    let published = drain_wire_events(&mut wire_events);
    let finished = published.iter().find_map(|event| match event {
        WireEvent::TurnFinished {
            session_id: event_session_id,
            turn_id,
            outcome,
        } if event_session_id == &session_id => Some((turn_id, outcome)),
        _ => None,
    });
    let (turn_id, outcome) = finished.expect("Host-only pending cancel must publish TurnFinished");
    assert_eq!(turn_id.as_deref(), Some(pending_turn_id.as_str()));
    assert_eq!(outcome["completion"]["stop_reason"], "CancelledByUser");

    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(session.pending_turn_id, None);
    assert_eq!(session.active_turn_id, None);
    assert_eq!(session.state, "ready");
    assert!(session.pending_work_instruction_turn.is_none());
    let turn = session
        .turns
        .iter()
        .find(|turn| turn.turn_id == pending_turn_id)
        .unwrap();
    assert_eq!(turn.state, "finished");
    assert_eq!(
        turn.completion.as_ref().unwrap()["stop_reason"],
        "CancelledByUser"
    );
}

#[tokio::test]
async fn ask_mode_queues_the_first_turn_then_loads_work_instructions_after_matching_reply() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "WORK_GUIDE_DONE");
    let current_dir = {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.work_instruction_mode = WorkInstructionLoadMode::Ask;
        PathBuf::from(&session.current_dir)
    };
    std::fs::write(
        current_dir.join("AGENTS.md"),
        "Use the workspace-specific verification rule.",
    )
    .unwrap();
    let mut wire_events = state.events.subscribe();

    submit_turn(&state, &session_id, "continue the task".to_string()).unwrap();

    let request_event = drain_wire_events(&mut wire_events)
        .into_iter()
        .find_map(|event| match event {
            WireEvent::CoreTopic { event, .. }
                if event["topic"]["name"] == CORE_TOPIC_WORK_INSTRUCTION_LOAD =>
            {
                Some(event)
            }
            _ => None,
        })
        .expect("ask mode must publish a work-instruction request");
    let request_id = request_event["payload"]["request_id"].as_str().unwrap();
    assert_eq!(request_event["state"]["name"], "waiting_user_with_timeout");
    assert!(state.sessions.lock().unwrap()[&session_id]
        .pending_work_instruction_turn
        .is_some());

    assert!(resolve_work_instruction_decision(
        &state,
        &session_id,
        Some(request_id),
        HostDecision::Accept,
    )
    .unwrap());
    assert!(session_context(&state, &session_id, &[])
        .unwrap()
        .unwrap()
        .contains("workspace-specific verification rule"));

    for _ in 0..200 {
        for (event_session_id, event_context_id, event_worker_id, event) in
            drain_worker_events(&state)
        {
            handle_scoped_worker_event(
                &state,
                &event_session_id,
                &event_context_id,
                &event_worker_id,
                event,
            );
        }
        if state.sessions.lock().unwrap()[&session_id]
            .messages
            .iter()
            .any(|message| message.text == "WORK_GUIDE_DONE")
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let session = &state.sessions.lock().unwrap()[&session_id];
    assert_eq!(session.work_instruction_allowed, Some(true));
    assert!(session.pending_work_instruction_turn.is_none());
    assert!(session
        .messages
        .iter()
        .any(|message| message.text == "WORK_GUIDE_DONE"));
}

#[tokio::test]
async fn ask_mode_rejects_a_mismatched_reply_without_releasing_the_pending_turn() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "MISMATCH_DONE");
    let current_dir = {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.work_instruction_mode = WorkInstructionLoadMode::Ask;
        PathBuf::from(&session.current_dir)
    };
    std::fs::write(current_dir.join("AGENTS.md"), "Apply this guide.").unwrap();

    submit_turn(&state, &session_id, "continue the task".to_string()).unwrap();
    let error = resolve_work_instruction_decision(
        &state,
        &session_id,
        Some("wrong_request_id"),
        HostDecision::Accept,
    )
    .unwrap_err();

    assert_eq!(error, "topic_reply_request_id_mismatch");
    let sessions = state.sessions.lock().unwrap();
    assert!(sessions[&session_id]
        .pending_work_instruction_turn
        .is_some());
    assert_eq!(sessions[&session_id].work_instruction_allowed, None);
}

#[tokio::test]
async fn ask_mode_decline_continues_the_turn_without_loading_work_instructions() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "DECLINE_DONE");
    let current_dir = {
        let mut sessions = state.sessions.lock().unwrap();
        let session = sessions.get_mut(&session_id).unwrap();
        session.work_instruction_mode = WorkInstructionLoadMode::Ask;
        PathBuf::from(&session.current_dir)
    };
    std::fs::write(current_dir.join("AGENTS.md"), "MUST_NOT_REACH_MODEL").unwrap();

    submit_turn(&state, &session_id, "continue the task".to_string()).unwrap();
    let request_id = state.sessions.lock().unwrap()[&session_id]
        .pending_work_instruction_turn
        .as_ref()
        .unwrap()
        .request_id
        .clone();
    assert!(resolve_work_instruction_decision(
        &state,
        &session_id,
        Some(&request_id),
        HostDecision::Decline,
    )
    .unwrap());

    let sessions = state.sessions.lock().unwrap();
    assert_eq!(sessions[&session_id].work_instruction_allowed, Some(false));
    assert!(sessions[&session_id]
        .pending_work_instruction_turn
        .is_none());
    drop(sessions);
    let context = session_context(&state, &session_id, &[]).unwrap();
    let context = context.as_deref().unwrap_or("");
    assert!(!context.contains("host: local_web"));
    assert!(!context.contains("transport: websocket"));
    assert!(!context.contains("MUST_NOT_REACH_MODEL"));
}

fn publish_web_test_tool(repo: &SessionToolRepo, name: &str, searchable: &str) -> ToolSummary {
    let draft = repo.create_draft().unwrap();
    std::fs::write(
        draft.join("README.md"),
        format!("# {name}\n\n`{name} <file>`\n"),
    )
    .unwrap();
    std::fs::write(
        draft.join("tool.sh"),
        format!("#!/bin/bash\nprintf '%s\\n' {searchable}\n"),
    )
    .unwrap();
    std::fs::write(
        draft.join(".timem-tool.json"),
        serde_json::json!({
            "name": name,
            "type": "debug",
            "language": "bash",
            "entrypoint": "tool.sh",
            "synopsis": format!("{name} <file>"),
            "self_test": {"args": ["--self-test"], "timeout_ms": 2000}
        })
        .to_string(),
    )
    .unwrap();
    repo.publish(&draft).unwrap().summary
}

#[test]
fn toolrepo_commands_are_session_scoped() {
    let state = routing_test_state();
    let repo_a = session_tool_repo(&state, "session_a").unwrap();
    let tool = publish_web_test_tool(&repo_a, "trace-window-finder", "exclusive-search-marker");

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ToolRepoSearch {
            session_id: "session_a".into(),
            query: "exclusive-search-marker".into(),
            limit: Some(10),
        },
    )
    .unwrap()
    .unwrap();
    assert!(
        matches!(event, WireEvent::ToolRepoSearchResult { session_id, ref tools, .. } if session_id == "session_a" && tools[0].tool_id == tool.tool_id)
    );
    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ToolRepoSearch {
            session_id: "session_b".into(),
            query: "exclusive-search-marker".into(),
            limit: Some(10),
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(event, WireEvent::ToolRepoSearchResult { ref tools, .. } if tools.is_empty()));
}

#[test]
fn toolrepo_detail_rename_and_future_prompt_hint_share_the_published_state() {
    let state = routing_test_state();
    let repo = session_tool_repo(&state, "session_a").unwrap();
    let tool = publish_web_test_tool(&repo, "json-log-filter", "needle-in-code");

    let detail = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ToolRepoDetail {
            session_id: "session_a".into(),
            tool_id: tool.tool_id.clone(),
        },
    )
    .unwrap()
    .unwrap();
    assert!(
        matches!(detail, WireEvent::ToolRepoDetail { ref detail, .. } if detail.readme.contains("json-log-filter") && detail.files.iter().any(|file| file.path == "tool.sh"))
    );

    let renamed = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ToolRepoRename {
            session_id: "session_a".into(),
            tool_id: tool.tool_id,
            new_name: "structured-log-filter".into(),
        },
    )
    .unwrap()
    .unwrap();
    assert!(
        matches!(renamed, WireEvent::ToolRepoUpdated { ref tools, .. } if tools[0].name == "structured-log-filter")
    );
    let context = session_context(&state, "session_a", &[]).unwrap().unwrap();
    assert!(context.contains(repo.root().to_string_lossy().as_ref()));
    assert!(context.contains("semantic names"));
    assert!(context.contains("run the script's --help"));
    let other_context = session_context(&state, "session_b", &[]).unwrap();
    assert!(!other_context
        .as_deref()
        .unwrap_or("")
        .contains("Previously accumulated reusable scripts"));
}

#[test]
fn workspace_lock_error_identifies_owner_and_escape_routes() {
    let root = std::env::temp_dir().join(unique_web_id("workspace_lock_message"));
    std::fs::create_dir_all(&root).unwrap();
    let lock = agent_core::WorkspaceInstanceLock::acquire(&root, "timem-shell").unwrap();

    let message = friendly_workspace_instance_error(
        "workspace_already_in_use".to_string(),
        root.to_string_lossy().as_ref(),
    );
    assert!(message.contains("already open"));
    assert!(message.contains("actively locked"));
    assert!(message.contains(&format!("PID:  {}", std::process::id())));
    assert!(message.contains("Host: timem-shell"));
    assert!(message.contains("workspace-instance.lock"));
    assert!(message.contains("PID identity matches"));
    assert!(message.contains("--space /absolute/path/to/another-mem"));
    assert!(message.contains("Do not delete the lock file"));
    #[cfg(unix)]
    {
        assert!(message.contains(&format!("ps -fp {}", std::process::id())));
        assert!(message.contains(&format!("kill -TERM {}", std::process::id())));
        assert!(message.contains("lsof"));
    }
    #[cfg(windows)]
    {
        assert!(message.contains(&format!("tasklist /FI \"PID eq {}\"", std::process::id())));
        assert!(message.contains(&format!("taskkill /PID {} /T", std::process::id())));
    }

    drop(lock);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn workspace_lock_error_handles_unreadable_owner_metadata() {
    let root = std::env::temp_dir().join(unique_web_id("workspace_lock_unknown_owner"));
    std::fs::create_dir_all(root.join(".guard")).unwrap();
    std::fs::write(
        agent_core::WorkspaceInstanceLock::lock_path(&root),
        b"not-json",
    )
    .unwrap();

    let message = friendly_workspace_instance_error(
        "workspace_already_in_use".to_string(),
        root.to_string_lossy().as_ref(),
    );
    assert!(message.contains("PID:  unavailable"));
    assert!(message.contains("--space /absolute/path/to/another-mem"));
    #[cfg(unix)]
    assert!(message.contains("fuser -v"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn friendly_web_instance_error_replaces_in_use_with_actionable_message() {
    let data_dir = std::path::PathBuf::from("/tmp/timem_test_data");
    let msg =
        friendly_web_instance_error("web_instance_in_use".to_string(), &data_dir, ".test_mem");
    assert!(msg.contains("already running on this memory space"));
    assert!(msg.contains("/tmp/timem_test_data"));
    assert!(msg.contains(".test_mem"));
    assert!(msg.contains("--space"));
    assert!(msg.contains("/absolute/path/to/mem"));
    assert!(!msg.contains("web_instance_in_use"));
    let passthrough =
        friendly_web_instance_error("other_error".to_string(), &data_dir, ".test_mem");
    assert_eq!(passthrough, "other_error");
}

#[test]
fn existing_instance_is_rejected_with_pid_url_and_stop_command() {
    let path = std::env::temp_dir().join(unique_web_id("existing_instance_recovery"));
    let mut lease = WebInstanceLease::acquire(&path).unwrap();
    lease
        .publish(&WebInstanceInfo {
            pid: 4242,
            launch_parent_pid: None,
            port: Some(18080),
            token: None,
            browser_url: Some("http://127.0.0.1:18080/".to_string()),
            public_access: false,
            started_at_ms: 123,
        })
        .unwrap();

    let info = WebInstanceLease::read_info(&path).unwrap();
    let error = existing_web_instance_error(&info);
    assert!(error.contains("never reused"));
    assert!(error.contains("PID:  4242"));
    assert!(error.contains("http://127.0.0.1:18080/"));
    assert!(error.contains("--space"));
    assert!(!error.contains("--data-dir"));
    #[cfg(unix)]
    assert!(error.contains("kill -TERM 4242"));
    #[cfg(windows)]
    assert!(error.contains("taskkill /PID 4242 /T"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    drop(lease);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn healthy_existing_instance_is_detected_without_taking_its_lease() {
    let path = std::env::temp_dir().join(unique_web_id("healthy_existing_instance"));
    let mut owner = WebInstanceLease::acquire(&path).unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    owner
        .publish(&WebInstanceInfo {
            pid: 4242,
            launch_parent_pid: Some(std::process::id()),
            port: Some(port),
            token: None,
            browser_url: Some(format!("http://127.0.0.1:{port}/")),
            public_access: false,
            started_at_ms: 123,
        })
        .unwrap();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        let size = socket.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET /api/health "));
        let body = format!(r#"{{"ok":true,"port":{port}}}"#);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let outcome = open_web_instance_after_handoff(
        &path,
        Duration::from_secs(1),
        Duration::from_millis(10),
        Duration::from_millis(250),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        WebInstanceOpenOutcome::Existing(ref info) if info.pid == 4242
    ));
    server.await.unwrap();
    assert_eq!(
        WebInstanceLease::acquire(&path).unwrap_err(),
        "web_instance_in_use"
    );

    drop(owner);
    remove_web_instance_files(&path);
}

#[cfg(unix)]
#[tokio::test]
async fn healthy_host_with_dead_launcher_is_not_reused_during_shutdown_handoff() {
    let path = std::env::temp_dir().join(unique_web_id("dead_launcher_handoff"));
    let mut owner = WebInstanceLease::acquire(&path).unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    owner
        .publish(&WebInstanceInfo {
            pid: 4242,
            launch_parent_pid: Some(i32::MAX as u32),
            port: Some(port),
            token: None,
            browser_url: Some(format!("http://127.0.0.1:{port}/")),
            public_access: false,
            started_at_ms: 123,
        })
        .unwrap();

    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_for_server = Arc::clone(&accepted);
    let server = tokio::spawn(async move {
        if let Ok(Ok((_socket, _))) = timeout(Duration::from_millis(200), listener.accept()).await {
            accepted_for_server.fetch_add(1, Ordering::SeqCst);
        }
    });
    let release = tokio::spawn(async move {
        sleep(Duration::from_millis(80)).await;
        drop(owner);
    });

    let outcome = open_web_instance_after_handoff(
        &path,
        Duration::from_secs(1),
        Duration::from_millis(10),
        Duration::from_millis(100),
    )
    .await
    .unwrap();

    let WebInstanceOpenOutcome::Owned(new_owner) = outcome else {
        panic!("a Host whose launcher exited must not be reused");
    };
    release.await.unwrap();
    server.await.unwrap();
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        0,
        "launcher death must be checked before probing the dying Host"
    );

    drop(new_owner);
    remove_web_instance_files(&path);
}

#[tokio::test]
async fn startup_waits_for_exiting_instance_then_takes_over_its_lease() {
    let path = std::env::temp_dir().join(unique_web_id("instance_handoff"));
    let owner = WebInstanceLease::acquire(&path).unwrap();

    let release = tokio::spawn(async move {
        sleep(Duration::from_millis(80)).await;
        drop(owner);
    });
    let outcome = open_web_instance_after_handoff(
        &path,
        Duration::from_secs(1),
        Duration::from_millis(10),
        Duration::from_millis(20),
    )
    .await
    .unwrap();

    let WebInstanceOpenOutcome::Owned(new_owner) = outcome else {
        panic!("an inaccessible exiting instance must not be reported as reusable");
    };
    release.await.unwrap();
    assert_eq!(
        WebInstanceLease::acquire(&path).unwrap_err(),
        "web_instance_in_use"
    );
    drop(new_owner);
    remove_web_instance_files(&path);
}

#[tokio::test]
async fn inaccessible_instance_that_keeps_the_lock_fails_clearly() {
    let path = std::env::temp_dir().join(unique_web_id("inaccessible_instance"));
    let owner = WebInstanceLease::acquire(&path).unwrap();

    let error = match open_web_instance_after_handoff(
        &path,
        Duration::from_millis(80),
        Duration::from_millis(10),
        Duration::from_millis(10),
    )
    .await
    {
        Ok(_) => panic!("an inaccessible lock owner must not be treated as recovered"),
        Err(error) => error,
    };
    assert!(error.contains("still holding this workspace lock"));
    assert!(error.contains("not reachable"));
    assert!(!error.contains("web_instance_in_use"));

    drop(owner);
    remove_web_instance_files(&path);
}

fn remove_web_instance_files(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[test]
fn startup_resource_errors_are_actionable_instead_of_internal_codes() {
    let data_dir = std::path::PathBuf::from("/tmp/timem_test_data");
    let locked =
        friendly_memory_space_error("mem_guard_timeout".to_string(), &data_dir, ".test_mem");
    assert!(locked.contains("locked by another running operation"));
    assert!(locked.contains("automatically recovers locks"));
    assert!(locked.contains("cargo run -p timem_web -- --space"));
    assert!(!locked.contains("mem_guard_timeout"));

    let requested = friendly_bind_error("requested_port_unavailable".to_string(), Some(18080));
    assert!(requested.contains("Port 18080"));
    assert!(requested.contains("cargo run -p timem_web"));
    assert!(!requested.contains("requested_port_unavailable"));

    let exhausted = friendly_bind_error(
        format!("no_available_port_in_range:{PORT_START}..={PORT_END}"),
        None,
    );
    assert!(exhausted.contains("local web port"));
    assert!(exhausted.contains("firewall or sandbox"));
    assert!(!exhausted.contains("no_available_port_in_range"));
}

#[test]
#[ignore = "manual endpoint scale/concurrency performance profile"]
fn model_endpoint_scale_and_concurrency_performance_profile() {
    fn percentile(mut samples: Vec<std::time::Duration>, percentile: usize) -> std::time::Duration {
        samples.sort_unstable();
        samples[(samples.len() - 1) * percentile / 100]
    }

    fn endpoint(index: usize) -> ModelEndpointConfig {
        ModelEndpointConfig {
            id: format!("endpoint-{index:05}"),
            name: format!("Endpoint {index:05}"),
            model: "gpt-4.1-mini".to_string(),
            api_protocol: "openai-compatible".to_string(),
            response_protocol: "xml".to_string(),
            base_url: format!("https://api-{index:05}.example.test/v1"),
            max_llm_input_tokens: 100_000,
            max_llm_output_tokens: 10_000,
            stream: false,
            api_key: format!("secret-{index:05}"),
            http_headers: Default::default(),
            request_fields: Default::default(),
        }
    }

    for count in [100usize, 1_000, 5_000] {
        let state = routing_test_state();
        let root = std::env::temp_dir().join(unique_web_id("timem_web_endpoint_perf"));
        set_test_mem(&state, root.clone(), ".test_mem");
        {
            let mut mem = state.mem.lock().unwrap();
            mem.model_endpoints = (0..count).map(endpoint).collect();
            save_model_endpoints(&mem.layout.memory_dir(), &mem.model_endpoints).unwrap();
        }
        let memory_dir = state.mem.lock().unwrap().layout.memory_dir();
        let file_bytes = std::fs::metadata(model_endpoints_path(&memory_dir))
            .unwrap()
            .len();

        let reports = model_endpoint_reports(&state).unwrap();
        assert_eq!(reports.len(), count);
        let report_json_bytes = serde_json::to_vec(&reports).unwrap().len();
        let report_samples = (0..25)
            .map(|_| {
                let started = std::time::Instant::now();
                let reports = model_endpoint_reports(&state).unwrap();
                assert_eq!(reports.len(), count);
                started.elapsed()
            })
            .collect::<Vec<_>>();
        let secret_samples = (0..25)
            .map(|_| {
                let started = std::time::Instant::now();
                assert_eq!(
                    model_endpoint_secrets(&state, &format!("endpoint-{:05}", count - 1))
                        .unwrap()
                        .api_key,
                    format!("secret-{:05}", count - 1)
                );
                started.elapsed()
            })
            .collect::<Vec<_>>();

        let update_started = std::time::Instant::now();
        upsert_model_endpoint(
            &state,
            ModelEndpointInput {
                id: Some(format!("endpoint-{:05}", count - 1)),
                name: format!("Endpoint {:05} renamed", count - 1),
                model: "gpt-4.1".to_string(),
                api_protocol: "openai-compatible".to_string(),
                response_protocol: "json".to_string(),
                base_url: "https://updated.example.test/v1".to_string(),
                max_llm_input_tokens: 100_000,
                max_llm_output_tokens: 10_000,
                stream: true,
                api_key: None,
                http_headers: BTreeMap::from([
                    ("Authorization".to_string(), "****".to_string()),
                    ("X-Tenant".to_string(), "****".to_string()),
                ]),
                request_fields: Default::default(),
            },
        )
        .unwrap();
        let update_elapsed = update_started.elapsed();

        let reload_started = std::time::Instant::now();
        assert_eq!(
            load_model_endpoints_resilient(&memory_dir).unwrap().len(),
            count
        );
        let reload_elapsed = reload_started.elapsed();

        eprintln!(
            "endpoint_perf count={count} file_bytes={file_bytes} report_json_bytes={report_json_bytes} reports_p50_us={} reports_p95_us={} secret_p50_us={} secret_p95_us={} update_fsync_us={} reload_us={}",
            percentile(report_samples.clone(), 50).as_micros(),
            percentile(report_samples, 95).as_micros(),
            percentile(secret_samples.clone(), 50).as_micros(),
            percentile(secret_samples, 95).as_micros(),
            update_elapsed.as_micros(),
            reload_elapsed.as_micros(),
        );
        let _ = std::fs::remove_dir_all(root);
    }

    let state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_endpoint_concurrency_perf"));
    set_test_mem(&state, root.clone(), ".test_mem");
    {
        let mut mem = state.mem.lock().unwrap();
        mem.model_endpoints = (0..1_000).map(endpoint).collect();
        save_model_endpoints(&mem.layout.memory_dir(), &mem.model_endpoints).unwrap();
    }
    let started = std::time::Instant::now();
    let threads = (0..16)
        .map(|index| {
            let state = state.clone();
            std::thread::spawn(move || {
                upsert_model_endpoint(
                    &state,
                    ModelEndpointInput {
                        id: Some(format!("endpoint-{index:05}")),
                        name: format!("Concurrent {index:05}"),
                        model: "gpt-4.1-mini".to_string(),
                        api_protocol: "openai-compatible".to_string(),
                        response_protocol: "xml".to_string(),
                        base_url: format!("https://concurrent-{index}.example.test/v1"),
                        max_llm_input_tokens: 100_000,
                        max_llm_output_tokens: 10_000,
                        stream: false,
                        api_key: None,
                        http_headers: Default::default(),
                        request_fields: Default::default(),
                    },
                )
                .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    let elapsed = started.elapsed();
    eprintln!(
        "endpoint_perf concurrent_updates=16 list_count=1000 total_us={} mean_us={}",
        elapsed.as_micros(),
        elapsed.as_micros() / 16
    );
    assert_eq!(model_endpoint_reports(&state).unwrap().len(), 1_000);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_model_endpoints_load_with_default_token_limits() {
    let root = std::env::temp_dir().join(unique_web_id("timem_web_legacy_model_endpoint"));
    let memory_dir = root.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(
        model_endpoints_path(&memory_dir),
        r#"[{"id":"legacy","name":"Legacy","model":"gpt","api_protocol":"openai-compatible","response_protocol":"xml","base_url":"https://api.example.test/v1","api_key":""}]"#,
    )
    .unwrap();

    let endpoints = load_model_endpoints_resilient(&memory_dir).unwrap();
    assert_eq!(endpoints[0].max_llm_input_tokens, 100_000);
    assert_eq!(endpoints[0].max_llm_output_tokens, 10_000);
    assert!(!endpoints[0].stream);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn model_endpoint_headers_accept_safe_special_values_and_reject_injection() {
    let valid = BTreeMap::from([
        (
            "X-Signature_1".to_string(),
            "quoted=\"yes\"; path=C:\\tmp; city=东京".to_string(),
        ),
        ("X-Api.Key".to_string(), "a+b/c==".to_string()),
    ]);
    assert_eq!(agent_core::validate_model_http_headers(&valid), Ok(()));
    assert!(agent_core::validate_model_http_headers(&BTreeMap::from([(
        "Bad Header".to_string(),
        "value".to_string()
    )]))
    .is_err());
    assert!(agent_core::validate_model_http_headers(&BTreeMap::from([(
        "X-Test".to_string(),
        "ok\r\nInjected: yes".to_string()
    )]))
    .is_err());
    assert!(agent_core::validate_model_http_headers(&BTreeMap::from([(
        "X-Test".to_string(),
        "bad\0value".to_string()
    )]))
    .is_err());
}

#[test]
fn model_endpoint_rejects_token_limits_outside_supported_lists() {
    let invalid_input = ModelEndpointInput {
        id: None,
        name: "Invalid input".to_string(),
        model: "gpt".to_string(),
        api_protocol: "openai-compatible".to_string(),
        response_protocol: "xml".to_string(),
        base_url: "https://api.example.test/v1".to_string(),
        max_llm_input_tokens: 128_000,
        max_llm_output_tokens: 10_000,
        stream: false,
        api_key: None,
        http_headers: Default::default(),
        request_fields: Default::default(),
    };
    assert_eq!(
        normalize_model_endpoint_input(None, invalid_input).unwrap_err(),
        "invalid_model_endpoint_max_input_tokens"
    );

    let invalid_output = ModelEndpointInput {
        id: None,
        name: "Invalid output".to_string(),
        model: "gpt".to_string(),
        api_protocol: "openai-compatible".to_string(),
        response_protocol: "xml".to_string(),
        base_url: "https://api.example.test/v1".to_string(),
        max_llm_input_tokens: 200_000,
        max_llm_output_tokens: 8_000,
        stream: false,
        api_key: None,
        http_headers: Default::default(),
        request_fields: Default::default(),
    };
    assert_eq!(
        normalize_model_endpoint_input(None, invalid_output).unwrap_err(),
        "invalid_model_endpoint_max_output_tokens"
    );
}

#[test]
fn model_endpoint_stream_requires_openai_compatible_protocol() {
    let input = ModelEndpointInput {
        id: None,
        name: "Responses stream".to_string(),
        model: "gpt".to_string(),
        api_protocol: "openai-responses".to_string(),
        response_protocol: "json".to_string(),
        base_url: "https://api.example.test/v1".to_string(),
        max_llm_input_tokens: 200_000,
        max_llm_output_tokens: 20_000,
        stream: true,
        api_key: None,
        http_headers: Default::default(),
        request_fields: Default::default(),
    };
    assert_eq!(
        normalize_model_endpoint_input(None, input).unwrap_err(),
        "model_endpoint_stream_requires_openai_compatible"
    );
}

#[test]
fn shared_model_endpoints_are_persisted_redacted_editable_and_deletable() {
    let state = routing_test_state();
    let root = std::env::temp_dir().join(unique_web_id("timem_web_model_endpoints"));
    set_test_mem(&state, root.clone(), ".test_mem");
    let session_id = register_real_worker(&state, "MODEL_ENDPOINT_APPLY");

    let created = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ModelEndpointUpsert {
            endpoint: ModelEndpointInput {
                id: Some("endpoint-one".to_string()),
                name: "Production".to_string(),
                model: "gpt-4.1".to_string(),
                api_protocol: "openai-compatible".to_string(),
                response_protocol: "xml".to_string(),
                base_url: "https://api.example.test/v1".to_string(),
                max_llm_input_tokens: 100_000,
                max_llm_output_tokens: 10_000,
                stream: false,
                api_key: Some("secret-endpoint-key".to_string()),
                http_headers: BTreeMap::from([
                    ("X-Tenant".to_string(), "tenant\"one\\东京".to_string()),
                    ("Authorization".to_string(), "Basic special==".to_string()),
                ]),
                request_fields: BTreeMap::from([
                    ("service_tier".to_string(), json!("fast")),
                    (
                        "vendor_options".to_string(),
                        json!({"priority": 2, "enabled": true}),
                    ),
                ]),
            },
        },
    )
    .unwrap()
    .unwrap();
    let serialized = serde_json::to_string(&created).unwrap();
    assert!(serialized.contains("Production"));
    assert!(serialized.contains("api_key_configured"));
    assert!(!serialized.contains("secret-endpoint-key"));
    assert!(!serialized.contains("tenant\"one"));
    assert!(serialized.contains("\"X-Tenant\":\"****\""));
    assert!(serialized.contains("\"service_tier\":\"****\""));
    assert!(serialized.contains("\"vendor_options\":\"****\""));
    assert!(!serialized.contains("priority"));
    assert_eq!(
        model_endpoint_secrets(&state, "endpoint-one")
            .unwrap()
            .api_key,
        "secret-endpoint-key"
    );
    let applied = apply_model_endpoint(&state, &session_id, "endpoint-one").unwrap();
    assert_eq!(applied.model, "gpt-4.1");
    assert_eq!(applied.max_llm_input_tokens, 100_000);
    assert_eq!(applied.max_llm_output_tokens, 10_000);
    assert!(!applied.stream);
    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert_eq!(
            session
                .runtime
                .env
                .get("TIMEM_MAX_LLM_INPUT")
                .map(String::as_str),
            Some("100000")
        );
        assert_eq!(
            session
                .runtime
                .env
                .get("TIMEM_MAX_LLM_OUTPUT")
                .map(String::as_str),
            Some("10000")
        );
        assert_eq!(
            session.runtime.env.get("TIMEM_STREAM").map(String::as_str),
            Some("false")
        );
        assert_eq!(
            session
                .runtime
                .settings
                .config
                .request_fields
                .get("service_tier"),
            Some(&json!("fast"))
        );
        let request_fields: Value =
            serde_json::from_str(session.runtime.env.get("TIMEM_REQUEST_FIELDS").unwrap()).unwrap();
        assert_eq!(request_fields["vendor_options"]["priority"], 2);
    }
    let reveal = execute_browser_command(
        &state,
        TEST_PORT,
        BrowserCommand {
            command_id: None,
            accepted_mem_epoch: 1,
            accepted_lane: None,
            accepted_at_ms: now_ms(),
            performance_sent_at_ms: None,
            command: ClientCommand::ModelEndpointSecretReveal {
                endpoint_id: "endpoint-one".to_string(),
            },
        },
    );
    assert!(matches!(
        reveal.event,
        Some(WireEvent::ModelEndpointSecretRevealed {
            ref endpoint_id,
            ref api_key,
            ref http_headers,
            ref request_fields,
        }) if endpoint_id == "endpoint-one" && api_key == "secret-endpoint-key"
            && http_headers.get("X-Tenant").map(String::as_str) == Some("tenant\"one\\东京")
            && request_fields.get("service_tier") == Some(&json!("fast"))
    ));

    let mut endpoint_events = state.events.subscribe();
    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ModelEndpointUpsert {
            endpoint: ModelEndpointInput {
                id: Some("endpoint-one".to_string()),
                name: "Production".to_string(),
                model: "gpt-4.1".to_string(),
                api_protocol: "openai-compatible".to_string(),
                response_protocol: "xml".to_string(),
                base_url: "https://api.example.test/v1".to_string(),
                max_llm_input_tokens: 1_000_000,
                max_llm_output_tokens: 50_000,
                stream: true,
                api_key: None,
                http_headers: Default::default(),
                request_fields: Default::default(),
            },
        },
    )
    .unwrap();
    {
        let sessions = state.sessions.lock().unwrap();
        let session = &sessions[&session_id];
        assert_eq!(session.max_llm_input_tokens, 1_000_000);
        assert_eq!(session.runtime_profile.max_llm_input_tokens, 1_000_000);
        assert_eq!(session.runtime_profile.max_llm_output_tokens, 50_000);
        assert!(session.runtime_profile.stream);
        assert!(session.runtime.settings.config.openai_compatible.stream);
        assert_eq!(
            session.runtime.env.get("TIMEM_STREAM").map(String::as_str),
            Some("true")
        );
    }
    let endpoint_update_events = drain_wire_events(&mut endpoint_events);
    assert!(endpoint_update_events.iter().any(|event| matches!(
        event,
        WireEvent::SessionRuntimeUpdated {
            session_id: updated_session_id,
            runtime_profile,
        } if updated_session_id == &session_id
            && runtime_profile.max_llm_input_tokens == 1_000_000
            && runtime_profile.max_llm_output_tokens == 50_000
            && runtime_profile.stream
    )));

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ModelEndpointUpsert {
            endpoint: ModelEndpointInput {
                id: Some("endpoint-one".to_string()),
                name: "Production renamed".to_string(),
                model: "gpt-4.1-mini".to_string(),
                api_protocol: "openai-responses".to_string(),
                response_protocol: "json".to_string(),
                base_url: "https://responses.example.test/v1".to_string(),
                max_llm_input_tokens: 1_000_000,
                max_llm_output_tokens: 50_000,
                stream: false,
                api_key: None,
                http_headers: Default::default(),
                request_fields: Default::default(),
            },
        },
    )
    .unwrap();
    assert_eq!(
        model_endpoint_secrets(&state, "endpoint-one")
            .unwrap()
            .api_key,
        "secret-endpoint-key"
    );
    let memory_dir = state.mem.lock().unwrap().layout.memory_dir();
    let restored = load_model_endpoints_resilient(&memory_dir).unwrap();
    assert_eq!(restored[0].name, "Production renamed");
    assert_eq!(restored[0].max_llm_input_tokens, 1_000_000);
    assert_eq!(restored[0].max_llm_output_tokens, 50_000);
    assert!(!restored[0].stream);
    assert_eq!(restored[0].api_key, "secret-endpoint-key");

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::ModelEndpointDelete {
            endpoint_id: "endpoint-one".to_string(),
        },
    )
    .unwrap();
    assert!(load_model_endpoints_resilient(&memory_dir)
        .unwrap()
        .is_empty());
    let manager = {
        let mut guard = state.manager.lock().unwrap();
        std::mem::replace(&mut *guard, CoreSessionWorkerManager::new())
    };
    manager.shutdown_all().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn default_mem_auto_port_candidates_prefer_13764_once() {
    let offset = 777;
    let candidates = automatic_web_port_candidates(true, offset);
    assert_eq!(
        candidates.first().copied(),
        Some(DEFAULT_MEM_PREFERRED_PORT)
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|port| **port == DEFAULT_MEM_PREFERRED_PORT)
            .count(),
        1
    );
    assert_eq!(candidates.len(), usize::from(PORT_END - PORT_START + 1));
    assert!(candidates
        .iter()
        .all(|port| (PORT_START..=PORT_END).contains(port)));
}

#[test]
fn non_default_mem_auto_port_candidates_keep_rotating_pool_order() {
    let offset = 777;
    let candidates = automatic_web_port_candidates(false, offset);
    assert_eq!(candidates.first().copied(), Some(PORT_START + offset));
    assert_eq!(candidates.len(), usize::from(PORT_END - PORT_START + 1));
}

#[test]
fn system_default_mem_detection_uses_the_resolved_path() {
    let default = agent_core::default_memory_dir().unwrap();
    assert!(uses_system_default_mem(default.to_str().unwrap()));
    assert!(!uses_system_default_mem(
        default.with_file_name("different-mem").to_str().unwrap()
    ));
}

#[tokio::test]
async fn occupied_default_mem_preferred_port_falls_back_to_another_supported_port() {
    let occupied = match TcpListener::bind((Ipv4Addr::LOCALHOST, DEFAULT_MEM_PREFERRED_PORT)).await
    {
        Ok(listener) => Some(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => None,
        Err(error) => panic!("failed to occupy preferred port: {error}"),
    };
    let selected = bind_web_listener(None, false, true).await.unwrap();
    let selected_port = selected.local_addr().unwrap().port();
    assert_ne!(selected_port, DEFAULT_MEM_PREFERRED_PORT);
    assert!((PORT_START..=PORT_END).contains(&selected_port));
    drop(selected);
    drop(occupied);
}

#[tokio::test]
async fn explicit_port_remains_higher_priority_than_default_mem_preference() {
    let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let explicit_port = probe.local_addr().unwrap().port();
    drop(probe);
    if !(PORT_START..=PORT_END).contains(&explicit_port) {
        return;
    }
    let selected = bind_web_listener(Some(explicit_port), false, true)
        .await
        .unwrap();
    assert_eq!(selected.local_addr().unwrap().port(), explicit_port);
}

#[test]
fn browser_url_highlight_is_ansi_only_when_color_is_enabled() {
    let url = "http://127.0.0.1:13764/?token=test-token";
    assert_eq!(highlighted_browser_url(url, false), url);
    assert_eq!(
        highlighted_browser_url(url, true),
        format!("{ANSI_BOLD_BRIGHT_CYAN}{url}{ANSI_RESET}")
    );
}

#[tokio::test]
async fn performance_trace_endpoint_authenticates_validates_and_records_browser_stages() {
    let mut state = routing_test_state();
    state.public_access = true;
    let path = std::env::temp_dir().join(unique_web_id("timem_web_runtime.log"));
    state.runtime_log = RuntimeLog::with_path_and_limit(path.clone(), 4096);
    let denied = performance_trace(
        State((state.clone(), 13764)),
        Query(AuthQuery { token: None }),
        HeaderMap::new(),
        Json(ClientPerformanceTrace {
            stage: "browser_turn_updated".to_string(),
            session_id: "session_a".to_string(),
            command_id: "command_1".to_string(),
            turn_id: Some("turn_1".to_string()),
            elapsed_ms: Some(12.5),
            event_count: Some(7),
        }),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    assert!(!path.exists());

    let recorded = performance_trace(
        State((state.clone(), 13764)),
        Query(AuthQuery {
            token: Some("test".to_string()),
        }),
        HeaderMap::new(),
        Json(ClientPerformanceTrace {
            stage: "browser_turn_updated".to_string(),
            session_id: "session_a".to_string(),
            command_id: "command_1".to_string(),
            turn_id: Some("turn_1".to_string()),
            elapsed_ms: Some(12.5),
            event_count: Some(7),
        }),
    )
    .await;
    assert_eq!(recorded.status(), StatusCode::NO_CONTENT);
    let value: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
    assert_eq!(value["stage"], "browser_turn_updated");
    assert_eq!(value["fields"]["command_id"], "command_1");
    assert_eq!(value["fields"]["event_count"], 7);

    let rejected = performance_trace(
        State((state, 13764)),
        Query(AuthQuery {
            token: Some("test".to_string()),
        }),
        HeaderMap::new(),
        Json(ClientPerformanceTrace {
            stage: "server_forgery".to_string(),
            session_id: "session_a".to_string(),
            command_id: "command_2".to_string(),
            turn_id: None,
            elapsed_ms: None,
            event_count: None,
        }),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_file(path);
}

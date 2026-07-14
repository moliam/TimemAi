use super::*;
use agent_core::session_runtime::ModelClient;
use agent_core::{
    core_initialized_topic_event, CoreProfile, CoreSessionState, CoreSessionWorkerWorkspace,
    CoreTopic, CoreTopicEvent, LlmResponse, TurnOutcome, UsageStats, CORE_TOPIC_ACTION,
    CORE_TOPIC_MODEL_RESPONSE,
};
use std::sync::atomic::AtomicUsize;
use std::thread;
use std::time::{Duration, Instant};

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
    assert!(options.open_browser);

    let headless = WebLaunchOptions::parse(&["--no-open".to_string()]).unwrap();
    assert!(!headless.open_browser);
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
    assert!(!session_context(&state, session_id, &[])
        .unwrap()
        .unwrap()
        .contains("upload_1_notes.md"));

    rollback_web_turn(&state, session_id, &turn.turn_id, vec![attachment.clone()]);
    assert_eq!(
        state.sessions.lock().unwrap()[session_id].attachments,
        vec![attachment]
    );
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

    let attachment_remove = serde_json::from_str::<ClientCommand>(
        r#"{"type":"attachment_remove","session_id":"session_1","attachment_id":"upload_1"}"#,
    )
    .unwrap();
    assert!(matches!(
        attachment_remove,
        ClientCommand::AttachmentRemove { .. }
    ));

    assert!(serde_json::from_str::<ClientCommand>(r#"{"type":"shell_exec"}"#).is_err());
}

#[test]
fn browser_open_uses_a_direct_argument_without_shell_interpolation() {
    let url = "http://127.0.0.1:12345/?token=test&name=a b";
    let (_program, args) = browser_command(url);
    assert_eq!(args.last().and_then(|arg| arg.to_str()), Some(url));
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
fn runtime_update_refreshes_new_session_defaults_without_rewriting_existing_sessions() {
    let state = routing_test_state();
    let existing_session_id = state
        .sessions
        .lock()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let existing_profile = state.sessions.lock().unwrap()[&existing_session_id]
        .runtime_profile
        .clone();
    let mut events = state.events.subscribe();

    assert!(handle_command(
        &state,
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
    } = events.try_recv().unwrap()
    else {
        panic!("runtime update must publish refreshed session defaults")
    };
    assert_eq!(key, "TIMEM_MODEL");
    assert_eq!(value, "future-session-model");
    assert_eq!(
        session_env_defaults.get("TIMEM_MODEL").map(String::as_str),
        Some("future-session-model")
    );
    assert!(!session_env_defaults.contains_key("TIMEM_API_KEY"));
    assert_eq!(
        state.sessions.lock().unwrap()[&existing_session_id].runtime_profile,
        existing_profile
    );
}

#[test]
fn generated_local_access_token_has_expected_entropy_shape() {
    let token = generate_token().unwrap();
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|character| character.is_ascii_hexdigit()));
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
    assert!(policy.contains("object-src 'none'"));
    assert!(policy.contains("frame-ancestors 'none'"));
}

#[test]
fn workspace_snapshot_deduplicates_registered_current_directory() {
    let template = WorkerTemplate {
        settings: Arc::new(Mutex::new(RuntimeSettings {
            config: ProviderConfig {
                provider: "test".to_string(),
                model: "test-model".to_string(),
                base_url: "http://127.0.0.1".to_string(),
                api_key: "test".to_string(),
                timeout_secs: 1,
                max_llm_output_tokens: 1_024,
                max_llm_input_tokens: 10_000,
                api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
                response_protocol: ResponseProtocolKind::default(),
            },
            bash_approval_mode: BashApprovalMode::Ask,
            work_instruction_mode: WorkInstructionLoadMode::Off,
        })),
        memory_dir: PathBuf::from("/tmp/memory"),
        audit_file: PathBuf::from("/tmp/audit.json"),
        data_dir: PathBuf::from("/tmp/data"),
        env: BTreeMap::new(),
        current_dir: PathBuf::from("/work/a"),
        workspace_dirs: vec![PathBuf::from("/work/a"), PathBuf::from("/work/b")],
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
    template.memory_dir = root.join("memory");
    template.audit_file = root.join("audit.json");
    state.template = Arc::new(template);

    let event = handle_command(
        &state,
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
fn unnamed_web_session_uses_session_name_while_worker_keeps_core_identity() {
    let mut state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_default_session_name_{}", now_ms()));
    std::fs::create_dir_all(&root).unwrap();
    let mut template = (*state.template).clone();
    template.current_dir = root.clone();
    template.workspace_dirs = vec![root.clone()];
    template.data_dir = root.join("data");
    template.memory_dir = root.join("memory");
    template.audit_file = root.join("audit.json");
    state.template = Arc::new(template);

    let session_id = create_session(
        &state,
        None,
        Some(root.display().to_string()),
        BTreeMap::new(),
    )
    .unwrap();
    let sessions = state.sessions.lock().unwrap();
    let session = &sessions[&session_id];
    assert_eq!(session.display_name, format!("Session{}", session.ordinal));
    assert_eq!(session.workers.len(), 1);
    assert_eq!(session.workers[0].display_name, "ID0");
    drop(sessions);

    let renamed = handle_command(
        &state,
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
    assert_eq!(
        state.sessions.lock().unwrap()[&session_id].display_name,
        "Build session"
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
    template.memory_dir = root.join("memory");
    template.audit_file = root.join("audit.json");
    state.template = Arc::new(template);

    let first_env = BTreeMap::from([
        (
            "TIMEM_GATEWAY_PROVIDER".to_string(),
            "anthropic".to_string(),
        ),
        ("TIMEM_MODEL".to_string(), "claude-session-a".to_string()),
        ("TIMEM_API_PROTOCOL".to_string(), "anthropic".to_string()),
        ("TIMEM_RESPONSE_PROTOCOL".to_string(), "json".to_string()),
        ("TIMEM_API_KEY".to_string(), "session-a-secret".to_string()),
        ("TIMEM_MAX_LLM_INPUT".to_string(), "128K".to_string()),
    ]);
    let second_env = BTreeMap::from([
        ("TIMEM_MODEL".to_string(), "qwen-session-b".to_string()),
        (
            "TIMEM_RESPONSE_PROTOCOL".to_string(),
            "markdown".to_string(),
        ),
        ("TIMEM_MAX_LLM_INPUT".to_string(), "64K".to_string()),
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
    assert_eq!(sessions[&first].runtime_profile.provider, "anthropic");
    assert_eq!(sessions[&first].runtime_profile.model, "claude-session-a");
    assert_eq!(sessions[&first].runtime_profile.api_protocol, "anthropic");
    assert_eq!(sessions[&first].runtime_profile.response_protocol, "json");
    assert_eq!(sessions[&first].max_llm_input_tokens, 128_000);
    assert_eq!(sessions[&second].runtime_profile.provider, "test");
    assert_eq!(sessions[&second].runtime_profile.model, "qwen-session-b");
    assert_eq!(
        sessions[&second].runtime_profile.response_protocol,
        "markdown"
    );
    assert_eq!(sessions[&second].max_llm_input_tokens, 64_000);
    drop(sessions);

    let defaults = state.template.settings.lock().unwrap();
    assert_eq!(defaults.config.provider, "test");
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
                            lifecycle.profile.provider,
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
        Some(&(
            "anthropic".to_string(),
            "claude-session-a".to_string(),
            "json".to_string(),
            128_000,
        ))
    );
    assert_eq!(
        lifecycle_profiles.get(&second),
        Some(&(
            "test".to_string(),
            "qwen-session-b".to_string(),
            "markdown".to_string(),
            64_000,
        ))
    );
    assert!(!lifecycle_leaked_secret);
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
    let config = ProviderConfig {
        provider: "test".to_string(),
        model: "test-model".to_string(),
        base_url: "http://127.0.0.1".to_string(),
        api_key: "test".to_string(),
        timeout_secs: 1,
        max_llm_output_tokens: 1_024,
        max_llm_input_tokens: 10_000,
        api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
        response_protocol: ResponseProtocolKind::Xml,
    };
    let template = WorkerTemplate {
        settings: Arc::new(Mutex::new(RuntimeSettings {
            config,
            bash_approval_mode: BashApprovalMode::Ask,
            work_instruction_mode: WorkInstructionLoadMode::Off,
        })),
        memory_dir: PathBuf::from("/tmp/memory"),
        audit_file: PathBuf::from("/tmp/audit.json"),
        data_dir: PathBuf::from("/tmp/data"),
        env: BTreeMap::new(),
        current_dir: PathBuf::from("/work"),
        workspace_dirs: vec![PathBuf::from("/work")],
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
        manager: Arc::new(Mutex::new(CoreSessionWorkerManager::new())),
        template: Arc::new(template),
        events,
        sessions: Arc::new(Mutex::new(sessions)),
    }
}

fn test_web_session(session_id: &str, ordinal: u32, display_name: String) -> WebSession {
    let context_id = test_context_id(session_id);
    let worker_id = test_worker_id(session_id);
    let settings = test_runtime_settings();
    WebSession {
        session_id: session_id.to_string(),
        display_name,
        ordinal,
        state: "ready".to_string(),
        current_dir: "/work".to_string(),
        max_llm_input_tokens: 10_000,
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
        messages: Vec::new(),
        turns: Vec::new(),
        active_turn_id: None,
        pending_completion_message_id: None,
        work_instruction_mode: WorkInstructionLoadMode::Off,
        work_instruction_allowed: None,
        pending_work_instruction_turn: None,
        runtime: WebSessionRuntime {
            settings,
            env: BTreeMap::new(),
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
        config: ProviderConfig {
            provider: "test".to_string(),
            model: "model".to_string(),
            base_url: "http://127.0.0.1:9".to_string(),
            api_key: "test".to_string(),
            timeout_secs: 1,
            max_llm_output_tokens: 1_024,
            max_llm_input_tokens: 10_000,
            api_protocol: agent_core::ApiProtocol::OpenAiCompatible,
            response_protocol: ResponseProtocolKind::Xml,
        },
        bash_approval_mode: BashApprovalMode::Ask,
        work_instruction_mode: WorkInstructionLoadMode::Off,
    }
}

fn test_runtime_profile() -> WebSessionRuntimeProfile {
    WebSessionRuntimeProfile {
        provider: "test".to_string(),
        model: "model".to_string(),
        api_protocol: "openai-compatible".to_string(),
        response_protocol: "xml".to_string(),
        base_url: "http://127.0.0.1:9".to_string(),
        timeout_secs: 1,
        max_llm_input_tokens: 10_000,
        max_llm_output_tokens: 1_024,
        bash_approval: "ask".to_string(),
        work_instructions: "off".to_string(),
    }
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
            Ok(event) => events.push(event),
            Err(broadcast::error::TryRecvError::Empty)
            | Err(broadcast::error::TryRecvError::Closed) => return events,
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                panic!("topic routing test exceeded broadcast capacity: skipped {skipped} events")
            }
        }
    }
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
        CoreSessionWorkerEvent::ModelRequest { round: 1 },
    );
    handle_scoped_worker_event(
        &state,
        session_id,
        &sub_context_id,
        &sub_worker_id,
        CoreSessionWorkerEvent::ModelRequest { round: 1 },
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

struct CancelThenFinishModel {
    calls: Arc<AtomicUsize>,
    entered: Arc<AtomicUsize>,
    final_text: &'static str,
}

impl ModelClient for CancelThenFinishModel {
    fn call_model(
        &mut self,
        _config: &ProviderConfig,
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
            content: format!(
                "<response><free_talk>done</free_talk><final_answer>{}</final_answer></response>",
                self.final_text
            ),
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
                name: "test".to_string(),
                provider: "test".to_string(),
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
        ClientCommand::TurnCancel {
            session_id: session_id.to_string(),
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
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "continue".to_string(),
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
            usage: UsageStats {
                prompt_tokens: 8_200,
                completion_tokens: 123,
                cached_tokens: 6_400,
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
    assert_eq!(active.events.len(), 1);
    assert_eq!(active.events[0].payload["kind"], "model_response");
    assert_eq!(active.events[0].payload["usage"]["prompt_tokens"], 8_200);
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
            name: "test".to_string(),
            provider: "local".to_string(),
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
    handle_command(
        &state,
        ClientCommand::TurnSupplement {
            session_id: session_id.clone(),
            text: "Use the second verification path".to_string(),
        },
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
fn active_turn_event_windows_are_bounded_and_session_isolated() {
    const SESSION_COUNT: usize = 5;
    const EVENTS_PER_SESSION: usize = MAX_TURN_EVENTS + 75;
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
        assert_eq!(turn.events.len(), MAX_TURN_EVENTS);
        assert_eq!(
            turn.events.first().unwrap().payload["sequence"],
            EVENTS_PER_SESSION - MAX_TURN_EVENTS
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

struct TaggedFinalModel(&'static str);

impl ModelClient for TaggedFinalModel {
    fn call_model(
        &mut self,
        _config: &ProviderConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Ok(LlmResponse {
            content: format!(
                "<response><free_talk>done</free_talk><final_answer>{}</final_answer></response>",
                self.0
            ),
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

struct ChangeCwdModel {
    new_path: String,
    round: u8,
}

impl ModelClient for ChangeCwdModel {
    fn call_model(
        &mut self,
        _config: &ProviderConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.round += 1;
        let content = if self.round == 1 {
            format!(
                "<response><working_still_action><action_json><![CDATA[[{{\"self_tool\":{{\"type\":\"cwd\",\"op\":\"chg_cwd\",\"new_path\":{}}}}}]]]></action_json></working_still_action></response>",
                serde_json::to_string(&self.new_path).unwrap()
            )
        } else {
            "<response><final_answer>cwd updated</final_answer></response>".to_string()
        };
        Ok(LlmResponse {
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
            name: "test".to_string(),
            provider: "test".to_string(),
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
fn real_worker_cwd_tool_call_updates_web_session_state() {
    let state = routing_test_state();
    let root = std::env::temp_dir().join(format!("timem_web_cwd_e2e_{}", now_ms()));
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let nested = std::fs::canonicalize(nested).unwrap();
    let mut core = AgentCore::new(
        STATIC_PROMPT,
        CoreProfile {
            name: "test".to_string(),
            provider: "test".to_string(),
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
            ChangeCwdModel {
                new_path: nested.display().to_string(),
                round: 0,
            },
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
        if state.sessions.lock().unwrap()[&session_id].current_dir == nested.display().to_string() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        state.sessions.lock().unwrap()[&session_id].current_dir,
        nested.display().to_string()
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
    let context = session_context(&state, &session_id, &[]).unwrap().unwrap();
    assert!(context.contains("host: local_web"));
    assert!(!context.contains("MUST_NOT_REACH_MODEL"));
}

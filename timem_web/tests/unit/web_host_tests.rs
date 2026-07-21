use super::*;
use agent_core::session_runtime::ModelClient;
use agent_core::session_store::read_all_history_records;
use agent_core::{
    core_initialized_topic_event, CoreProfile, CoreSessionState, CoreSessionWorkerWorkspace,
    CoreTopic, CoreTopicEvent, LlmResponse, TurnOutcome, UsageStats, CORE_TOPIC_ACTION,
    CORE_TOPIC_MODEL_RESPONSE,
};
use std::sync::atomic::AtomicUsize;
use std::thread;
use std::time::{Duration, Instant};

const TEST_PORT: u16 = 12345;

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
    assert!(options.open_browser);

    let headless =
        WebLaunchOptions::parse(&["--no-open".to_string(), "--public".to_string()]).unwrap();
    assert!(!headless.open_browser);
    assert!(headless.public_access);
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
            token: Some("wrong".to_string())
        },
        &HeaderMap::new()
    ));
    assert!(authorized(
        &state,
        &AuthQuery {
            token: Some("test".to_string())
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
}

#[tokio::test]
async fn static_web_entry_requires_token_or_authenticated_cookie() {
    let state = routing_test_state();
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
                openai_compatible: agent_core::OpenAiCompatibleOptions::default(),
            },
            bash_approval_mode: BashApprovalMode::Ask,
            work_instruction_mode: WorkInstructionLoadMode::Off,
        })),
        data_dir: PathBuf::from("/tmp/data"),
        initial_space: ".test_mem".to_string(),
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
fn unnamed_web_session_uses_session_name_while_worker_keeps_core_identity() {
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
    assert_eq!(session.display_name, format!("Session{}", session.ordinal));
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
    state.template = Arc::new(template);
    set_test_mem(&state, root.join("data"), ".test_mem");

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

    // Simulate a session persisted by the previous Web host, where `env`
    // contained the fully resolved profile and carried no override provenance.
    let store = current_session_store(&state).unwrap();
    let mut legacy = store.load_session(&session_id).unwrap().unwrap();
    legacy.env_overrides = None;
    legacy
        .env
        .insert("TIMEM_MODEL".to_string(), "stale-model".to_string());
    store.upsert_session(&legacy).unwrap();

    // A default-inheriting session must follow the environment/configuration
    // used by the newly started host instead of pinning its old resolved model.
    template.settings.lock().unwrap().config.model = "model-from-new-env".to_string();

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
    assert_eq!(restored_session.runtime_profile.model, "model-from-new-env");
    drop(sessions);

    let context = session_context(&restarted, &session_id, &[])
        .unwrap()
        .expect("restored session should inject resume context");
    assert!(context.contains("This session was restored"));
    assert!(context.contains("raw_chat_history.jsonl"));
    assert!(context.contains("format: JSONL, one record per line."));
    let context_after_first_use = session_context(&restarted, &session_id, &[])
        .unwrap()
        .unwrap_or_default();
    assert!(!context_after_first_use.contains("This session was restored"));
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
    store
        .append_history_record(
            &session_id,
            &ChatHistoryRecord::Message {
                role: ChatHistoryRole::User,
                turn_id: turn_id.to_string(),
                created_at_ms: 1,
                kind: Some("task".to_string()),
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
                    created_at_ms: index + 2,
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
                created_at_ms: 205,
                kind: Some("supplement".to_string()),
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
fn restored_session_keeps_only_explicit_non_secret_runtime_overrides() {
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
    assert!(stored.env.is_empty());

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
        "new-process-secret"
    );
    assert!(restored.runtime.settings.config.openai_compatible.stream);
}

#[test]
fn restored_web_turns_follow_history_time_not_turn_id_lexical_order() {
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_10".to_string(),
            created_at_ms: 10,
            kind: None,
            content: "first by time".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_10".to_string(),
            created_at_ms: 11,
            kind: None,
            content: "first answer".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_2".to_string(),
            created_at_ms: 20,
            kind: None,
            content: "second by time".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_2".to_string(),
            created_at_ms: 21,
            kind: None,
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
fn restored_web_turns_preserve_user_entry_kinds() {
    let records = vec![
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 10,
            kind: Some("task".to_string()),
            content: "original task".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 11,
            kind: Some("supplement".to_string()),
            content: "mid-turn supplement".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 12,
            kind: Some("approval".to_string()),
            content: "approved request".to_string(),
        },
        ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_1".to_string(),
            created_at_ms: 13,
            kind: Some("unknown_legacy_kind".to_string()),
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
            content: "first valid".to_string(),
        })
        .unwrap(),
        "partial json from interrupted append".to_string(),
        serde_json::to_string(&ChatHistoryRecord::Message {
            role: ChatHistoryRole::Assistant,
            turn_id: "turn_1".to_string(),
            created_at_ms: 1,
            kind: None,
            content: "second valid".to_string(),
        })
        .unwrap(),
        serde_json::to_string(&ChatHistoryRecord::Message {
            role: ChatHistoryRole::User,
            turn_id: "turn_2".to_string(),
            created_at_ms: 2,
            kind: None,
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
fn snapshot_reports_the_active_mem_space_and_paths() {
    let state = routing_test_state();
    let snapshot = snapshot_for(&state, TEST_PORT);

    assert_eq!(snapshot.server.mem.space, ".test_mem");
    assert!(snapshot.server.mem.data_dir.contains("timem_web_data_test"));
    assert!(snapshot.server.mem.space_dir.ends_with(".test_mem"));
    assert!(snapshot.server.mem.memory_dir.ends_with(".test_mem/memory"));
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
    template.initial_space = "alpha".to_string();
    state.template = Arc::new(template);
    set_test_mem(&state, data_dir.clone(), "alpha");
    state.sessions.lock().unwrap().clear();

    let alpha_session = create_session(
        &state,
        Some("Alpha work".to_string()),
        None,
        BTreeMap::new(),
    )
    .unwrap();
    start_web_turn(&state, &alpha_session, "alpha task").unwrap();

    set_test_mem(&state, data_dir.clone(), "beta");
    state.sessions.lock().unwrap().clear();
    let beta_session =
        create_session(&state, Some("Beta work".to_string()), None, BTreeMap::new()).unwrap();
    start_web_turn(&state, &beta_session, "beta task").unwrap();

    set_test_mem(&state, data_dir, "alpha");
    state.sessions.lock().unwrap().clear();
    restore_stored_sessions(&state).unwrap();
    assert!(state.sessions.lock().unwrap().contains_key(&alpha_session));
    assert!(!state.sessions.lock().unwrap().contains_key(&beta_session));

    let mut events = state.events.subscribe();
    assert!(handle_command(
        &state,
        TEST_PORT,
        ClientCommand::MemSwitch {
            space: "beta".to_string(),
        },
    )
    .unwrap()
    .is_none());

    let WireEvent::Hello { snapshot } = events.try_recv().unwrap() else {
        panic!("expected hello snapshot after mem switch")
    };
    assert_eq!(snapshot.server.mem.space, "beta");
    assert!(snapshot
        .sessions
        .iter()
        .any(|session| session.session_id == beta_session));
    assert!(!snapshot
        .sessions
        .iter()
        .any(|session| session.session_id == alpha_session));
}

#[test]
fn mem_switch_rejects_paths_and_parent_traversal() {
    let state = routing_test_state();
    for space in [
        "",
        ".",
        "..",
        "../other",
        "/tmp/mem",
        "alpha/beta",
        "alpha..beta",
    ] {
        assert!(handle_command(
            &state,
            TEST_PORT,
            ClientCommand::MemSwitch {
                space: space.to_string(),
            },
        )
        .is_err());
    }
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
        openai_compatible: agent_core::OpenAiCompatibleOptions::default(),
    };
    let template = WorkerTemplate {
        settings: Arc::new(Mutex::new(RuntimeSettings {
            config,
            bash_approval_mode: BashApprovalMode::Ask,
            work_instruction_mode: WorkInstructionLoadMode::Off,
        })),
        data_dir: std::env::temp_dir().join(unique_web_id("timem_web_data_test")),
        initial_space: ".test_mem".to_string(),
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
        public_access: false,
        manager: Arc::new(Mutex::new(CoreSessionWorkerManager::new())),
        mem: Arc::new(Mutex::new(
            WebMemState::new(template.data_dir.clone(), template.initial_space.clone()).unwrap(),
        )),
        template: Arc::new(template),
        events,
        sessions: Arc::new(Mutex::new(sessions)),
    }
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
        display_name,
        ordinal,
        state: "ready".to_string(),
        current_dir: "/work".to_string(),
        max_llm_input_tokens: 10_000,
        tools: Vec::new(),
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
        consumed_attachment_ids: BTreeSet::new(),
        messages: Vec::new(),
        turns: Vec::new(),
        history_before_cursor: None,
        history_has_more: false,
        resume_notice_pending: false,
        active_turn_id: None,
        pending_completion_message_id: None,
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
            openai_compatible: agent_core::OpenAiCompatibleOptions::default(),
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
        TEST_PORT,
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
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.to_string(),
            text: "continue".to_string(),
            input_kind: None,
            source_turn_id: None,
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
        TEST_PORT,
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

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "also use this attached context".to_string(),
            input_kind: None,
            source_turn_id: None,
        },
    )
    .unwrap()
    .expect("active submit should become an attached supplement");

    let WireEvent::TurnUpdated { turn: updated, .. } = event else {
        panic!("expected turn update")
    };
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
    assert!(uploaded_files_context(&updated.user_entries[1].attachments)
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
fn turn_user_entries_are_persisted_with_raw_text_and_semantic_kind() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "HISTORY_KIND_WRITE");
    let turn = start_web_turn(&state, &session_id, "initial task").unwrap();

    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSupplement {
            session_id: session_id.clone(),
            text: "mid-turn correction".to_string(),
        },
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
        },
    )
    .unwrap()
    .is_none());
}

#[test]
fn rapid_submit_during_an_active_turn_is_treated_as_a_supplement() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "SUBMIT_RACE");
    let first = start_web_turn(&state, &session_id, "initial upload task").unwrap();

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "stop if this is still running".to_string(),
            input_kind: None,
            source_turn_id: None,
        },
    )
    .unwrap()
    .expect("active submit should return the updated active turn");

    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("expected turn update")
    };
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
        handle_command(
            &state,
            TEST_PORT,
            ClientCommand::TurnSubmit {
                session_id: session_id.clone(),
                text: text.to_string(),
                input_kind: None,
                source_turn_id: None,
            },
        )
        .unwrap()
        .expect("active submit should update the current turn");
    }
    handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSupplement {
            session_id: session_id.clone(),
            text: "explicit supplement command stays in the same turn".to_string(),
        },
    )
    .unwrap()
    .expect("active supplement should update the current turn");

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

#[test]
fn rapid_stop_and_send_clicks_during_active_turn_do_not_break_the_session() {
    let state = routing_test_state();
    let session_id = register_real_worker(&state, "STOP_AND_SEND_RACE");
    let first = start_web_turn(&state, &session_id, "copy a large artifact").unwrap();

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "first correction before stopping".to_string(),
            input_kind: None,
            source_turn_id: None,
        },
    )
    .unwrap()
    .expect("active submit should update the active turn");
    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("expected turn update")
    };
    assert_eq!(turn.turn_id, first.turn_id);

    for _ in 0..3 {
        assert!(handle_command(
            &state,
            TEST_PORT,
            ClientCommand::TurnCancel {
                session_id: session_id.clone(),
            },
        )
        .unwrap()
        .is_none());
    }

    let event = handle_command(
        &state,
        TEST_PORT,
        ClientCommand::TurnSubmit {
            session_id: session_id.clone(),
            text: "late correction from another rapid send click".to_string(),
            input_kind: None,
            source_turn_id: None,
        },
    )
    .unwrap()
    .expect("late active submit should still update the active turn");
    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("expected turn update")
    };
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

struct ToolGenPromptCaptureModel {
    prompts: Arc<Mutex<Vec<String>>>,
}

impl ModelClient for ToolGenPromptCaptureModel {
    fn call_model(
        &mut self,
        _config: &ProviderConfig,
        prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        Ok(LlmResponse {
            content: "<response><toolgen_retrospect>No reusable tool was published.</toolgen_retrospect><final_answer>ToolGen review complete.</final_answer></response>".to_string(),
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
        _config: &ProviderConfig,
        prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        let content = if self.calls == 1 {
            "<response><free_talk>Checking the reusable workflow.</free_talk><working_still_action><action_json><![CDATA[[{\"run_bash\":{\"cmd\":\"printf toolgen-host-check\",\"timeout_ms\":5000}}]]]></action_json></working_still_action></response>".to_string()
        } else if prompt.contains("Action result: toolgen\nop: publish\nstatus: ready") {
            "<response><toolgen_retrospect>Published host-tool after runtime validation.</toolgen_retrospect><final_answer>ToolGen host workflow completed.</final_answer></response>".to_string()
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
                "<response><free_talk>Publishing the verified draft.</free_talk><working_still_action><action_json><![CDATA[[{{\"toolgen\":{{\"op\":\"publish\",\"draft_path\":{}}}}}]]]></action_json></working_still_action></response>",
                serde_json::to_string(draft).unwrap()
            )
        };
        Ok(LlmResponse {
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

fn register_toolgen_capture_worker(state: &AppState, prompts: Arc<Mutex<Vec<String>>>) -> String {
    let ordinal = state.sessions.lock().unwrap().len() as u32;
    let session_id = unique_web_id("toolgen_session");
    let context_id = test_context_id(&session_id);
    let worker_dir = std::env::temp_dir().join(format!("timem_web_toolgen_{}", now_ms()));
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
            name: "test".to_string(),
            provider: "test".to_string(),
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
        },
    )
    .unwrap()
    .unwrap();
    let WireEvent::TurnUpdated { turn, .. } = event else {
        panic!("manual ToolGen must create a Web turn");
    };
    assert_eq!(turn.state, "working");
    assert!(turn.turn_id.starts_with("web_toolgen_turn_"));
    assert!(turn.user_entries.is_empty());
    drive_worker_until_session_ready(&state, &session_id, &prompts);

    let prompt = prompts.lock().unwrap().last().unwrap().clone();
    assert!(prompt.contains("[TOOL_GEN_TASK] Please extract the reusable function"));
    assert!(!prompt.contains("Referenced completed turn id:"));
    assert!(!prompt.contains("Completed task:"));
    assert!(!prompt.contains("Completed task result:"));
    assert!(!prompt.contains("Observed action evidence:"));
    assert!(prompt.contains("## ToolGen test"));
    assert!(!prompt.contains("ToolGen test_TOOLGEN"));
    let marker = "[TOOL_GEN_TASK] Please extract the reusable function";
    let delta_start = prompt[..prompt.find(marker).unwrap()]
        .rfind("[BEGIN DELTA]")
        .unwrap();
    let delta_end = prompt[delta_start..].find("[END DELTA]").unwrap() + delta_start;
    let toolgen_delta = &prompt[delta_start..delta_end];
    assert!(toolgen_delta.contains("## SYSTEM"));
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
    let delta_start = prompt[..guidance_at].rfind("[BEGIN DELTA]").unwrap();
    let delta_end = prompt[guidance_at..].find("[END DELTA]").unwrap() + guidance_at;
    let toolgen_delta = &prompt[delta_start..delta_end];
    assert!(toolgen_delta.contains("## USER"));
    let system_at = toolgen_delta.find("## SYSTEM").unwrap();
    let user_at = toolgen_delta.find("## USER").unwrap();
    assert!(
        system_at < user_at,
        "the fixed ToolGen SYSTEM instruction must precede optional USER guidance"
    );
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
    assert!(!session_context(&state, "session_b", &[])
        .unwrap()
        .unwrap()
        .contains("Previously accumulated reusable scripts"));
}

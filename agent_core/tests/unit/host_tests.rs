use super::*;

#[test]
fn noop_turn_ui_defaults_are_noninteractive() {
    let mut ui = NoopTurnUi;
    assert!(!ui.is_cancel_requested());
    assert!(!ui.take_cancel_request());
    assert!(ui.drain_user_supplements().is_empty());
    assert!(ui.request_round_limit_continue(RoundLimitDecisionRequest::new(50)));
    assert!(ui.can_request_output_expansion());
    assert!(ui.request_expand_output_tokens(OutputExpansionRequest::new(10_000)));
}

#[test]
fn turn_ui_default_callbacks_follow_host_decision_policy() {
    let mut ui = NoopTurnUi;
    let approval = ApprovalRequest {
        approval_id: "approval_1".to_string(),
        action: "run_bash".to_string(),
        command: "printf ok".to_string(),
        reason: "requires_approval".to_string(),
        risk: "local_command".to_string(),
    };
    let round = RoundLimitDecisionRequest::new(20);
    let expansion = OutputExpansionRequest::new(10_000);

    assert_eq!(
        ui.request_user_approval(&approval),
        HostDecisionRequest::UserApproval(approval)
            .safe_default()
            .as_bool()
    );
    assert_eq!(
        ui.request_round_limit_continue(round),
        HostDecisionRequest::RoundLimitContinue(round)
            .safe_default()
            .as_bool()
    );
    assert_eq!(
        ui.request_expand_output_tokens(expansion),
        HostDecisionRequest::OutputExpansion(expansion)
            .safe_default()
            .as_bool()
    );
}

#[test]
fn turn_ui_specific_requests_delegate_to_generic_host_decision() {
    #[derive(Default)]
    struct DeclineAll {
        seen: Vec<&'static str>,
    }

    impl TurnUi for DeclineAll {
        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            self.seen.push(request.kind());
            HostDecision::Decline
        }
    }

    let mut ui = DeclineAll::default();
    let approval = ApprovalRequest {
        approval_id: "approval_1".to_string(),
        action: "run_bash".to_string(),
        command: "printf ok".to_string(),
        reason: "requires_approval".to_string(),
        risk: "local_command".to_string(),
    };

    assert!(!ui.request_user_approval(&approval));
    assert!(!ui.request_round_limit_continue(RoundLimitDecisionRequest::new(20)));
    assert!(!ui.request_expand_output_tokens(OutputExpansionRequest::new(10_000)));
    assert_eq!(
        ui.seen,
        vec!["user_approval", "round_limit_continue", "output_expansion"]
    );
}

#[test]
fn turn_ui_request_topic_emits_blocking_event_and_resolves_reply() {
    #[derive(Default)]
    struct TopicAwareUi {
        seen_topics: Vec<String>,
        seen_blocking: Vec<bool>,
        seen_request_ids: Vec<String>,
    }

    impl TurnUi for TopicAwareUi {
        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            for event in events {
                self.seen_topics.push(event.topic.name.clone());
                self.seen_blocking.push(event.is_blocking_request());
                if let Some(request_id) = event.request_id() {
                    self.seen_request_ids.push(request_id.to_string());
                }
            }
        }

        fn request_host_decision(&mut self, request: HostDecisionRequest) -> HostDecision {
            assert_eq!(request.kind(), "round_limit_continue");
            HostDecision::Decline
        }
    }

    let mut ui = TopicAwareUi::default();
    let decision = ui.request_host_decision_topic(
        "session_a",
        HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
    );

    assert_eq!(decision, HostDecision::Decline);
    assert_eq!(
        ui.seen_topics,
        vec![CORE_TOPIC_ROUND_LIMIT_REQUEST.to_string()]
    );
    assert_eq!(ui.seen_blocking, vec![true]);
    assert_eq!(ui.seen_request_ids.len(), 1);
    assert!(ui.seen_request_ids[0].starts_with("request_round_limit_continue_"));
}

#[test]
fn turn_ui_request_topic_requires_matching_topic_reply_before_resuming() {
    #[derive(Default)]
    struct BadReplyUi {
        seen_event: Option<CoreTopicEvent>,
    }

    impl TurnUi for BadReplyUi {
        fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
            self.seen_event = events.first().cloned();
        }

        fn reply_to_core_topic(&mut self, event: &CoreTopicEvent) -> Option<TopicReply> {
            TopicReply::for_decision_request(event, HostDecision::Decline)
                .map(|reply| reply.with_request_id("wrong_request_id"))
        }
    }

    let mut ui = BadReplyUi::default();
    let decision = ui.request_host_decision_topic(
        "session_a",
        HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
    );

    assert_eq!(
        decision,
        HostDecision::Accept,
        "mismatched replies must fall back to the request safe default"
    );
    let event = ui.seen_event.expect("request topic should be published");
    assert!(event.is_blocking_request());
    assert_eq!(event.session_id, "session_a");
    assert_eq!(event.topic.name, CORE_TOPIC_ROUND_LIMIT_REQUEST);
    assert!(event.request_id().is_some());
}

#[test]
fn turn_ui_decision_requests_are_structured_and_ui_neutral() {
    let round = RoundLimitDecisionRequest::new(20);
    assert_eq!(round.max_rounds, 20);
    assert_eq!(round.recharge_rounds, 20);
    assert!(round.keep_task_context);

    let output = OutputExpansionRequest::new(10_000);
    assert_eq!(output.current_tokens, 10_000);
    assert_eq!(output.increment_tokens, 10_000);
    assert_eq!(output.expanded_tokens(), 20_000);
    assert!(output.retry_same_turn);

    let debug = format!("{round:?} {output:?}");
    for forbidden in ["继续", "停止", "增加", "重试", "[", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core decision request leaked shell/ui text {forbidden:?}: {debug}"
        );
    }
}

#[test]
fn host_decision_request_exposes_ui_neutral_policy_metadata() {
    let requests = [
        HostDecisionRequest::UserApproval(ApprovalRequest {
            approval_id: "approval_1".to_string(),
            action: "run_bash".to_string(),
            command: "printf ok".to_string(),
            reason: "requires_approval".to_string(),
            risk: "local_command".to_string(),
        }),
        HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
        HostDecisionRequest::OutputExpansion(OutputExpansionRequest::new(10_000)),
        HostDecisionRequest::StaleContextContinue(StaleContextDecisionRequest {
            idle: Duration::from_secs(3 * 60 * 60),
            dynamic_context_tokens: 10_001,
            continue_keeps_dynamic_context: true,
            decline_clears_dynamic_context: true,
        }),
        HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
            directory: "/tmp/project".into(),
            file_names: vec!["AGENTS.md".to_string()],
        }),
    ];

    assert_eq!(requests[0].kind(), "user_approval");
    assert_eq!(requests[1].kind(), "round_limit_continue");
    assert_eq!(requests[2].kind(), "output_expansion");
    assert_eq!(requests[3].kind(), "stale_context_continue");
    assert_eq!(requests[4].kind(), "work_instruction_load");
    assert!(requests[..4]
        .iter()
        .all(|request| request.timeout().is_none()));
    assert_eq!(
        requests[4].timeout(),
        Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT)
    );
    assert_eq!(requests[0].safe_default(), HostDecisionDefault::Accept);
    assert_eq!(requests[1].safe_default(), HostDecisionDefault::Accept);
    assert_eq!(requests[2].safe_default(), HostDecisionDefault::Accept);
    assert_eq!(requests[3].safe_default(), HostDecisionDefault::Decline);
    assert_eq!(requests[4].safe_default(), HostDecisionDefault::Decline);

    let debug = format!("{requests:?}");
    for forbidden in ["继续", "停止", "加载", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core host request metadata leaked UI text {forbidden:?}: {debug}"
        );
    }
}

#[test]
fn host_decision_requests_can_be_published_as_topic_events() {
    let request = HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
        directory: "/tmp/project".into(),
        file_names: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
    });

    let event = request.topic_event_with_request_id("session_a", "request_1");
    assert_eq!(event.session_id, "session_a");
    assert_eq!(event.topic.name, "core.work_instruction_load");
    assert_eq!(event.topic.attributes["name"], event.topic.name);
    assert_eq!(event.topic.attributes["expects_reply"], true);
    assert_eq!(event.request_id(), Some("request_1"));
    assert!(event.expects_reply());
    assert!(event.is_blocking_request());
    assert_eq!(event.state.name(), "waiting_user_with_timeout");
    assert_eq!(
        event.state.timeout_ms(),
        Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis())
    );
    assert_eq!(event.state_payload()["name"], "waiting_user_with_timeout");
    assert_eq!(
        event.state_payload()["timeout_ms"].as_u64(),
        Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis() as u64)
    );
    assert_eq!(event.payload["kind"], "work_instruction_load");
    assert_eq!(event.payload["request_id"], "request_1");
    assert_eq!(event.payload["safe_default"], "decline");
    assert_eq!(
        event.payload["timeout_ms"].as_u64(),
        Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis() as u64)
    );
    assert_eq!(event.payload["request"]["directory"], "/tmp/project");
    assert_eq!(event.payload["request"]["file_names"][0], "AGENTS.md");
    assert_eq!(
        event.wire_payload(),
        json!({
            "session_id": "session_a",
            "topic": {
                "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                "attributes": {
                    "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                    "kind": "work_instruction_load",
                    "expects_reply": true,
                },
            },
            "state": {
                "name": "waiting_user_with_timeout",
                "timeout_ms": DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis(),
            },
            "payload": {
                "kind": "work_instruction_load",
                "request_id": "request_1",
                "safe_default": "decline",
                "timeout_ms": DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT.as_millis(),
                "request": {
                    "directory": "/tmp/project",
                    "file_names": ["AGENTS.md", "CLAUDE.md"],
                },
            },
        })
    );
    assert_eq!(
        event.as_host_decision_request(),
        Some(CoreHostDecisionRequestTopic {
            session_id: "session_a".to_string(),
            kind: "work_instruction_load",
            state: CoreSessionState::WaitingUserWithTimeout {
                timeout: DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT,
            },
            safe_default: HostDecisionDefault::Decline,
            timeout: Some(DEFAULT_OPTIONAL_HOST_REQUEST_TIMEOUT),
            request,
        })
    );
}

#[test]
fn work_instruction_load_status_can_be_published_as_topic_event() {
    let report = WorkInstructionLoadReport {
        status: WorkInstructionLoadStatus::Loaded,
        directory: "/tmp/project".into(),
        file_names: vec!["AGENTS.md".to_string()],
        context: Some("guide".to_string()),
        error: None,
    };

    let event = work_instruction_load_topic_event("session_a", &report);
    assert_eq!(event.session_id, "session_a");
    assert_eq!(event.topic.name, CORE_TOPIC_WORK_INSTRUCTION_LOAD);
    assert_eq!(event.state, CoreSessionState::Running);
    assert!(!event.expects_reply());
    assert_eq!(
        event.wire_payload(),
        json!({
            "session_id": "session_a",
            "topic": {
                "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                "attributes": {
                    "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                },
            },
            "state": {
                "name": "running",
            },
            "payload": {
                "status": "loaded",
                "directory": "/tmp/project",
                "file_names": ["AGENTS.md"],
                "error": null,
            },
        })
    );
    assert_eq!(
        event.as_work_instruction_load(),
        Some(CoreWorkInstructionLoadTopic {
            status: "loaded".to_string(),
            directory: "/tmp/project".to_string(),
            file_names: vec!["AGENTS.md".to_string()],
            error: None,
        })
    );
}

#[test]
fn host_decision_request_topic_accessor_round_trips_all_request_kinds() {
    let requests = [
        HostDecisionRequest::UserApproval(ApprovalRequest {
            approval_id: "approval_1".to_string(),
            action: "run_bash".to_string(),
            command: "printf ok".to_string(),
            reason: "requires_approval".to_string(),
            risk: "local_command".to_string(),
        }),
        HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20)),
        HostDecisionRequest::OutputExpansion(OutputExpansionRequest::new(10_000)),
        HostDecisionRequest::StaleContextContinue(StaleContextDecisionRequest {
            idle: Duration::from_secs(3 * 60 * 60),
            dynamic_context_tokens: 10_001,
            continue_keeps_dynamic_context: true,
            decline_clears_dynamic_context: true,
        }),
        HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
            directory: "/tmp/project".into(),
            file_names: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
        }),
        HostDecisionRequest::LongRunningCommandContinue(LongRunningCommandContinueRequest::new(
            "run_bash",
            "sleep 120",
            Duration::from_secs(65),
            None,
        )),
    ];

    for request in requests {
        let request_id = format!("request_{}", request.kind());
        let event = request.topic_event_with_request_id("session_a", &request_id);
        let topic = event
            .as_host_decision_request()
            .expect("host decision request topic should decode");
        assert!(event.expects_reply());
        assert!(event.is_blocking_request());
        assert_eq!(event.request_id(), Some(request_id.as_str()));
        assert_eq!(topic.session_id, "session_a");
        assert_eq!(topic.kind, request.kind());
        assert_eq!(
            topic.state.name(),
            if request.timeout().is_some() {
                "waiting_user_with_timeout"
            } else {
                "waiting_user"
            }
        );
        assert_eq!(topic.safe_default, request.safe_default());
        assert_eq!(topic.timeout, request.timeout());
        assert_eq!(topic.request, request);
    }
}

#[test]
fn topic_reply_correlates_to_blocking_request_topic() {
    let request = HostDecisionRequest::WorkInstructionLoad(WorkInstructionLoadRequest {
        directory: "/tmp/project".into(),
        file_names: vec!["AGENTS.md".to_string()],
    });
    let event = request.topic_event_with_request_id("session_a", "request_1");

    let reply = TopicReply::for_decision_request(&event, HostDecision::Accept)
        .expect("blocking request should accept a topic reply");

    assert_eq!(reply.session_id, "session_a");
    assert_eq!(reply.topic_name, CORE_TOPIC_WORK_INSTRUCTION_LOAD);
    assert_eq!(reply.request_id.as_deref(), Some("request_1"));
    assert_eq!(reply.decision, HostDecision::Accept);
    assert_eq!(
        reply.wire_payload(),
        json!({
            "session_id": "session_a",
            "topic_name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
            "request_id": "request_1",
            "decision": "accept",
            "payload": {
                "decision": "accept",
            },
        })
    );

    let progress = notification_topic_event(
        "session_a",
        &CoreNotification::ModelResponse {
            status: "working".to_string(),
            free_talk: String::new(),
            final_answer: String::new(),
            continue_work: true,
        },
    );
    assert!(TopicReply::for_decision_request(&progress, HostDecision::Accept).is_none());
}

#[test]
fn topic_reply_resolution_validates_session_topic_and_request_id() {
    let request = HostDecisionRequest::RoundLimitContinue(RoundLimitDecisionRequest::new(20));
    let event = request.topic_event_with_request_id("session_a", "request_1");
    let reply = TopicReply::for_decision_request(&event, HostDecision::Decline)
        .expect("blocking request should produce reply")
        .with_request_id("request_1");

    assert_eq!(
        resolve_topic_reply(&event, Some("request_1"), &reply),
        Ok(HostDecision::Decline)
    );

    let mut wrong_session = reply.clone();
    wrong_session.session_id = "session_b".to_string();
    assert_eq!(
        resolve_topic_reply(&event, Some("request_1"), &wrong_session),
        Err(TopicReplyError::SessionMismatch)
    );

    let mut wrong_topic = reply.clone();
    wrong_topic.topic_name = CORE_TOPIC_ACTION.to_string();
    assert_eq!(
        resolve_topic_reply(&event, Some("request_1"), &wrong_topic),
        Err(TopicReplyError::TopicMismatch)
    );

    let mut wrong_request_id = reply.clone();
    wrong_request_id.request_id = Some("request_2".to_string());
    assert_eq!(
        resolve_topic_reply(&event, Some("request_1"), &wrong_request_id),
        Err(TopicReplyError::RequestIdMismatch)
    );

    let progress = notification_topic_event(
        "session_a",
        &CoreNotification::ModelResponse {
            status: "working".to_string(),
            free_talk: String::new(),
            final_answer: String::new(),
            continue_work: true,
        },
    );
    assert_eq!(
        resolve_topic_reply(&progress, None, &reply),
        Err(TopicReplyError::NotBlockingRequest)
    );
}

#[test]
fn core_notifications_can_be_published_as_topic_events() {
    let notifications = vec![
        CoreNotification::ModelResponse {
            status: "working".to_string(),
            free_talk: "planning next step".to_string(),
            final_answer: String::new(),
            continue_work: true,
        },
        CoreNotification::Action {
            action: "run_bash".to_string(),
            input: serde_json::json!({"cmd": "pwd"}),
            kind: crate::CoreActionKind::Bash {
                command: "pwd".to_string(),
                mode: "normal".to_string(),
                interval_ms: None,
                timeout_ms: None,
                loop_timeout_ms: None,
                once_timeout_ms: None,
            },
            active: true,
            memory_activity: crate::CoreMemoryActivity::None,
        },
    ];

    let events = notification_topic_events("session_a", &notifications);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].session_id, "session_a");
    assert_eq!(events[0].topic.name, CORE_TOPIC_MODEL_RESPONSE);
    assert_eq!(events[0].state, CoreSessionState::Running);
    assert!(!events[0].expects_reply());
    assert!(!events[0].is_blocking_request());
    assert_eq!(
        events[0].wire_payload(),
        json!({
            "session_id": "session_a",
            "topic": {
                "name": CORE_TOPIC_MODEL_RESPONSE,
                "attributes": {
                    "name": CORE_TOPIC_MODEL_RESPONSE,
                },
            },
            "state": {
                "name": "running",
            },
            "payload": {
                "status": "working",
                "free_talk": "planning next step",
                "final_answer": "",
                "continue_work": true,
                "global": {
                    "working_worker_count": 1,
                },
            },
        })
    );
    assert!(events[0].payload.get("text").is_none());
    assert_eq!(
        events[0].as_model_response(),
        Some(CoreModelResponseTopic {
            status: "working".to_string(),
            free_talk: "planning next step".to_string(),
            final_answer: String::new(),
            continue_work: true,
            global: CoreGlobalWorkerStatus::new(1),
        })
    );

    assert_eq!(events[1].topic.name, CORE_TOPIC_ACTION);
    assert_eq!(events[1].topic.attributes["action"], "run_bash");
    assert!(!events[1].expects_reply());
    assert!(!events[1].is_blocking_request());
    assert_eq!(
        events[1].wire_payload(),
        json!({
            "session_id": "session_a",
            "topic": {
                "name": CORE_TOPIC_ACTION,
                "attributes": {
                    "name": CORE_TOPIC_ACTION,
                    "action": "run_bash",
                    "active": true,
                    "event": "start",
                },
            },
            "state": {
                "name": "running",
            },
            "payload": {
                "action": "run_bash",
                "input": {
                    "cmd": "pwd",
                },
                "kind": {
                    "kind": "bash",
                    "command": "pwd",
                    "mode": "normal",
                    "interval_ms": null,
                    "timeout_ms": null,
                    "loop_timeout_ms": null,
                    "once_timeout_ms": null,
                },
                "active": true,
                "event": "start",
                "status": "running",
                "memory_activity": "none",
            },
        })
    );
    assert_eq!(
        events[1].as_action(),
        Some(CoreActionTopic {
            action: "run_bash".to_string(),
            input: serde_json::json!({"cmd": "pwd"}),
            kind: CoreActionKind::Bash {
                command: "pwd".to_string(),
                mode: "normal".to_string(),
                interval_ms: None,
                timeout_ms: None,
                loop_timeout_ms: None,
                once_timeout_ms: None,
            },
            active: true,
            event: "start".to_string(),
            status: "running".to_string(),
            pid: None,
            memory_activity: CoreMemoryActivity::None,
        })
    );
    assert_eq!(
        topic_event_status_hint(&events),
        Some(CoreTopicStatusHint {
            action: "run_bash".to_string(),
            input: serde_json::json!({"cmd": "pwd"}),
            memory_activity: CoreMemoryActivity::None,
        })
    );
}

#[test]
fn model_repair_topic_round_trips_protocol_issue_and_attempt() {
    let event = model_repair_topic_event("session_a", "invalid_xml", 2, 5);

    assert_eq!(event.session_id, "session_a");
    assert_eq!(event.topic.name, CORE_TOPIC_MODEL_REPAIR);
    assert_eq!(event.topic.attributes["name"], CORE_TOPIC_MODEL_REPAIR);
    assert_eq!(event.state, CoreSessionState::WaitingModel);
    assert!(!event.expects_reply());
    assert!(!event.is_blocking_request());
    assert_eq!(
        event.as_model_repair(),
        Some(CoreModelRepairTopic {
            issue: "invalid_xml".to_string(),
            attempt: 2,
            max_attempts: 5,
        })
    );
    assert_eq!(
        event.wire_payload(),
        json!({
            "session_id": "session_a",
            "topic": {
                "name": CORE_TOPIC_MODEL_REPAIR,
                "attributes": {
                    "name": CORE_TOPIC_MODEL_REPAIR,
                },
            },
            "state": {
                "name": "waiting_model",
            },
            "payload": {
                "issue": "invalid_xml",
                "attempt": 2,
                "max_attempts": 5,
            },
        })
    );
}

#[test]
fn core_init_lifecycle_topic_is_structured_and_ui_neutral() {
    let profile = CoreProfile {
        name: "test".to_string(),
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
    };

    let event = core_initialized_topic_event("session_a", &profile, "markdown", 100_000, 50, 6, 2);

    assert_eq!(event.session_id, "session_a");
    assert_eq!(event.topic.name, CORE_TOPIC_LIFECYCLE);
    assert_eq!(event.topic.attributes["name"], CORE_TOPIC_LIFECYCLE);
    assert_eq!(event.topic.attributes["event"], "initialized");
    assert_eq!(event.state, CoreSessionState::Running);
    assert!(!event.expects_reply());
    assert!(!event.is_blocking_request());
    assert_eq!(
        event.as_lifecycle(),
        Some(CoreLifecycleTopic {
            event: CoreLifecycleEvent::Initialized,
            version: env!("CARGO_PKG_VERSION").to_string(),
            profile,
            response_protocol: "markdown".to_string(),
            max_llm_input_tokens: 100_000,
            max_rounds: 50,
            tool_count: 6,
            skill_count: 2,
            worker: None,
            workspace: None,
            context: None,
        })
    );
    assert_eq!(
        event.wire_payload(),
        json!({
            "session_id": "session_a",
            "topic": {
                "name": CORE_TOPIC_LIFECYCLE,
                "attributes": {
                    "name": CORE_TOPIC_LIFECYCLE,
                    "event": "initialized",
                },
            },
            "state": {
                "name": "running",
            },
            "payload": {
                "event": "initialized",
                "version": env!("CARGO_PKG_VERSION"),
                "profile": {
                    "name": "test",
                    "provider": "aliyun",
                    "model": "qwen-plus",
                },
                "response_protocol": "markdown",
                "max_llm_input_tokens": 100000,
                "max_rounds": 50,
                "capabilities": {
                    "tools": 6,
                    "skills": 2,
                },
                "worker": null,
                "workspace": null,
                "context": null,
            },
        })
    );
    let debug = format!("{event:?}");
    for forbidden in ["启动成功", "ⓘ", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core lifecycle topic leaked shell rendering {forbidden:?}: {debug}"
        );
    }
}

#[test]
fn core_lifecycle_topic_round_trips_worker_identity_workspace_and_context() {
    let profile = CoreProfile {
        name: "test".to_string(),
        provider: "local".to_string(),
        model: "fake".to_string(),
    };
    let identity = CoreSessionWorkerIdentity::new(
        "session_child",
        2,
        Some("日志分析".to_string()),
        Some("session_parent".to_string()),
    );
    let mut workspace = CoreSessionWorkerWorkspace::new(
        "/tmp/timem-data",
        "/tmp/timem-data/audit/api_audit.json",
        "timem_native_shell",
        "user_local_machine",
    );
    workspace.current_dir = Some(PathBuf::from("/tmp/project"));
    workspace
        .env
        .insert("TIMEM_GATEWAY_PROVIDER".to_string(), "local".to_string());
    workspace.env.insert(
        "TIMEM_API_KEY".to_string(),
        "sk-lifecycle-secret".to_string(),
    );
    workspace.workspace_dirs.push(PathBuf::from("/tmp/project"));
    let mut expected_workspace = workspace.clone();
    expected_workspace.env.insert(
        "TIMEM_API_KEY".to_string(),
        crate::redaction::REDACTED.to_string(),
    );
    let context = CoreDynamicContextSummary {
        visible_delta_count: 3,
        visible_slice_count: 5,
        estimated_tokens: 2048,
    };

    let event = core_initialized_topic_event_with_worker(
        "session_child",
        &profile,
        "markdown",
        100_000,
        50,
        6,
        0,
        Some(&identity),
        Some(&workspace),
        Some(context),
    );
    let lifecycle = event.as_lifecycle().expect("lifecycle should parse");

    assert_eq!(lifecycle.worker, Some(identity));
    assert_eq!(lifecycle.workspace, Some(expected_workspace));
    assert_eq!(lifecycle.context, Some(context));
    assert_eq!(event.payload["worker"]["display_name"], "日志分析");
    assert_eq!(event.payload["worker"]["ordinal"], 2);
    assert_eq!(event.payload["context"]["visible_delta_count"], 3);
    assert_eq!(
        event.payload["workspace"]["env"]["TIMEM_API_KEY"],
        crate::redaction::REDACTED
    );
    assert_eq!(
        event.payload["workspace"]["env"]["TIMEM_GATEWAY_PROVIDER"],
        "local"
    );
    assert!(
        !event
            .wire_payload()
            .to_string()
            .contains("sk-lifecycle-secret"),
        "lifecycle topic must not leak env secrets"
    );
}

#[test]
fn topic_callbacks_can_copy_owned_snapshots_for_async_hosts() {
    let notifications = vec![CoreNotification::Action {
        action: "run_bash".to_string(),
        input: serde_json::json!({"cmd": "pwd"}),
        kind: CoreActionKind::Bash {
            command: "pwd".to_string(),
            mode: "normal".to_string(),
            interval_ms: None,
            timeout_ms: None,
            loop_timeout_ms: None,
            once_timeout_ms: None,
        },
        active: true,
        memory_activity: CoreMemoryActivity::None,
    }];

    let mut queued: Vec<CoreTopicEvent> = Vec::new();
    {
        let mut sink = |events: &[CoreTopicEvent]| {
            queued.extend_from_slice(events);
        };
        let events = notification_topic_events("session_a", &notifications);
        sink(&events);
    }
    drop(notifications);

    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].session_id, "session_a");
    assert_eq!(queued[0].as_action().unwrap().input["cmd"], "pwd");
}

#[test]
fn action_topic_kind_wire_payload_is_explicit_and_round_trips() {
    let cases = [
        (
            CoreActionKind::Bash {
                command: "pwd".to_string(),
                mode: "normal".to_string(),
                interval_ms: None,
                timeout_ms: None,
                loop_timeout_ms: None,
                once_timeout_ms: None,
            },
            json!({"kind": "bash", "command": "pwd", "mode": "normal", "interval_ms": null, "timeout_ms": null, "loop_timeout_ms": null, "once_timeout_ms": null}),
        ),
        (
            CoreActionKind::Bash {
                command: "gh run list".to_string(),
                mode: "poll".to_string(),
                interval_ms: Some(5000),
                timeout_ms: None,
                loop_timeout_ms: Some(600000),
                once_timeout_ms: Some(5000),
            },
            json!({"kind": "bash", "command": "gh run list", "mode": "poll", "interval_ms": 5000, "timeout_ms": null, "loop_timeout_ms": 600000, "once_timeout_ms": 5000}),
        ),
        (
            CoreActionKind::ShellJob {
                job_id: "job_1".to_string(),
            },
            json!({"kind": "shell_job", "job_id": "job_1"}),
        ),
        (
            CoreActionKind::Memory {
                surface: "scratch".to_string(),
                operation: "read".to_string(),
            },
            json!({"kind": "memory", "surface": "scratch", "operation": "read"}),
        ),
        (
            CoreActionKind::Capability {
                op: "load".to_string(),
                kind: "skill".to_string(),
                id: "release".to_string(),
            },
            json!({"kind": "capability", "op": "load", "capability_kind": "skill", "id": "release"}),
        ),
        (
            CoreActionKind::SelfTool {
                self_type: "about_me".to_string(),
                op: "read".to_string(),
            },
            json!({"kind": "self_tool", "self_type": "about_me", "op": "read"}),
        ),
        (
            CoreActionKind::ChatHistory {
                operation: "query".to_string(),
            },
            json!({"kind": "chat_history", "operation": "query"}),
        ),
        (
            CoreActionKind::Other {
                action: "future_tool".to_string(),
            },
            json!({"kind": "other", "action": "future_tool"}),
        ),
    ];

    for (kind, payload) in cases {
        assert_eq!(action_kind_topic_payload(&kind), payload);
        assert_eq!(action_kind_from_topic_payload(&payload, "fallback"), kind);
    }

    assert_eq!(
        action_kind_from_topic_payload(&json!({"kind": "unknown"}), "future_tool"),
        CoreActionKind::Other {
            action: "future_tool".to_string()
        }
    );
}

#[test]
fn stopped_turn_summary_is_structured_and_ui_neutral() {
    let usage = UsageStats {
        llm_calls: 1,
        prompt_tokens: 10,
        completion_tokens: 2,
        total_tokens: 12,
        ..UsageStats::zero()
    };
    let stops = [
        TurnStopSummary::cancelled_by_user(),
        TurnStopSummary::model_error("provider_network_error"),
        TurnStopSummary::output_limit_stopped_by_user(10_000, usage.clone()),
        TurnStopSummary::round_limit_stopped_by_user(50, usage.clone(), Some(usage)),
    ];

    assert_eq!(stops[0].stop_reason, TurnStopReason::CancelledByUser);
    assert_eq!(stops[0].repair_issue.as_deref(), Some("cancelled_by_user"));
    assert_eq!(stops[0].stats.llm_calls, 0);
    assert!(stops[0].latest_usage.is_none());
    assert_eq!(
        stops[1].detail,
        TurnStopDetail::ModelError {
            error: "provider_network_error".to_string()
        }
    );
    assert_eq!(
        stops[2].detail,
        TurnStopDetail::OutputLimit {
            current_tokens: 10_000
        }
    );
    assert_eq!(
        stops[3].detail,
        TurnStopDetail::RoundLimit { max_rounds: 50 }
    );
    let stopped = stops[1].clone().into_stopped_turn();
    assert_eq!(stopped.stop_reason, TurnStopReason::ModelError);
    assert_eq!(stopped.repair_issue, None);
    let outcome = TurnOutcome::stopped("host-rendered text", stopped, Duration::from_secs(2));
    assert_eq!(outcome.text, "host-rendered text");
    assert_eq!(outcome.stop_reason, Some(TurnStopReason::ModelError));
    assert_eq!(outcome.elapsed, Duration::from_secs(2));

    let serialized = serde_json::to_value(TurnStopSummary::protocol_repair_failed(
        "invalid_json",
        "status_required",
        true,
        UsageStats::zero(),
        None,
    ))
    .unwrap();
    assert_eq!(serialized["stop_reason"], "protocol_repair_failed");
    assert_eq!(serialized["detail"]["kind"], "protocol_repair_failure");
    assert_eq!(serialized["detail"]["first_issue"], "invalid_json");
    assert_eq!(serialized["detail"]["final_issue"], "status_required");
    assert_eq!(serialized["detail"]["truncated"], true);

    let debug = format!("{stops:?}");
    for forbidden in ["已取消", "模型调用失败", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core stop summary leaked UI text {forbidden:?}: {debug}"
        );
    }
}

#[test]
fn user_supplement_normalization_is_host_independent() {
    assert_eq!(
        normalize_user_supplements(vec![
            "  keep this  ".to_string(),
            "\n\t".to_string(),
            "第二条\n".to_string(),
        ]),
        vec!["keep this".to_string(), "第二条".to_string()]
    );
}

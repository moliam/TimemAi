use crate::response_protocol::ParsedAction;
use crate::{
    ActionExecution, ActionRuntime, AgentCore, CoreStep, LlmResponse, TimerSpec, UsageStats,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn setup(name: &str) -> (AgentCore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "timem_timer_{name}_{}_{}",
        std::process::id(),
        crate::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut core = AgentCore::new(
        "STATIC",
        crate::CoreProfile {
            model: "test".into(),
        },
        &dir,
    );
    let _ = core.begin_turn("question", None);
    (core, dir)
}

fn timer_action(tool: &str, input: serde_json::Value) -> ParsedAction {
    ParsedAction {
        action: tool.into(),
        name: None,
        call_id: format!("call_{tool}"),
        raw_input: input,
    }
}

#[derive(Default)]
struct NoopRuntime;
impl ActionRuntime for NoopRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }
}

fn run(core: &mut AgentCore, tool: &str, input: serde_json::Value) -> String {
    let mut runtime = NoopRuntime;
    match core.execute_action(timer_action(tool, input), &mut runtime) {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("unexpected approval"),
    }
}

#[derive(Default)]
struct RecordingRuntime {
    ops: Arc<Mutex<Vec<String>>>,
}

impl ActionRuntime for RecordingRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }
    fn on_core_topic_events(&mut self, events: &[crate::host::CoreTopicEvent]) {
        for event in events {
            if event.topic.name == crate::host::CORE_TOPIC_TIMER {
                if let Ok(mut ops) = self.ops.lock() {
                    ops.push(
                        event
                            .payload
                            .get("op")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    );
                }
            }
        }
    }
}

fn usage() -> UsageStats {
    UsageStats {
        llm_calls: 1,
        prompt_tokens: 10,
        completion_tokens: 2,
        total_tokens: 12,
        ..UsageStats::zero()
    }
}

#[test]
fn timer_after_defaults_name_and_once_mode() {
    let (mut core, dir) = setup("after_basic");
    let out = run(
        &mut core,
        "timer_after",
        json!({"prompt": "check the build", "delay_ms": 60000}),
    );
    assert!(!out.contains("error"), "{out}");
    let timers = core.timers();
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0].name, "timer0");
    assert_eq!(timers[0].mode, "once");
    assert_eq!(timers[0].after_ms, Some(60000));
    assert_eq!(timers[0].at, None);
    assert_eq!(timers[0].interval_ms, None);
    assert_eq!(timers[0].trigger, None);
    assert_eq!(timers[0].queue_timeout_minutes, 0);
    assert_eq!(timers[0].created_by, "model");
    assert!(!timers[0].timer_id.is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_after_accepts_absolute_at_instead_of_delay() {
    let (mut core, dir) = setup("after_at");
    let out = run(
        &mut core,
        "timer_after",
        json!({"prompt": "morning meeting", "at": "2026-01-01T09:00:00Z"}),
    );
    assert!(!out.contains("error"), "{out}");
    let timers = core.timers();
    assert_eq!(timers[0].mode, "once");
    assert_eq!(timers[0].at.as_deref(), Some("2026-01-01T09:00:00Z"));
    assert_eq!(timers[0].after_ms, None);
    assert_eq!(timers[0].interval_ms, None);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_after_requires_exactly_one_schedule_and_a_prompt() {
    let (mut core, dir) = setup("after_validation");
    assert!(run(&mut core, "timer_after", json!({"prompt": "x"})).contains("delay_ms, at"));
    assert!(run(
        &mut core,
        "timer_after",
        json!({"prompt": "x", "delay_ms": 1000, "at": "2026-01-01T00:00:00Z"})
    )
    .contains("not both"));
    assert!(run(
        &mut core,
        "timer_after",
        json!({"prompt": "x", "delay_ms": 0})
    )
    .contains("greater than zero"));
    let missing_prompt = run(&mut core, "timer_after", json!({"delay_ms": 1000}));
    assert!(
        missing_prompt.contains("prompt") && missing_prompt.contains("missing"),
        "{missing_prompt}"
    );
    // An empty at is treated as absent, matching the forgiving executor path.
    assert!(!run(
        &mut core,
        "timer_after",
        json!({"prompt": "x", "delay_ms": 1000, "at": ""})
    )
    .contains("error"));
    assert_eq!(core.timers().len(), 1);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_every_sets_repeat_and_first_fire_one_interval_out() {
    let (mut core, dir) = setup("every_basic");
    let out = run(
        &mut core,
        "timer_every",
        json!({"prompt": "report status", "name": "progress", "interval_ms": 900000}),
    );
    assert!(!out.contains("error"), "{out}");
    let timers = core.timers();
    assert_eq!(timers[0].name, "progress");
    assert_eq!(timers[0].mode, "repeat");
    assert_eq!(timers[0].after_ms, Some(900000));
    assert_eq!(timers[0].interval_ms, Some(900000));
    assert_eq!(timers[0].at, None);
    assert_eq!(timers[0].trigger, None);

    let missing_interval = run(&mut core, "timer_every", json!({"prompt": "x"}));
    assert!(
        missing_interval.contains("interval_ms") && missing_interval.contains("missing"),
        "{missing_interval}"
    );
    assert!(run(
        &mut core,
        "timer_every",
        json!({"prompt": "x", "interval_ms": 0})
    )
    .contains("greater than zero"));
    let missing_prompt = run(&mut core, "timer_every", json!({"interval_ms": 1000}));
    assert!(
        missing_prompt.contains("prompt") && missing_prompt.contains("missing"),
        "{missing_prompt}"
    );
    assert_eq!(core.timers().len(), 1);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_on_supports_both_events_and_rejects_schedules() {
    let (mut core, dir) = setup("on_basic");
    let out = run(
        &mut core,
        "timer_on",
        json!({"prompt": "continue after compression", "name": "compress-watch", "event": "after_compression"}),
    );
    assert!(!out.contains("error"), "{out}");
    assert!(out.contains("stays armed"), "{out}");
    let timers = core.timers();
    assert_eq!(timers[0].name, "compress-watch");
    assert_eq!(timers[0].mode, "trigger");
    assert_eq!(timers[0].trigger.as_deref(), Some("after_compression"));
    assert_eq!(timers[0].after_ms, None);
    assert_eq!(timers[0].at, None);
    assert_eq!(timers[0].interval_ms, None);
    assert_eq!(timers[0].created_by, "model");
    let line = core.active_timers_confirmation_line();
    assert!(line.contains("[compress-watch|trigger|model]"), "{line}");

    let out = run(
        &mut core,
        "timer_on",
        json!({"prompt": "wake when idle", "event": "session_idle"}),
    );
    assert!(!out.contains("error"), "{out}");
    assert_eq!(core.timers()[1].trigger.as_deref(), Some("session_idle"));

    let missing_event = run(&mut core, "timer_on", json!({"prompt": "x"}));
    assert!(
        missing_event.contains("event") && missing_event.contains("missing"),
        "{missing_event}"
    );
    assert!(run(
        &mut core,
        "timer_on",
        json!({"prompt": "x", "event": "whenever"})
    )
    .contains("does not accept"));
    let missing_prompt = run(&mut core, "timer_on", json!({"event": "session_idle"}));
    assert!(
        missing_prompt.contains("prompt") && missing_prompt.contains("missing"),
        "{missing_prompt}"
    );
    // Unknown fields are rejected by the schema before dispatch; the
    // executor's own event check still rejects a missing/unknown event.
    assert_eq!(core.timers().len(), 2);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_cancel_deletes_by_name_and_reports_missing_names() {
    let (mut core, dir) = setup("cancel_basic");
    run(
        &mut core,
        "timer_after",
        json!({"prompt": "x", "delay_ms": 1000}),
    );
    let missing_name = run(&mut core, "timer_cancel", json!({}));
    assert!(
        missing_name.contains("name") && missing_name.contains("missing"),
        "{missing_name}"
    );
    assert!(run(&mut core, "timer_cancel", json!({"name": "nope"}))
        .contains("no timer named \"nope\" exists"));
    let out = run(&mut core, "timer_cancel", json!({"name": "timer0"}));
    assert!(!out.contains("error"), "{out}");
    assert!(core.timers().is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_cancel_respects_user_timer_agent_permission() {
    let (mut core, dir) = setup("cancel_permission");
    let user_spec = |name: &str, allow: bool| TimerSpec {
        timer_id: format!("timer_user_{name}"),
        name: name.into(),
        prompt: "user job".into(),
        mode: "once".into(),
        after_ms: Some(60_000),
        at: None,
        interval_ms: None,
        trigger: None,
        queue_timeout_minutes: 0,
        created_by: "user".into(),
        allow_agent_edit: allow,
    };
    core.replace_timers(vec![user_spec("locked", false), user_spec("open", true)]);

    let blocked = run(&mut core, "timer_cancel", json!({"name": "locked"}));
    assert!(blocked.contains("disabled agent changes"), "{blocked}");
    assert_eq!(
        core.timers().len(),
        2,
        "a blocked delete must change nothing"
    );

    let deleted = run(&mut core, "timer_cancel", json!({"name": "open"}));
    assert!(!deleted.contains("error"), "{deleted}");
    assert_eq!(core.timers().len(), 1);
    assert_eq!(core.timers()[0].name, "locked");
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_spec_allows_agent_changes_for_legacy_json() {
    // timers.json rows written before the option existed carry no field and
    // must keep the default: the agent may delete or modify them.
    let spec: TimerSpec = serde_json::from_value(json!({
        "timer_id": "timer_legacy",
        "name": "timer0",
        "prompt": "legacy prompt",
        "mode": "once",
        "after_ms": 60000,
        "queue_timeout_minutes": 0,
        "created_by": "user"
    }))
    .unwrap();
    assert!(spec.allow_agent_edit);
}

#[test]
fn timer_tools_share_auto_naming_and_duplicate_rejection() {
    let (mut core, dir) = setup("auto_name");
    run(
        &mut core,
        "timer_after",
        json!({"prompt": "a", "delay_ms": 1000}),
    );
    run(
        &mut core,
        "timer_every",
        json!({"prompt": "b", "interval_ms": 1000}),
    );
    run(
        &mut core,
        "timer_on",
        json!({"prompt": "c", "event": "session_idle"}),
    );
    let names: Vec<&str> = core.timers().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["timer0", "timer1", "timer2"]);
    run(&mut core, "timer_cancel", json!({"name": "timer0"}));
    run(
        &mut core,
        "timer_after",
        json!({"prompt": "d", "delay_ms": 1000}),
    );
    let names: Vec<&str> = core.timers().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["timer1", "timer2", "timer0"]);
    let out = run(
        &mut core,
        "timer_every",
        json!({"prompt": "e", "name": "timer1", "interval_ms": 1000}),
    );
    assert!(out.contains("already exists"), "{out}");
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_tools_enforce_the_session_cap() {
    let (mut core, dir) = setup("cap");
    for index in 0..crate::MAX_TIMERS {
        let out = run(
            &mut core,
            "timer_after",
            json!({"prompt": "p", "name": format!("t{index}"), "delay_ms": 1000}),
        );
        assert!(!out.contains("error"), "{out}");
    }
    let out = run(
        &mut core,
        "timer_every",
        json!({"prompt": "p", "name": "over", "interval_ms": 1000}),
    );
    assert!(out.contains("maximum number of timers"), "{out}");
    assert_eq!(core.timers().len(), crate::MAX_TIMERS);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_tools_publish_created_and_deleted_topic_ops() {
    let (mut core, dir) = setup("topic_ops");
    let ops = Arc::new(Mutex::new(Vec::new()));
    let mut runtime = RecordingRuntime {
        ops: Arc::clone(&ops),
    };
    let _ = core.execute_action(
        timer_action("timer_every", json!({"prompt": "a", "interval_ms": 1000})),
        &mut runtime,
    );
    let _ = core.execute_action(
        timer_action("timer_cancel", json!({"name": "timer0"})),
        &mut runtime,
    );
    assert_eq!(*ops.lock().unwrap(), vec!["created", "deleted"]);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_manifest_examples_execute_through_core_action_dispatch() {
    let registry = crate::capability::CapabilityRegistry::builtin();
    let definitions = registry.native_tool_definitions();
    let (mut core, dir) = setup("manifest_examples");
    let order = ["timer_after", "timer_every", "timer_on", "timer_cancel"];
    for (index, tool) in order.into_iter().enumerate() {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == tool)
            .unwrap();
        let example: serde_json::Value = definition
            .description
            .split("Valid argument examples:\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let out = run(&mut core, tool, example);
        assert!(!out.contains("error"), "{tool}: {out}");
        if tool == "timer_cancel" {
            assert_eq!(core.timers().len(), 2);
        } else {
            assert_eq!(core.timers().len(), index + 1);
        }
    }
    assert!(core
        .timers()
        .iter()
        .all(|timer| timer.name != "progress-watch"));
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_snapshot_roundtrip_restores_timers() {
    let (mut core, dir) = setup("snapshot");
    run(
        &mut core,
        "timer_after",
        json!({"prompt": "keep me", "name": "t1", "delay_ms": 1000}),
    );
    let snapshot = core.export_dynamic_context();
    assert_eq!(snapshot.active_timers.len(), 1);
    assert_eq!(snapshot.active_timers[0].name, "t1");
    assert!(!snapshot.is_empty());

    core.clear_dynamic_context();
    // Timers and the memo survive a plain clear; the runtime restates
    // them once on the next request.
    assert_eq!(core.timers().len(), 1);

    // Import wholesale-replaces: empty snapshot removes them.
    core.import_dynamic_context(crate::DynamicContextSnapshot {
        deltas: vec![],
        native_exchanges: vec![],
        last_observed_prompt_tokens: 0,
        active_memo: None,
        pending_forcible_memo_note: None,
        pending_interrupted_memo_note: None,
        active_timers: vec![],
    });
    assert!(core.timers().is_empty());

    core.import_dynamic_context(snapshot);
    assert_eq!(core.timers().len(), 1);
    assert_eq!(core.timers()[0].prompt, "keep me");
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_confirmation_line_marks_created_by() {
    let (mut core, dir) = setup("confirmation");
    run(
        &mut core,
        "timer_every",
        json!({"prompt": "model job", "name": "m1", "interval_ms": 900000}),
    );
    let line = core.active_timers_confirmation_line();
    assert!(line.contains("[m1|repeat|model] model job"), "{line}");
    // Projected user timer (as the host would after TimerSync).
    let user_spec = TimerSpec {
        timer_id: "timer_user_1".into(),
        name: "u1".into(),
        prompt: "user job".into(),
        mode: "repeat".into(),
        after_ms: Some(5000),
        at: None,
        interval_ms: Some(900000),
        trigger: None,
        queue_timeout_minutes: u64::MAX,
        created_by: "user".into(),
        allow_agent_edit: true,
    };
    let mut timers = core.timers().to_vec();
    timers.push(user_spec);
    core.replace_timers(timers);
    let line = core.active_timers_confirmation_line();
    assert!(line.contains("[u1|repeat|user] user job"), "{line}");
    assert!(line.contains("[m1|repeat|model] model job"), "{line}");
    // Empty set produces no line at all.
    core.replace_timers(vec![]);
    assert!(core.active_timers_confirmation_line().is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_tools_work_from_a_model_response_through_core_dispatch() {
    let (mut core, dir) = setup("model_e2e");
    core.set_response_protocol(crate::ResponseProtocolKind::Json);
    let ops = Arc::new(Mutex::new(Vec::new()));
    let mut runtime = RecordingRuntime {
        ops: Arc::clone(&ops),
    };
    let create = r#"{"free_talk":"schedule progress guard","working_still_action":[{"timer_every":{"name":"l80-progress-guard","prompt":"继续完成 L80/M200 SGLang 分析与发布回读；完成或取消时删除本定时器。","interval_ms":900000}}]}"#;
    let step = core.apply_model_response_with_action_runtime(
        LlmResponse {
            content: create.into(),
            tool_calls: Vec::new(),
            model_name: "test".into(),
            usage: usage(),
            truncated: false,
        },
        &mut runtime,
    );
    assert!(matches!(step, CoreStep::NeedModel { .. }), "{step:?}");
    assert_eq!(core.timers().len(), 1);
    let timer = &core.timers()[0];
    assert_eq!(timer.name, "l80-progress-guard");
    assert_eq!(timer.mode, "repeat");
    assert_eq!(timer.after_ms, Some(900000));
    assert_eq!(timer.interval_ms, Some(900000));
    assert_eq!(timer.trigger, None);
    assert_eq!(*ops.lock().unwrap(), vec!["created"]);

    let cancel = r#"{"free_talk":"cancel progress guard","working_still_action":[{"timer_cancel":{"name":"l80-progress-guard"}}]}"#;
    let step = core.apply_model_response_with_action_runtime(
        LlmResponse {
            content: cancel.into(),
            tool_calls: Vec::new(),
            model_name: "test".into(),
            usage: usage(),
            truncated: false,
        },
        &mut runtime,
    );
    assert!(matches!(step, CoreStep::NeedModel { .. }), "{step:?}");
    assert!(core.timers().is_empty());
    assert_eq!(*ops.lock().unwrap(), vec!["created", "deleted"]);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

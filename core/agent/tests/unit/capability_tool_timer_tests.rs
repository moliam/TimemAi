use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionRuntime, AgentCore, TimerSpec};
use serde_json::json;

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

fn timer_action(input: serde_json::Value) -> ParsedAction {
    ParsedAction {
        action: "timer".into(),
        name: None,
        call_id: "call_timer".into(),
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

fn run(core: &mut AgentCore, input: serde_json::Value) -> String {
    let mut runtime = NoopRuntime;
    match super::execute_action(core, &timer_action(input), &mut runtime) {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("unexpected approval"),
    }
}

#[derive(Default)]
struct RecordingRuntime {
    ops: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
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

#[test]
fn timer_new_once_defaults_name_and_mode() {
    let (mut core, dir) = setup("once_basic");
    let out = run(
        &mut core,
        json!({"op": "new", "prompt": "check the build", "after_ms": 60000}),
    );
    assert!(!out.contains("error"), "{out}");
    let timers = core.timers();
    assert_eq!(timers.len(), 1);
    assert_eq!(timers[0].name, "timer0");
    assert_eq!(timers[0].mode, "once");
    assert_eq!(timers[0].after_ms, Some(60000));
    assert_eq!(timers[0].at, None);
    assert_eq!(timers[0].interval_ms, None);
    assert_eq!(timers[0].created_by, "model");
    assert!(!timers[0].timer_id.is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_new_repeat_with_at_and_interval() {
    let (mut core, dir) = setup("repeat_basic");
    let out = run(
        &mut core,
        json!({"op": "new", "prompt": "report status", "mode": "repeat",
               "at": "2026-01-01T09:00:00Z", "interval_ms": 3600000}),
    );
    assert!(!out.contains("error"), "{out}");
    let timers = core.timers();
    assert_eq!(timers[0].mode, "repeat");
    assert_eq!(timers[0].at.as_deref(), Some("2026-01-01T09:00:00Z"));
    assert_eq!(timers[0].after_ms, None);
    assert_eq!(timers[0].interval_ms, Some(3600000));
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_auto_name_fills_gaps_and_respects_explicit_names() {
    let (mut core, dir) = setup("auto_name");
    run(&mut core, json!({"op": "new", "prompt": "a", "after_ms": 1000}));
    run(&mut core, json!({"op": "new", "prompt": "b", "after_ms": 1000}));
    run(&mut core, json!({"op": "del", "name": "timer0"}));
    run(&mut core, json!({"op": "new", "prompt": "c", "after_ms": 1000}));
    let names: Vec<&str> = core.timers().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["timer1", "timer0"]);
    // Duplicate explicit name is rejected.
    let out = run(
        &mut core,
        json!({"op": "new", "prompt": "d", "name": "timer1", "after_ms": 1000}),
    );
    assert!(out.contains("name_exists"), "{out}");
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_validates_schedule_and_mode() {
    let (mut core, dir) = setup("validation");
    // missing prompt
    assert!(run(&mut core, json!({"op": "new", "after_ms": 1000})).contains("prompt_required"));
    // no schedule
    assert!(run(&mut core, json!({"op": "new", "prompt": "x"})).contains("schedule_invalid"));
    // both after_ms and at
    assert!(run(
        &mut core,
        json!({"op": "new", "prompt": "x", "after_ms": 1000, "at": "2026-01-01T00:00:00Z"})
    )
    .contains("schedule_invalid"));
    // zero delay
    assert!(run(&mut core, json!({"op": "new", "prompt": "x", "after_ms": 0})).contains("after_ms_invalid"));
    // interval on once
    assert!(run(
        &mut core,
        json!({"op": "new", "prompt": "x", "after_ms": 1000, "interval_ms": 5000})
    )
    .contains("interval_requires_repeat"));
    // repeat without interval
    assert!(run(
        &mut core,
        json!({"op": "new", "prompt": "x", "mode": "repeat", "after_ms": 1000})
    )
    .contains("interval_required"));
    // repeat with zero interval
    assert!(run(
        &mut core,
        json!({"op": "new", "prompt": "x", "mode": "repeat", "after_ms": 1000, "interval_ms": 0})
    )
    .contains("interval_invalid"));
    // invalid mode
    assert!(run(
        &mut core,
        json!({"op": "new", "prompt": "x", "mode": "hourly", "after_ms": 1000})
    )
    .contains("mode_invalid"));
    // invalid op
    assert!(run(&mut core, json!({"op": "create", "prompt": "x"})).contains("op_invalid"));
    // nothing was recorded
    assert!(core.timers().is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_del_unknown_name_errors_and_name_required() {
    let (mut core, dir) = setup("del");
    run(&mut core, json!({"op": "new", "prompt": "x", "after_ms": 1000}));
    assert!(run(&mut core, json!({"op": "del"})).contains("name_required"));
    assert!(run(&mut core, json!({"op": "del", "name": "nope"})).contains("not_found"));
    let out = run(&mut core, json!({"op": "del", "name": "timer0"}));
    assert!(!out.contains("error"), "{out}");
    assert!(core.timers().is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_enforces_cap() {
    let (mut core, dir) = setup("cap");
    for i in 0..crate::MAX_TIMERS {
        run(
            &mut core,
            json!({"op": "new", "prompt": "p", "name": format!("t{i}"), "after_ms": 1000}),
        );
    }
    assert_eq!(core.timers().len(), crate::MAX_TIMERS);
    assert!(run(
        &mut core,
        json!({"op": "new", "prompt": "p", "name": "over", "after_ms": 1000})
    )
    .contains("timer_limit"));
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_publishes_created_and_deleted_topic_ops() {
    let (mut core, dir) = setup("topic_ops");
    let ops = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = RecordingRuntime {
        ops: std::sync::Arc::clone(&ops),
    };
    let _ = super::execute_action(
        &mut core,
        &timer_action(json!({"op": "new", "prompt": "a", "after_ms": 1000})),
        &mut runtime,
    );
    let _ = super::execute_action(
        &mut core,
        &timer_action(json!({"op": "del", "name": "timer0"})),
        &mut runtime,
    );
    assert_eq!(*ops.lock().unwrap(), vec!["created", "deleted"]);
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timer_snapshot_roundtrip_restores_timers() {
    let (mut core, dir) = setup("snapshot");
    run(
        &mut core,
        json!({"op": "new", "prompt": "keep me", "name": "t1", "after_ms": 1000}),
    );
    let snapshot = core.export_dynamic_context();
    assert_eq!(snapshot.active_timers.len(), 1);
    assert_eq!(snapshot.active_timers[0].name, "t1");
    assert!(!snapshot.is_empty());

    core.clear_dynamic_context();
    // Timers survive a plain clear, unlike the memo.
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
        json!({"op": "new", "prompt": "model job", "name": "m1", "after_ms": 1000}),
    );
    let line = core.active_timers_confirmation_line();
    assert!(line.contains("[m1|once|model] model job"), "{line}");
    // Projected user timer (as the host would after TimerSync).
    let user_spec = TimerSpec {
        timer_id: "timer_user_1".into(),
        name: "u1".into(),
        prompt: "user job".into(),
        mode: "repeat".into(),
        after_ms: Some(5000),
        at: None,
        interval_ms: Some(900000),
        created_by: "user".into(),
    };
    let mut timers = core.timers().to_vec();
    timers.push(user_spec);
    core.replace_timers(timers);
    let line = core.active_timers_confirmation_line();
    assert!(line.contains("[u1|repeat|user] user job"), "{line}");
    assert!(line.contains("[m1|once|model] model job"), "{line}");
    // Empty set produces no line at all.
    core.replace_timers(vec![]);
    assert!(core.active_timers_confirmation_line().is_empty());
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

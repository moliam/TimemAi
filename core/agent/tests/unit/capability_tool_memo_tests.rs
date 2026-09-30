use crate::response_protocol::ParsedAction;
use crate::{ActionExecution, ActionRuntime, AgentCore};
use serde_json::json;

fn setup(name: &str) -> (AgentCore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "timem_memo_{name}_{}_{}",
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

fn memo_action(op: &str, text: &str) -> ParsedAction {
    ParsedAction {
        action: "memo".into(),
        name: None,
        call_id: "call_memo".into(),
        raw_input: json!({"op": op, "text": text}),
    }
}

#[derive(Default)]
struct NoopRuntime;
impl ActionRuntime for NoopRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }
}

fn run(core: &mut AgentCore, op: &str, text: &str) -> String {
    let mut runtime = NoopRuntime;
    match super::execute_action(core, &memo_action(op, text), &mut runtime) {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("unexpected approval"),
    }
}

#[test]
fn memo_create_overwrites_and_delete_clears() {
    let (mut core, _dir) = setup("lifecycle");
    let first = run(&mut core, "create", "first memo");
    assert!(!first.contains("error"));
    assert_eq!(core.active_memo(), Some("first memo"));
    run(&mut core, "create", "second memo");
    assert_eq!(core.active_memo(), Some("second memo"));
    run(&mut core, "update", "third memo");
    assert_eq!(core.active_memo(), Some("third memo"));
    run(&mut core, "delete", "");
    assert_eq!(core.active_memo(), None);
    run(&mut core, "delete", "");
    assert_eq!(core.active_memo(), None);
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
            if event.topic.name == crate::host::CORE_TOPIC_MEMO {
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
fn memo_publishes_created_then_updated_lifecycle_ops() {
    let (mut core, _dir) = setup("lifecycle_ops");
    let ops = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = RecordingRuntime {
        ops: std::sync::Arc::clone(&ops),
    };
    let _ = super::execute_action(&mut core, &memo_action("create", "first"), &mut runtime);
    let _ = super::execute_action(&mut core, &memo_action("update", "second"), &mut runtime);
    let _ = super::execute_action(&mut core, &memo_action("create", "third"), &mut runtime);
    assert_eq!(*ops.lock().unwrap(), vec!["created", "updated", "updated"]);
}

#[test]
fn memo_validates_op_and_text() {
    let (mut core, _dir) = setup("validation");
    assert!(run(&mut core, "create", "").contains("error"));
    assert!(run(&mut core, "update", "   ").contains("error"));
    assert!(run(&mut core, "bogus", "text").contains("error"));
    assert_eq!(core.active_memo(), None);
}

#[test]
fn memo_recreation_cancels_stale_delete_notice_and_rearms_next_delete() {
    let (mut core, dir) = setup("recreate_notice");
    run(&mut core, "create", "first goal");
    assert!(!core
        .build_next_prompt()
        .contains("You just deleted the memo:"));
    run(&mut core, "delete", "");
    run(&mut core, "create", "second goal");
    assert_eq!(core.active_memo(), Some("second goal"));
    assert!(!core
        .build_next_prompt()
        .contains("You just deleted the memo:"));
    run(&mut core, "delete", "");
    assert!(core
        .build_next_prompt()
        .contains("You just deleted the memo: \"second goal\""));
    assert!(!core
        .build_next_prompt()
        .contains("You just deleted the memo:"));
    run(&mut core, "update", "third goal");
    run(&mut core, "delete", "");
    assert!(core
        .build_next_prompt()
        .contains("You just deleted the memo: \"third goal\""));
    assert!(!core
        .build_next_prompt()
        .contains("You just deleted the memo:"));
    drop(core);
    std::fs::remove_dir_all(dir).unwrap();
}

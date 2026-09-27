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

#[test]
fn memo_validates_op_and_text() {
    let (mut core, _dir) = setup("validation");
    assert!(run(&mut core, "create", "").contains("error"));
    assert!(run(&mut core, "update", "   ").contains("error"));
    assert!(run(&mut core, "bogus", "text").contains("error"));
    assert_eq!(core.active_memo(), None);
}

use super::*;
use serde_json::json;

fn setup(name: &str) -> (AgentCore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "timem_turn_finished_{name}_{}_{}",
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

fn action(summary: &str) -> ParsedAction {
    ParsedAction {
        action: "task_finished".into(),
        name: None,
        call_id: "call_tf".into(),
        raw_input: json!({"summary": summary}),
    }
}

fn text(result: ActionExecution) -> String {
    match result {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("unexpected approval"),
    }
}

#[derive(Default)]
struct NoopRuntime;
impl ActionRuntime for NoopRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }
}

#[test]
fn records_summary_and_succeeds() {
    let (mut core, _dir) = setup("success");
    let mut runtime = NoopRuntime;
    let result = execute_action(&mut core, &action("All done."), &mut runtime);
    assert!(text(result).contains("Turn finished"));
    assert_eq!(
        core.take_turn_finished_summary().as_deref(),
        Some("All done.")
    );
    assert_eq!(core.take_turn_finished_summary(), None);
}

#[test]
fn rejects_missing_summary() {
    let (mut core, _dir) = setup("missing");
    let mut runtime = NoopRuntime;
    let mut a = action("x");
    a.raw_input = json!({});
    let result = execute_action(&mut core, &a, &mut runtime);
    assert!(text(result).contains("summary_required"));
    assert_eq!(core.take_turn_finished_summary(), None);
}

#[test]
fn summary_reset_on_new_turn() {
    let (mut core, _dir) = setup("reset");
    let mut runtime = NoopRuntime;
    let _ = execute_action(&mut core, &action("done"), &mut runtime);
    assert!(core.take_turn_finished_summary().is_some());
    let _ = core.begin_turn("next", None);
    assert_eq!(core.take_turn_finished_summary(), None);
}

#[test]
fn legacy_turn_finished_alias_still_dispatches() {
    let (mut core, _dir) = setup("alias");
    let mut runtime = NoopRuntime;
    let action = ParsedAction {
        action: "turn_finished".into(),
        name: None,
        call_id: "call_alias".into(),
        raw_input: json!({"summary": "别名路径最终答复"}),
    };
    let outcome = crate::tool_registry::execute_builtin_tool(
        &mut core,
        "turn_finished",
        &action,
        &mut runtime,
    )
    .expect("builtin dispatch")
    .expect("binding matched");
    let text = match outcome {
        crate::ActionExecution::Completed(outcome) => outcome.text,
        _ => panic!("unexpected approval"),
    };
    assert!(!text.contains("error"));
    assert_eq!(
        core.take_turn_finished_summary().as_deref(),
        Some("别名路径最终答复")
    );
}

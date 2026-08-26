use super::*;
use crate::{CoreProfile, CoreTopicEvent};
use serde_json::json;

#[derive(Default)]
struct CaptureRuntime {
    events: Vec<CoreTopicEvent>,
}
impl ActionRuntime for CaptureRuntime {
    fn should_cancel(&mut self) -> bool {
        false
    }
    fn on_core_topic_events(&mut self, events: &[CoreTopicEvent]) {
        self.events.extend_from_slice(events);
    }
}

fn setup(name: &str) -> (AgentCore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "timem_sub_answer_{name}_{}_{}",
        std::process::id(),
        crate::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut core = AgentCore::new(
        "STATIC",
        CoreProfile {
            model: "test".into(),
        },
        &dir,
    );
    let _ = core.begin_turn("question", None);
    (core, dir)
}
fn action(task: &str, answer: &str) -> ParsedAction {
    ParsedAction {
        action: "sub_answer".into(),
        name: None,
        call_id: "call_sub".into(),
        raw_input: json!({"task":task,"answer":answer}),
    }
}
fn text(result: ActionExecution) -> String {
    match result {
        ActionExecution::Completed(outcome) => outcome.text,
        ActionExecution::NeedsApproval(_) => panic!("unexpected approval"),
    }
}

#[test]
fn emits_topic_and_exact_success_result() {
    let (mut core, dir) = setup("success");
    let mut runtime = CaptureRuntime::default();
    assert_eq!(
        text(execute_action(
            &mut core,
            &action("Question", "Answer"),
            &mut runtime
        )),
        "Shown to user successfully."
    );
    assert_eq!(runtime.events.len(), 1);
    let event = &runtime.events[0];
    assert_eq!(event.topic.name, crate::CORE_TOPIC_SUB_ANSWER);
    assert_eq!(event.payload["ordinal"], 1);
    assert_eq!(event.payload["task"], "Question");
    assert_eq!(event.payload["answer"], "Answer");
    assert!(event.payload["sub_answer_id"]
        .as_str()
        .unwrap()
        .starts_with("sub_answer_"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn supports_many_answers_and_keeps_monotonic_ordinals() {
    let (mut core, dir) = setup("many");
    let mut runtime = CaptureRuntime::default();
    for index in 0..300u64 {
        assert_eq!(
            text(execute_action(
                &mut core,
                &action("Q", &format!("A{index}")),
                &mut runtime
            )),
            "Shown to user successfully."
        );
    }
    assert_eq!(runtime.events.len(), 300);
    assert_eq!(runtime.events.last().unwrap().payload["ordinal"], 300);
    let _ = core.begin_turn("next", None);
    assert_eq!(
        text(execute_action(&mut core, &action("Q2", "A2"), &mut runtime)),
        "Shown to user successfully."
    );
    assert_eq!(runtime.events.last().unwrap().payload["ordinal"], 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejects_invalid_input_and_non_primary_worker() {
    let (mut core, dir) = setup("invalid");
    let mut runtime = CaptureRuntime::default();
    assert!(
        text(execute_action(&mut core, &action("", "A"), &mut runtime)).contains("task_required")
    );
    assert!(
        text(execute_action(&mut core, &action("Q", ""), &mut runtime)).contains("answer_required")
    );
    core.set_sub_answer_enabled(false);
    assert!(
        text(execute_action(&mut core, &action("Q", "A"), &mut runtime))
            .contains("primary_worker_required")
    );
    assert!(runtime.events.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

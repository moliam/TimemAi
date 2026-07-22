use super::*;
use crate::capability::CapabilityRegistry;
use crate::ActionGroupOrder;

fn caps() -> CapabilityRegistry {
    CapabilityRegistry::builtin()
}

fn documented_markdown_examples(text: &str) -> Vec<String> {
    text.split("## -------- Example")
        .skip(1)
        .filter_map(|section| {
            ["## Status", "## Free_Talk", "## Context Compact"]
                .iter()
                .filter_map(|marker| section.find(marker))
                .min()
                .map(|start| section[start..].trim().to_string())
        })
        .collect()
}

#[test]
fn documented_markdown_response_examples_parse_with_runtime_parser() {
    let examples = documented_markdown_examples(MARKDOWN_RESPONSE_PROTOCOL_SECTION);
    assert!(
        examples.len() >= 4,
        "expected protocol document to contain concrete Markdown response examples"
    );

    for (idx, example) in examples.iter().enumerate() {
        let env = parse_markdown_envelope(example, &caps());
        assert!(
            env.repair_issue.is_none(),
            "documented Markdown example #{idx} did not parse: {:?}\n{}",
            env.repair_issue,
            example
        );
        assert!(
            !env.final_answer.trim().is_empty()
                || !env.next_actions.is_empty()
                || !env.context_compacts.is_empty(),
            "documented Markdown example #{idx} produced no runtime-visible result:\n{}",
            example
        );
    }
}

#[test]
fn plain_prose_becomes_final_answer() {
    let env = parse_markdown_envelope("Hello world", &caps());
    assert_eq!(env.final_answer, "Hello world");
    assert!(!env.continue_work);
    assert!(env.repair_issue.is_none());
}

#[test]
fn empty_markdown_response_requests_repair() {
    let env = parse_markdown_envelope("  \n\t", &caps());
    assert_eq!(env.repair_issue.as_deref(), Some("empty_response"));
    assert!(env.final_answer.is_empty());
    assert!(env.next_actions.is_empty());
}

#[test]
fn external_tool_call_protocol_requests_repair_instead_of_plain_answer() {
    let input = r#"<tool_call>
{"name": "run_bash", "arguments": {"cmd": "gh run list", "timeout_ms": 5000}}
</tool_call>"#;
    let env = parse_markdown_envelope(input, &caps());

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("external_tool_call_protocol")
    );
    assert!(env.final_answer.is_empty());
    assert!(env.next_actions.is_empty());
}

#[test]
fn json_fallback() {
    let input = r#"{"status":"finished","final_answer":"done"}"#;
    let env = parse_markdown_envelope(input, &caps());
    assert_eq!(env.final_answer, "done");
    assert!(!env.continue_work);
}

#[test]
fn fenced_json_response_protocol_still_parses() {
    let input = "```json\n{\"status\":\"finished\",\"final_answer\":\"done\"}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert_eq!(env.final_answer, "done");
    assert!(!env.continue_work);
}

#[test]
fn plain_answer_with_json_code_block_stays_plain_answer() {
    let input = "Here is a config example:\n```json\n{\"foo\":\"bar\"}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert_eq!(env.final_answer, input);
    assert!(!env.continue_work);
    assert!(env.next_actions.is_empty());
}

#[test]
fn final_answer_section_with_json_code_block_stays_final_answer() {
    let input = r#"## Status
finished

## Final_Answer
可以这样写：

```json
{
  "status": 400,
  "body": {
    "error": "example"
  }
}
```"#;
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert!(!env.continue_work);
    assert!(env.final_answer.contains("\"status\": 400"));
    assert!(env.next_actions.is_empty());
}

#[test]
fn final_answer_section_with_protocol_headings_stays_final_answer() {
    let input = r#"## Status
finished

## Final_Answer
Example only:

## Working_Still_Action
```action
{"run_bash":{}}
```

## Free_talk
not a real progress section
"#;
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert!(!env.continue_work);
    assert!(env.next_actions.is_empty());
    assert!(env.final_answer.contains("## Working_Still_Action"));
    assert!(env.final_answer.contains("not a real progress section"));
}

#[test]
fn plain_answer_with_inline_braces_stays_plain_answer() {
    let input = "Rust uses `{}` placeholders and blocks like `fn main() {}`.";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert_eq!(env.final_answer, input);
    assert!(!env.continue_work);
    assert!(env.next_actions.is_empty());
}

#[test]
fn prose_before_protocol_json_still_extracts_protocol_payload() {
    let input = "先说明一下。\n{\"status\":\"finished\",\"final_answer\":\"ok\"}";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert_eq!(env.final_answer, "ok");
    assert!(!env.continue_work);
}

#[test]
fn malformed_fenced_json_with_protocol_markers_requests_repair() {
    let input =
            "```json\n{\"working_still_action\":{\"action\":\"run_bash\",\"args\":{\"cmd\":\"bad dangling \\ path and raw \n newline\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert_eq!(env.repair_issue.as_deref(), Some("invalid_json"));
    assert!(env.final_answer.is_empty());
}

#[test]
fn unclosed_fenced_json_with_protocol_markers_requests_repair() {
    let input = "```json\n{\"working_still_action\":{\"action\":\"run_bash\",\"args\":{\"cmd\":\"bad dangling \\ path";
    let env = parse_markdown_envelope(input, &caps());

    assert_eq!(env.repair_issue.as_deref(), Some("invalid_json"));
    assert!(env.final_answer.is_empty());
}

#[test]
fn unclosed_plain_code_fence_requests_repair() {
    let input = "still ``` not { valid \\ json";
    let env = parse_markdown_envelope(input, &caps());

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("unclosed_markdown_code_fence")
    );
    assert!(env.final_answer.is_empty());
}

#[test]
fn sections_parsed_correctly() {
    let input = "## Status\nfinished\n\n## Final_Answer\nHello there";
    let env = parse_markdown_envelope(input, &caps());
    assert_eq!(env.final_answer, "Hello there");
    assert!(!env.continue_work);
    assert!(env.repair_issue.is_none());
}

#[test]
fn missing_structure_triggers_repair() {
    let input = "something { \"action\": \"run_bash\" }";
    let env = parse_markdown_envelope(input, &caps());
    assert!(env.repair_issue.is_some());
}

#[test]
fn finished_without_answer_is_repair() {
    let input = "## Status\nfinished\n\n## Free_talk\nDone";
    let env = parse_markdown_envelope(input, &caps());
    assert_eq!(
        env.repair_issue.as_deref(),
        Some("final_answer_required_when_status_finished")
    );
}

#[test]
fn parses_context_compact_section() {
    let input = "## Free_talk\n整理上下文\n\n## Context Compact\ndiscard: pd_a\noffload: pd_b\nsummary:\n保留当前任务结论。\n下一步继续验证。\n\n## Working_Still_Action\n```action\n{\"run_bash\":{\"cmd\":\"pwd\"}}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert_eq!(env.context_compacts.len(), 1);
    assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a", "pd_b"]);
    assert_eq!(env.context_compacts[0].discard_delta_ids, vec!["pd_a"]);
    assert_eq!(env.context_compacts[0].offload_delta_ids, vec!["pd_b"]);
    assert!(env.context_compacts[0].slice_ids.is_empty());
    assert!(env.context_compacts[0].summary.contains("保留当前任务结论"));
    assert_eq!(env.next_actions.len(), 1);
}

#[test]
fn actions_section_json_fence_still_parses_action() {
    let input = "## Free_talk\nchecking\n\n## Working_Still_Action\n```json\n{\"run_bash\":{\"cmd\":\"pwd\"}}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert!(env.continue_work);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].action, "run_bash");
    assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
}

#[test]
fn actions_section_accepts_nested_parallel_arrays() {
    let input = r#"## Free_talk
checking

## Working_Still_Action
```action
[
  [
    {"run_bash":{"cmd":"printf a"}},
    {"run_bash":{"cmd":"printf b"}}
  ],
  [
    {"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}}
  ]
]
```"#;
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.action_groups.len(), 2);
    assert_eq!(env.action_groups[0].order, ActionGroupOrder::Parallel);
    assert_eq!(env.action_groups[0].actions.len(), 2);
    assert_eq!(env.action_groups[1].order, ActionGroupOrder::Parallel);
    assert_eq!(env.next_actions.len(), 3);
}

#[test]
fn actions_section_rejects_old_group_object() {
    let input = r#"## Free_talk
checking

## Working_Still_Action
```action
{
  "order": "parallel",
  "actions": [
    {"run_bash":{"cmd":"sleep 15","background":true}},
    {"run_bash":{"cmd":"sleep 15","background":true}},
    {"run_bash":{"cmd":"sleep 15","background":true}}
  ]
}
```"#;
    let env = parse_markdown_envelope(input, &caps());

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("actions.old_group_object_not_supported")
    );
    assert!(env.next_actions.is_empty());
}

#[test]
fn actions_section_accepts_mixed_groups_and_actions_without_intent() {
    let input = r#"## Free_talk
checking

## Working_Still_Action
```action
[
  [
    {"run_bash":{"cmd":"printf a","timeout_ms":5000}},
    {"run_bash":{"cmd":"printf b","timeout_ms":5000}}
  ],
  {"run_bash":{"cmd":"pwd","timeout_ms":5000}}
]
```"#;
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.action_groups.len(), 2);
    assert_eq!(env.action_groups[0].order, ActionGroupOrder::Parallel);
    assert_eq!(env.action_groups[1].order, ActionGroupOrder::Sequential);
    assert_eq!(env.next_actions.len(), 3);
}

#[test]
fn mixed_actions_preserve_model_order() {
    let input = r#"## Free_talk
checking

## Working_Still_Action
```action
[
  {"run_bash":{"cmd":"printf first","timeout_ms":5000}},
  [
    {"run_bash":{"cmd":"printf middle-a","timeout_ms":5000}},
    {"run_bash":{"cmd":"printf middle-b","timeout_ms":5000}}
  ],
  {"run_bash":{"cmd":"printf last","timeout_ms":5000}}
]
```"#;
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.action_groups.len(), 3);
    assert_eq!(env.action_groups[0].order, ActionGroupOrder::Sequential);
    assert_eq!(
        env.action_groups[0].actions[0].input_str("cmd"),
        "printf first"
    );
    assert_eq!(env.action_groups[1].order, ActionGroupOrder::Parallel);
    assert_eq!(
        env.action_groups[1].actions[0].input_str("cmd"),
        "printf middle-a"
    );
    assert_eq!(
        env.action_groups[1].actions[1].input_str("cmd"),
        "printf middle-b"
    );
    assert_eq!(env.action_groups[2].order, ActionGroupOrder::Sequential);
    assert_eq!(
        env.action_groups[2].actions[0].input_str("cmd"),
        "printf last"
    );
    let commands = env
        .next_actions
        .iter()
        .map(|action| action.input_str("cmd"))
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        vec![
            "printf first",
            "printf middle-a",
            "printf middle-b",
            "printf last"
        ]
    );
}

#[test]
fn extracts_markdown_protocol_after_preface() {
    let input = "我先说明一下处理计划。\n\n## Free_talk\nchecking\n\n## Working_Still_Action\n```action\n{\"run_bash\":{\"cmd\":\"pwd\"}}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert_eq!(env.thought, "checking");
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].action, "run_bash");
    assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
}

#[test]
fn action_block_without_sections_is_working_protocol() {
    let input = "```action\n{\"run_bash\":{\"cmd\":\"pwd\"}}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert!(env.continue_work);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].action, "run_bash");
}

#[test]
fn actions_section_accepts_bare_json_array() {
    let input = "## Free_talk\nchecking\n\n## Working_Still_Action\n[{\"run_bash\":{\"cmd\":\"pwd\"}},{\"memmgr\":{\"type\":\"durable\",\"op\":\"sql\",\"sql\":\"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5\",\"params\":[\"%project%\"],\"limit\":5}}]";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert!(env.continue_work);
    assert_eq!(env.next_actions.len(), 2);
    assert_eq!(env.next_actions[0].action, "run_bash");
    assert_eq!(env.next_actions[1].action, "memmgr");
    assert_eq!(env.next_actions[1].input_str("op"), "sql");
}

#[test]
fn non_protocol_markdown_heading_stays_plain_answer() {
    let input = "## Notes\nThis is ordinary markdown, not the response protocol.";
    let env = parse_markdown_envelope(input, &caps());

    assert!(env.repair_issue.is_none());
    assert!(!env.continue_work);
    assert_eq!(env.final_answer, input);
    assert!(env.next_actions.is_empty());
}

#[test]
fn malformed_action_block_is_not_downgraded_to_plain_answer() {
    let input = "some preface\n```action\n{\"run_bash\":\"cmd=pwd\"}\n```";
    let env = parse_markdown_envelope(input, &caps());

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("actions.args_must_be_object")
    );
    assert!(env.final_answer.is_empty());
}

#[test]
fn markdown_repair_instruction_stays_markdown_protocol() {
    let instruction = md_repair_instruction("next_actions_required_when_status_working");

    assert!(instruction.contains("Markdown response protocol"));
    assert!(instruction.contains("## Free_talk"));
    assert!(instruction.contains("## Working_Still_Action"));
    assert!(instruction.contains("## Status"));
    assert!(!instruction.contains("Return exactly one valid JSON object"));
    assert!(!instruction.contains("Do not use markdown fences"));

    let truncated = md_repair_instruction("truncated_model_output");
    assert!(truncated.contains("Markdown response protocol"));
    assert!(truncated.contains("max output token"));
    assert!(!truncated.contains("JSON object"));
}

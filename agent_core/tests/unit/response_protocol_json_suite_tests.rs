use super::*;
use crate::ActionGroupOrder;
use serde_json::json;

fn caps() -> CapabilityRegistry {
    CapabilityRegistry::builtin()
}

fn documented_json_examples(text: &str) -> Vec<String> {
    text.split("## -------- Example")
        .skip(1)
        .filter_map(extract_first_json_object)
        .collect()
}

fn extract_first_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(text[start..start + offset + ch.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn documented_json_response_examples_parse_with_runtime_parser() {
    let examples = documented_json_examples(JSON_RESPONSE_PROTOCOL_SECTION);
    assert!(
        examples.len() >= 4,
        "expected protocol document to contain concrete JSON response examples"
    );

    for (idx, example) in examples.iter().enumerate() {
        let env = parse_envelope(example, &caps());
        assert!(
            env.repair_issue.is_none(),
            "documented JSON example #{idx} did not parse: {:?}\n{}",
            env.repair_issue,
            example
        );
        assert!(
            !env.final_answer.trim().is_empty()
                || !env.next_actions.is_empty()
                || !env.context_compacts.is_empty(),
            "documented JSON example #{idx} produced no runtime-visible result:\n{}",
            example
        );
    }
}

#[test]
fn unwraps_common_fields_envelope_without_repair() {
    let env = parse_envelope(
        r#"{"fields":{"status":"ALL_FINISHED","final_answer":"ok"}}"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none());
    assert!(!env.continue_work);
    assert_eq!(env.final_answer, "ok");
}

#[test]
fn parses_context_compact_field() {
    let env = parse_envelope(
        r#"{"free_talk":"整理上下文","context_compact":{"delta_ids":["pd_a"],"summary":"keep important state"},"working_still_action":{"run_bash":{"cmd":"pwd"}}}"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none());
    assert_eq!(env.context_compacts.len(), 1);
    assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a"]);
    assert_eq!(env.context_compacts[0].discard_delta_ids, vec!["pd_a"]);
    assert!(env.context_compacts[0].offload_delta_ids.is_empty());
    assert!(env.context_compacts[0].slice_ids.is_empty());
    assert_eq!(env.context_compacts[0].summary, "keep important state");
}

#[test]
fn parses_context_compact_discard_and_offload_fields() {
    let env = parse_envelope(
        r#"{"free_talk":"整理上下文","context_compact":{"discard":["pd_a"],"offload":["pd_b"],"summary":"keep important state"},"working_still_action":{"run_bash":{"cmd":"pwd"}}}"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none());
    assert_eq!(env.context_compacts.len(), 1);
    assert_eq!(env.context_compacts[0].discard_delta_ids, vec!["pd_a"]);
    assert_eq!(env.context_compacts[0].offload_delta_ids, vec!["pd_b"]);
    assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a", "pd_b"]);
}

#[test]
fn parses_action_groups_and_flattens_actions_for_notifications() {
    let env = parse_envelope(
        r#"{"free_talk":"checking","working_still_action":[[{"run_bash":{"cmd":"printf a"}},{"run_bash":{"cmd":"printf b"}}],{"run_bash":{"cmd":"printf c"}}]}"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.action_groups.len(), 2);
    assert_eq!(env.action_groups[0].order, ActionGroupOrder::Parallel);
    assert_eq!(env.action_groups[0].actions.len(), 2);
    assert_eq!(env.action_groups[1].order, ActionGroupOrder::Sequential);
    assert_eq!(env.next_actions.len(), 3);
}

#[test]
fn text_fields_with_protocol_language_are_not_parsed_as_actions_or_control() {
    let text_cases = [
        (
            "final_answer contains json action object",
            json!({
                "status": "ALL_FINISHED",
                "final_answer": "Example only: {\"working_still_action\":{\"action\":\"run_bash\",\"args\":{}}}"
            }),
        ),
        (
            "final_answer contains xml action tags",
            json!({
                "status": "ALL_FINISHED",
                "final_answer": "<working_still_action><action_json>{\"action\":\"run_bash\",\"args\":{}}</action_json></working_still_action>"
            }),
        ),
        (
            "final_answer contains markdown action fence",
            json!({
                "status": "ALL_FINISHED",
                "final_answer": "```action\n{\"action\":\"run_bash\",\"args\":{}}\n```"
            }),
        ),
        (
            "free_talk contains malformed action object but real action is valid",
            json!({
                "free_talk": "Bad example only: {\"action\":\"run_bash\",\"args\":{}}",
                "free_talk": "checking",
                "working_still_action": {"run_bash": {"cmd": "pwd", "timeout_ms": 5000}
                }
            }),
        ),
        (
            "progress contains status and final answer words but real action is valid",
            json!({
                "free_talk": "Example only: {\"status\":\"ALL_FINISHED\",\"final_answer\":\"not real\"}",
                "working_still_action": {"run_bash": {"cmd": "pwd", "timeout_ms": 5000}
                }
            }),
        ),
    ];

    for (label, value) in text_cases {
        let env = parse_envelope(&value.to_string(), &caps());
        assert_eq!(env.repair_issue, None, "{label}: {env:?}");
        if value.get("status").and_then(Value::as_str) == Some("ALL_FINISHED") {
            assert!(!env.continue_work, "{label}");
            assert!(env.next_actions.is_empty(), "{label}");
            assert!(env.action_groups.is_empty(), "{label}");
        } else {
            assert!(env.continue_work, "{label}");
            assert_eq!(env.next_actions.len(), 1, "{label}");
            assert_eq!(env.next_actions[0].input_str("cmd"), "pwd", "{label}");
        }
    }
}

#[test]
fn diverse_confusing_json_responses_keep_strict_execution_boundary() {
    let valid_cases = [
        (
            "mixed groups and standalone actions preserve order",
            json!({
                "free_talk": "checking",
                "working_still_action": [
                    [
                        {"run_bash": {"cmd": "printf a", "timeout_ms": 5000}},
                        {"run_bash": {"cmd": "printf b", "timeout_ms": 5000}}
                    ],
                    {"memmgr": {"type": "durable", "op": "schema"}},
                    [
                        {"run_bash": {"cmd": "printf c", "timeout_ms": 5000}}
                    ]
                ]
            }),
            vec!["run_bash", "run_bash", "memmgr", "run_bash"],
        ),
        (
            "context compact plus action",
            json!({
                "free_talk": "compact before continuing",
                "free_talk": "compacting",
                "context_compact": {"delta_ids": ["pd_a", "pd_b"], "summary": "keep active task, progress, todo"},
                "working_still_action": {"run_bash": {"cmd": "pwd"}}
            }),
            vec!["run_bash"],
        ),
        (
            "raw chat sql with punctuation params",
            json!({
                "free_talk": "searching",
                "working_still_action": {"memmgr": {"type": "raw_chat", "op": "sql", "sql": "SELECT content FROM chat_messages WHERE content LIKE ? LIMIT 5", "params": ["%{\"action\":\"run_bash\"}%"]}}
            }),
            vec!["memmgr"],
        ),
        (
            "polling bash action",
            json!({
                "free_talk": "waiting",
                "working_still_action": {"run_bash": {"loop_cmd": "test -f /tmp/timem_marker", "interval_ms": 1000, "loop_timeout_ms": 15000, "once_timeout_ms": 5000}}
            }),
            vec!["run_bash"],
        ),
        (
            "self tool read",
            json!({
                "free_talk": "checking self",
                "working_still_action": {"self_tool": {"type": "about_me", "op": "read"}}
            }),
            vec!["self_tool"],
        ),
    ];
    for (label, value, expected_actions) in valid_cases {
        let env = parse_envelope(&value.to_string(), &caps());
        assert_eq!(env.repair_issue, None, "{label}: {env:?}");
        assert_eq!(
            env.next_actions
                .iter()
                .map(|action| action.action.as_str())
                .collect::<Vec<_>>(),
            expected_actions,
            "{label}"
        );
    }

    let invalid_cases = [
        (
            "object args required",
            json!({"free_talk":"bad","working_still_action":{"run_bash":"cmd=pwd"}}),
            "actions.args_must_be_object",
        ),
        (
            "old group object rejected",
            json!({"free_talk":"bad","working_still_action":[{"order":"parallel"}]}),
            "actions[0].old_group_object_not_supported",
        ),
        (
            "old action args object rejected",
            json!({"free_talk":"bad","working_still_action":{"action":"fetch_web","args":{"url":"https://example.test"}}}),
            "actions.action_missing",
        ),
        (
            "durable sql required",
            json!({"free_talk":"bad","working_still_action":{"memmgr":{"type":"durable","op":"sql"}}}),
            "actions.input.sql_required_when_op=sql,type=durable",
        ),
        (
            "finished cannot include action",
            json!({"status":"ALL_FINISHED","final_answer":"done","working_still_action":{"run_bash":{"cmd":"pwd"}}}),
            "status_finished_must_not_include_next_actions",
        ),
    ];
    for (label, value, expected_issue) in invalid_cases {
        let env = parse_envelope(&value.to_string(), &caps());
        assert_eq!(
            env.repair_issue.as_deref(),
            Some(expected_issue),
            "{label}: {env:?}"
        );
        assert!(
            env.next_actions.is_empty()
                || expected_issue == "status_finished_must_not_include_next_actions",
            "{label}: invalid payload must not be treated as executable success"
        );
    }
}

#[test]
fn malformed_response_variants_return_repair_issues_without_panic() {
    let cases = [
        (
            r#"{"status":"done","final_answer":"ok"}"#,
            "status_must_be_working_or_all_finished",
        ),
        (
            r#"{"status":"ALL_FINISHED"}"#,
            "final_answer_required_when_status_finished",
        ),
        (
            r#"{"status":"ALL_FINISHED","final_answer":"ok","debug":"leak"}"#,
            "unexpected_top_level_field:debug",
        ),
        (
            r#"{"status":"working","free_talk":"checking","working_still_action":"bad"}"#,
            "actions_section_must_be_action_or_array",
        ),
        (
            r#"{"status":"working","free_talk":"checking","working_still_action":[{"action":"run_bash"}]}"#,
            "actions[0].args_must_be_object",
        ),
        (
            r#"{"status":"working","free_talk":"checking","working_still_action":[{"action":"fetch_web_page","args":{"url":"https://example.test"}}]}"#,
            "actions[0].action_missing",
        ),
        (
            r#"{"continue":false,"note":"done"}"#,
            "unexpected_top_level_field:continue",
        ),
        (
            r#"{"response_to_user":"done"}"#,
            "unexpected_top_level_field:response_to_user",
        ),
        (
            r#"{"status":"working","free_talk":"checking","working_still_action":[],"acceptance_check":{"is_satisfied":false}}"#,
            "unexpected_top_level_field:acceptance_check",
        ),
        (
            r#"{"status":"ALL_FINISHED","final_answer":"◉ 准备汇报结果..."}"#,
            "final_answer_must_not_start_with_runtime_progress_marker",
        ),
        (
            r#"{"status":"working","free_talk":"compact","context_compact":{"delta_ids":["pd_a"]}}"#,
            "context_compact[0].summary_required",
        ),
    ];

    for (raw, expected_issue) in cases {
        let env = parse_envelope(raw, &caps());
        assert_eq!(
            env.repair_issue.as_deref(),
            Some(expected_issue),
            "raw response should be repaired with expected issue: {raw}"
        );
    }
}

#[test]
fn final_response_action_gets_specific_repair_instruction() {
    for action_name in ["final_answer", "final_response"] {
        let expected_issue = "unexpected_top_level_field:action";
        let env = parse_envelope(
            &format!(r#"{{"action":"{action_name}","args":{{"response_text":"OK"}}}}"#),
            &caps(),
        );

        assert_eq!(env.repair_issue.as_deref(), Some(expected_issue));
    }
}

#[test]
fn malformed_truncated_json_returns_invalid_json_without_panic() {
    let env = parse_envelope(
        r#"{"status":"working","free_talk":"正在查询","working_still_action":[{"action":"memmgr"#,
        &caps(),
    );

    assert_eq!(env.repair_issue.as_deref(), Some("invalid_json"));
    assert!(env.continue_work);
    assert!(env.next_actions.is_empty());
}

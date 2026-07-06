use serde_json::{json, Value};

use super::{
    ActionGroupOrder, ParsedAction, ParsedActionGroup, ParsedContextCompact, ParsedEnvelope,
    ResponseProtocolSuite,
};
use crate::capability::CapabilityRegistry;

/// JSON envelope v1 response protocol.
pub struct JsonSuiteV1;

const JSON_RESPONSE_PROTOCOL_SECTION: &str =
    include_str!("../../../resources/protocol/json/response_protocol.md");
const JSON_RESPONSE_SCHEMA_SUMMARY: &str =
    include_str!("../../../resources/protocol/json/response_schema_summary.json");

impl ResponseProtocolSuite for JsonSuiteV1 {
    fn name(&self) -> &str {
        "json_v1"
    }
    fn lang_format(&self) -> &str {
        "JSON"
    }
    fn protocol_schema(&self) -> &str {
        ""
    }
    fn protocol_examples(&self) -> &str {
        ""
    }
    fn response_schema_summary(&self) -> &str {
        JSON_RESPONSE_SCHEMA_SUMMARY
    }
    fn protocol_prompt_section(&self) -> String {
        JSON_RESPONSE_PROTOCOL_SECTION.to_string()
    }
    fn parse(&self, raw: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
        parse_envelope(raw, capabilities)
    }
    fn repair_instruction(&self, issue: &str) -> &str {
        protocol_repair_instruction(issue)
    }
    fn repair_reason(&self, issue: &str) -> &str {
        protocol_repair_reason(issue)
    }
    fn focused_repair_text(&self, issue: &str, text: &str) -> String {
        focused_repair_response_text(issue, text)
    }
    fn can_show_plain_text_after_repair_failure(&self, content: &str) -> bool {
        can_show_plain_text_after_repair_failure(content)
    }
}

pub fn can_show_plain_text_after_repair_failure(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }
    if matches!(trimmed.chars().next(), Some('{') | Some('[')) {
        return false;
    }
    if trimmed.contains("```") || trimmed.contains('{') || trimmed.contains('}') {
        return false;
    }
    if extract_balanced_json_object(trimmed).is_some() {
        return false;
    }
    let lowered = trimmed.to_lowercase();
    ![
        "next_actions",
        "report_job_progress",
        "memory_candidates",
        "\"action\"",
        "'action'",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

pub fn parse_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let value: Value = match parse_json_value_from_model_text(content) {
        Ok(value) => value,
        Err(_) => {
            let tc = content.trim();
            let has_brace = tc.contains('{');
            let looks_json =
                tc.starts_with('{') || tc.starts_with('[') || tc.starts_with("```") || has_brace;
            if !looks_json && !tc.is_empty() {
                return ParsedEnvelope {
                    report_job_progress: String::new(),
                    final_answer: tc.to_string(),
                    continue_work: false,
                    thought: String::new(),
                    thought_keep_in_context: false,
                    next_actions: vec![],
                    action_groups: vec![],
                    context_compacts: vec![],
                    memory_candidates: vec![],
                    runtime_note: Some("auto_wrapped_prose_as_final_answer".to_string()),
                    repair_issue: None,
                };
            }
            return ParsedEnvelope {
                report_job_progress: String::new(),
                final_answer: String::new(),
                continue_work: true,
                thought: String::new(),
                thought_keep_in_context: false,
                next_actions: vec![],
                action_groups: vec![],
                context_compacts: vec![],
                memory_candidates: vec![],
                runtime_note: None,
                repair_issue: Some("invalid_json".to_string()),
            };
        }
    };
    // Auto-wrap action-only shapes into {"next_actions": [...]}.
    let value = if value.is_array() {
        let arr = value.as_array().unwrap();
        let all_actions = !arr.is_empty()
            && arr.iter().all(|item| {
                item.as_object()
                    .is_some_and(|obj| obj.contains_key("action"))
            });
        if all_actions {
            json!({"next_actions": value})
        } else {
            value
        }
    } else if value
        .as_object()
        .is_some_and(|obj| obj.contains_key("action") && !obj.contains_key("final_answer"))
    {
        json!({"next_actions": [value]})
    } else {
        value
    };
    let value = unwrap_fields_envelope(value);
    if !value.is_object() {
        return ParsedEnvelope {
            report_job_progress: String::new(),
            final_answer: String::new(),
            continue_work: true,
            thought: String::new(),
            thought_keep_in_context: false,
            next_actions: vec![],
            action_groups: vec![],
            context_compacts: vec![],
            memory_candidates: vec![],
            runtime_note: None,
            repair_issue: Some("root_must_be_json_object".to_string()),
        };
    }
    let mut repair_issue: Option<String> = None;
    if let Some(object) = value.as_object() {
        if let Some(extra_key) = object
            .keys()
            .find(|key| !is_allowed_response_top_level_key(key))
        {
            repair_issue = Some(format!("unexpected_top_level_field:{extra_key}"));
        }
    }
    let report_job_progress = value
        .get("report_job_progress")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let final_answer = value
        .get("final_answer")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = value.get("status").and_then(Value::as_str);
    let continue_work = match status {
        Some("working") => true,
        Some("finished") => false,
        Some(_) => {
            repair_issue =
                repair_issue.or_else(|| Some("status_must_be_working_or_finished".to_string()));
            true
        }
        None => true,
    };
    let (thought, thought_keep_in_context) = {
        let v = value.get("free_talk");
        if let Some(obj) = v.and_then(Value::as_object) {
            let content = obj
                .get("content")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToString::to_string)
                .unwrap_or_default();
            let keep_in_context = !content.is_empty();
            (content, keep_in_context)
        } else {
            let s = v
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(ToString::to_string)
                .unwrap_or_default();
            let keep_in_context = !s.is_empty();
            (s, keep_in_context)
        }
    };
    let runtime_note: Option<String> = None;
    let context_compacts = parse_context_compacts(&value, &mut repair_issue);

    let mut next_actions = Vec::new();
    let mut action_groups = Vec::new();
    let bare_action = value.get("action").and_then(Value::as_str).is_some();
    if let Some(groups_value) = value.get("action_groups") {
        if let Some(groups) = groups_value.as_array() {
            for (group_idx, group) in groups.iter().enumerate() {
                let Some(group_object) = group.as_object() else {
                    repair_issue = Some(format!("action_groups[{group_idx}]_must_be_object"));
                    break;
                };
                let order = group_object
                    .get("order")
                    .and_then(Value::as_str)
                    .map(ActionGroupOrder::from_name)
                    .unwrap_or(ActionGroupOrder::Sequential);
                let Some(actions) = group_object.get("actions").and_then(Value::as_array) else {
                    repair_issue = Some(format!("action_groups[{group_idx}].actions_required"));
                    break;
                };
                let mut parsed_group_actions = Vec::new();
                for (action_idx, action) in actions.iter().enumerate() {
                    let label = format!("action_groups[{group_idx}].actions[{action_idx}]");
                    match parse_action_value(action, &label, capabilities) {
                        Ok(action) => {
                            next_actions.push(action.clone());
                            parsed_group_actions.push(action);
                        }
                        Err(issue) => {
                            repair_issue = Some(issue);
                            break;
                        }
                    }
                }
                if repair_issue.is_some() {
                    break;
                }
                action_groups.push(ParsedActionGroup {
                    order,
                    actions: parsed_group_actions,
                });
            }
        } else if !groups_value.is_null() {
            repair_issue = Some("action_groups_must_be_array".to_string());
        }
    }
    let action_values = if action_groups.is_empty() {
        if let Some(next_actions_value) = value.get("next_actions") {
            if let Some(actions) = next_actions_value.as_array() {
                Some(actions.iter().collect::<Vec<_>>())
            } else if !next_actions_value.is_null() {
                repair_issue = Some("next_actions_must_be_array".to_string());
                None
            } else {
                None
            }
        } else if bare_action {
            Some(vec![&value])
        } else {
            None
        }
    } else {
        None
    };
    if let Some(actions) = action_values {
        let mut group_actions = Vec::new();
        for (idx, action) in actions.iter().enumerate() {
            match parse_action_value(action, &format!("next_actions[{idx}]"), capabilities) {
                Ok(action) => {
                    next_actions.push(action.clone());
                    group_actions.push(action);
                }
                Err(issue) => {
                    repair_issue = Some(issue);
                    break;
                }
            }
        }
        if repair_issue.is_none() && !group_actions.is_empty() {
            action_groups.push(ParsedActionGroup {
                order: ActionGroupOrder::Sequential,
                actions: group_actions,
            });
        }
    }
    let mut memory_candidates = Vec::new();
    if let Some(candidates_value) = value.get("memory_candidates") {
        if let Some(candidates) = candidates_value.as_array() {
            for candidate in candidates {
                if let Some(text) = candidate.as_str().map(str::trim).filter(|x| !x.is_empty()) {
                    memory_candidates.push(text.to_string());
                    continue;
                }
                for key in ["content", "fact", "summary", "memory", "text", "title"] {
                    if let Some(text) = candidate
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                    {
                        memory_candidates.push(text.to_string());
                        break;
                    }
                }
            }
        } else if !candidates_value.is_null() {
            repair_issue =
                repair_issue.or_else(|| Some("memory_candidates_must_be_array".to_string()));
        }
    }
    if repair_issue.is_none() && !continue_work && final_answer.trim().is_empty() {
        repair_issue = Some("final_answer_required_when_status_finished".to_string());
    }
    if repair_issue.is_none()
        && continue_work
        && status != Some("finished")
        && !final_answer.trim().is_empty()
    {
        repair_issue = Some("final_answer_requires_status_finished".to_string());
    }
    if repair_issue.is_none()
        && !continue_work
        && starts_with_runtime_progress_marker(&final_answer)
    {
        repair_issue = Some("final_answer_must_not_start_with_runtime_progress_marker".to_string());
    }
    if repair_issue.is_none() && !continue_work && !next_actions.is_empty() {
        repair_issue = Some("status_finished_must_not_include_next_actions".to_string());
    }
    if repair_issue.is_none()
        && continue_work
        && next_actions.is_empty()
        && context_compacts.is_empty()
    {
        repair_issue = Some("next_actions_required_when_status_working".to_string());
    }
    ParsedEnvelope {
        report_job_progress,
        final_answer,
        continue_work,
        thought,
        thought_keep_in_context,
        next_actions,
        action_groups,
        context_compacts,
        memory_candidates,
        runtime_note,
        repair_issue,
    }
}

fn unwrap_fields_envelope(value: Value) -> Value {
    let Some(object) = value.as_object() else {
        return value;
    };
    if object.len() == 1 {
        if let Some(fields) = object.get("fields").filter(|fields| fields.is_object()) {
            return fields.clone();
        }
    }
    value
}

fn parse_action_value(
    action: &Value,
    label: &str,
    capabilities: &CapabilityRegistry,
) -> Result<ParsedAction, String> {
    let name = action
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(format!("{label}.action_missing"));
    }
    let input = match action.get("args") {
        Some(Value::Object(_)) => action.get("args").cloned().unwrap_or(Value::Null),
        Some(_) => return Err(format!("{label}.args_must_be_object")),
        None => return Err(format!("{label}.args_required")),
    };
    let intent = action
        .get("intent")
        .or_else(|| input.get("intent"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if intent.is_empty() {
        return Err(format!("{label}.intent_required"));
    }
    let normalized_name = name.as_str();
    if !capabilities.contains_tool(normalized_name) {
        return Err(format!("unsupported_action:{normalized_name}"));
    }
    if let Err(issue) = capabilities.validate_action_input(normalized_name, &input) {
        return Err(format!("{label}.{issue}"));
    }
    Ok(ParsedAction {
        action: name,
        intent: intent.to_string(),
        raw_input: input,
    })
}

fn parse_context_compacts(
    value: &Value,
    repair_issue: &mut Option<String>,
) -> Vec<ParsedContextCompact> {
    let Some(raw) = value
        .get("context_compact")
        .or_else(|| value.get("context_compacts"))
    else {
        return Vec::new();
    };
    let items = if let Some(array) = raw.as_array() {
        array.iter().collect::<Vec<_>>()
    } else {
        vec![raw]
    };
    let mut compacts = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            if repair_issue.is_none() {
                *repair_issue = Some(format!("context_compact[{idx}].must_be_object"));
            }
            break;
        };
        let delta_ids = object
            .get("delta_ids")
            .map(super::json_string_list)
            .unwrap_or_default();
        let summary = object
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if delta_ids.is_empty() {
            if repair_issue.is_none() {
                *repair_issue = Some(format!("context_compact[{idx}].ids_required"));
            }
            break;
        }
        if summary.is_empty() {
            if repair_issue.is_none() {
                *repair_issue = Some(format!("context_compact[{idx}].summary_required"));
            }
            break;
        }
        compacts.push(ParsedContextCompact {
            delta_ids,
            slice_ids: Vec::new(),
            summary,
        });
    }
    compacts
}

fn starts_with_runtime_progress_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('◉') || trimmed.starts_with("▰▱")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> CapabilityRegistry {
        CapabilityRegistry::builtin()
    }

    #[test]
    fn unwraps_common_fields_envelope_without_repair() {
        let env = parse_envelope(
            r#"{"fields":{"status":"finished","final_answer":"ok"}}"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none());
        assert!(!env.continue_work);
        assert_eq!(env.final_answer, "ok");
    }

    #[test]
    fn parses_context_compact_field() {
        let env = parse_envelope(
            r#"{"report_job_progress":"整理上下文","context_compact":{"delta_ids":["pd_a"],"summary":"keep important state"},"next_actions":[{"action":"run_bash","intent":"Check files.","args":{"cmd":"pwd"}}]}"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none());
        assert_eq!(env.context_compacts.len(), 1);
        assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a"]);
        assert!(env.context_compacts[0].slice_ids.is_empty());
        assert_eq!(env.context_compacts[0].summary, "keep important state");
    }

    #[test]
    fn parses_action_groups_and_flattens_actions_for_notifications() {
        let env = parse_envelope(
            r#"{"report_job_progress":"checking","action_groups":[{"order":"parallel","actions":[{"action":"run_bash","intent":"Check A.","args":{"cmd":"printf a"}},{"action":"run_bash","intent":"Check B.","args":{"cmd":"printf b"}}]},{"order":"sequential","actions":[{"action":"run_bash","intent":"Check C.","args":{"cmd":"printf c"}}]}]}"#,
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
    fn malformed_response_variants_return_repair_issues_without_panic() {
        let cases = [
            (
                r#"{"status":"done","final_answer":"ok"}"#,
                "status_must_be_working_or_finished",
            ),
            (
                r#"{"status":"finished"}"#,
                "final_answer_required_when_status_finished",
            ),
            (
                r#"{"status":"finished","final_answer":"ok","debug":"leak"}"#,
                "unexpected_top_level_field:debug",
            ),
            (
                r#"{"status":"working","report_job_progress":"checking","next_actions":{"action":"run_bash","intent":"List.","args":{"cmd":"ls"}}}"#,
                "next_actions_must_be_array",
            ),
            (
                r#"{"status":"working","report_job_progress":"checking","next_actions":[{"action":"run_bash","intent":"List."}]}"#,
                "next_actions[0].args_required",
            ),
            (
                r#"{"status":"working","report_job_progress":"checking","next_actions":[{"action":"run_bash","args":{"cmd":"ls"}}]}"#,
                "next_actions[0].intent_required",
            ),
            (
                r#"{"status":"working","report_job_progress":"checking","next_actions":[{"action":"fetch_web_page","intent":"Fetch.","args":{"url":"https://example.test"}}]}"#,
                "unsupported_action:fetch_web_page",
            ),
            (
                r#"{"continue":false,"report_job_progress":"done"}"#,
                "unexpected_top_level_field:continue",
            ),
            (
                r#"{"response_to_user":"done"}"#,
                "unexpected_top_level_field:response_to_user",
            ),
            (
                r#"{"status":"working","report_job_progress":"checking","next_actions":[],"acceptance_check":{"is_satisfied":false}}"#,
                "unexpected_top_level_field:acceptance_check",
            ),
            (
                r#"{"status":"finished","final_answer":"◉ 准备汇报结果..."}"#,
                "final_answer_must_not_start_with_runtime_progress_marker",
            ),
            (
                r#"{"status":"working","report_job_progress":"compact","context_compact":{"delta_ids":["pd_a"]}}"#,
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
            let expected_issue = format!("unsupported_action:{action_name}");
            let env = parse_envelope(
                &format!(
                    r#"{{"action":"{action_name}","intent":"Reply.","args":{{"response_text":"OK"}}}}"#
                ),
                &caps(),
            );

            assert_eq!(env.repair_issue.as_deref(), Some(expected_issue.as_str()));
            assert!(protocol_repair_instruction(&expected_issue).contains("不是工具 action"));
            assert!(protocol_repair_reason(&expected_issue).contains("final answers must use"));
        }
    }

    #[test]
    fn malformed_truncated_json_returns_invalid_json_without_panic() {
        let env = parse_envelope(
            r#"{"status":"working","report_job_progress":"正在查询","next_actions":[{"action":"memmgr","intent":"查询"#,
            &caps(),
        );

        assert_eq!(env.repair_issue.as_deref(), Some("invalid_json"));
        assert!(env.continue_work);
        assert!(env.next_actions.is_empty());
    }
}

fn is_allowed_response_top_level_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "report_job_progress"
            | "final_answer"
            | "next_actions"
            | "action_groups"
            | "free_talk"
            | "memory_candidates"
            | "context_compact"
            | "context_compacts"
            | "action"
            | "args"
            | "intent"
    )
}

pub fn protocol_repair_instruction(issue: &str) -> &'static str {
    if matches!(
        issue,
        "unsupported_action:final_answer" | "unsupported_action:final_response"
    ) {
        return "检查到刚刚的输出格式有点问题：final_answer/final_response 不是工具 action。最终回答请使用 status:\"finished\" 和 final_answer 顶层字段，不要放在 next_actions/action 中。Return exactly one valid JSON object. Do not use markdown fences.";
    }
    match issue {
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你提供了 final_answer，但缺少 status:\"finished\"。如果当前用户请求已经完成，请同时提供 status:\"finished\" 和 final_answer；finished 不会关闭 Timem session。如果仍需要 runtime 继续工作，请去掉 final_answer，并提供 next_actions。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你提供了 status:\"finished\"，但缺少 final_answer。如果当前用户请求已经完成，请同时提供 status:\"finished\" 和 final_answer；finished 不会关闭 Timem session。如果仍需要 runtime 继续工作，请不要使用 status:\"finished\"，并提供 next_actions。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：status:\"finished\" 表示当前用户请求已完成，因此不能同时包含 next_actions。如果还需要 runtime 执行动作，请使用 status:\"working\" 或省略 status，并提供 next_actions；拿到 action result 后再用 status:\"finished\" + final_answer 给最终答案。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：status:\"working\" 表示还需要 runtime 继续执行动作，因此必须提供 next_actions。如果当前用户请求已经完成，请改用 status:\"finished\" 和 final_answer；finished 不会关闭 Timem session。Return exactly one valid JSON object. Do not use markdown fences."
        }
        _ => {
            "Return exactly one valid JSON object. Omitted status defaults to working; include next_actions when working. Use status:\"finished\" together with final_answer when the current user request is complete; this does not close the Timem session. Do not use markdown fences."
        }
    }
}

pub fn protocol_repair_reason(issue: &str) -> &'static str {
    if matches!(
        issue,
        "unsupported_action:final_answer" | "unsupported_action:final_response"
    ) {
        return "The previous model response tried to use final_answer/final_response as a tool action, but final answers must use status:\"finished\" with final_answer.";
    }
    match issue {
        "truncated_model_output" => {
            "The provider stopped the model output before a complete response_v1 JSON object was produced."
        }
        "invalid_json" => "The previous model response could not be parsed as one JSON object.",
        "root_must_be_json_object" => {
            "The previous model response parsed as JSON, but the root value was not an object."
        }
        "final_answer_requires_status_finished" => {
            "The previous model response included final_answer without status:\"finished\"."
        }
        "final_answer_required_when_status_finished" => {
            "The previous model response included status:\"finished\" without final_answer."
        }
        "status_finished_must_not_include_next_actions" => {
            "The previous model response used status:\"finished\" together with next_actions. Finished responses must not request more runtime actions."
        }
        "final_answer_must_not_start_with_runtime_progress_marker" => {
            "The final_answer started with a runtime UI progress marker instead of user-facing content."
        }
        _ => "The previous model response did not match the local response_v1 protocol.",
    }
}

pub fn focused_repair_response_text(issue: &str, text: &str) -> String {
    const REPAIR_CONTEXT_CHARS: usize = 6_000;
    let trimmed = text.trim();
    let char_count = trimmed.chars().count();
    if char_count <= REPAIR_CONTEXT_CHARS * 2 {
        return trimmed.to_string();
    }
    if let Some(focus) = repair_focus_char_index(issue, trimmed) {
        return char_window_around_focus(trimmed, focus, REPAIR_CONTEXT_CHARS);
    }
    let head: String = trimmed.chars().take(REPAIR_CONTEXT_CHARS).collect();
    let tail_start = char_count.saturating_sub(REPAIR_CONTEXT_CHARS);
    let tail: String = trimmed.chars().skip(tail_start).collect();
    format!(
        "{head}\n[TRUNCATED previous response: omitted middle chars {}..{} of {} chars; no precise repair focus found]\n{tail}",
        REPAIR_CONTEXT_CHARS, tail_start, char_count
    )
}

fn repair_focus_char_index(issue: &str, text: &str) -> Option<usize> {
    if matches!(issue, "invalid_json" | "truncated_model_output") {
        let json_start_byte = text.find('{').unwrap_or(0);
        let json_text = &text[json_start_byte..];
        if let Err(err) = serde_json::from_str::<Value>(json_text) {
            if let Some(relative_idx) =
                line_column_to_char_index(json_text, err.line(), err.column())
            {
                return Some(text[..json_start_byte].chars().count() + relative_idx);
            }
        }
    }
    let marker = match issue {
        "final_answer_requires_status_finished"
        | "final_answer_must_not_start_with_runtime_progress_marker" => "final_answer",
        "final_answer_required_when_status_finished" | "status_must_be_working_or_finished" => {
            "status"
        }
        issue if issue.starts_with("next_actions") => "next_actions",
        issue if issue.contains("memmgr") => "memmgr",
        issue if issue.contains("capmgr") => "capmgr",
        _ => "",
    };
    if marker.is_empty() {
        return None;
    }
    text.find(marker)
        .map(|byte_idx| text[..byte_idx].chars().count())
}

fn line_column_to_char_index(text: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut current_line = 1usize;
    let mut current_column = 1usize;
    for (char_idx, ch) in text.chars().enumerate() {
        if current_line == line && current_column >= column.max(1) {
            return Some(char_idx);
        }
        if ch == '\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
    }
    Some(text.chars().count())
}

fn char_window_around_focus(text: &str, focus: usize, context_chars: usize) -> String {
    let char_count = text.chars().count();
    let start = focus.saturating_sub(context_chars);
    let end = focus.saturating_add(context_chars).min(char_count);
    let window: String = text.chars().skip(start).take(end - start).collect();
    format!(
        "[FOCUSED previous response: chars {}..{} of {} chars; focus char {}]\n{}",
        start, end, char_count, focus, window
    )
}

/// Strip markdown code fences (```json ... ``` or ``` ... ```) from model output.
fn strip_markdown_code_fences(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix("```")?;
    let after_tag = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or("");
    let body = after_tag.strip_suffix("```").map(str::trim)?;
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

/// Repair unescaped ASCII double-quotes inside JSON string values.
fn repair_unescaped_quotes_in_values(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    let len = chars.len();
    if len < 2 {
        return None;
    }
    let mut result = String::with_capacity(len + 64);
    let mut i = 0;
    let mut in_string = false;
    let mut string_is_key = false;
    let mut last_structural: char = '\0';
    let mut changed = false;
    while i < len {
        let ch = chars[i];
        if !in_string {
            result.push(ch);
            if ch == '"' {
                in_string = true;
                string_is_key =
                    last_structural == '{' || last_structural == ',' || last_structural == '[';
            }
            if matches!(ch, '{' | '}' | '[' | ']' | ':' | ',') {
                last_structural = ch;
            }
            i += 1;
        } else if ch == '\\' {
            result.push(ch);
            i += 1;
            if i < len {
                result.push(chars[i]);
                i += 1;
            }
        } else if ch == '"' {
            let rest: String = chars[i + 1..].iter().collect();
            let after = rest.trim_start();
            let is_close = after.starts_with(',')
                || after.starts_with('}')
                || after.starts_with(']')
                || after.starts_with(':')
                || after.is_empty();
            if is_close || string_is_key {
                result.push(ch);
                in_string = false;
            } else {
                result.push('\\');
                result.push('"');
                changed = true;
            }
            i += 1;
        } else {
            result.push(ch);
            i += 1;
        }
    }
    if changed {
        Some(result)
    } else {
        None
    }
}
fn parse_json_value_from_model_text(content: &str) -> Result<Value, serde_json::Error> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    if let Some(repaired) = repair_known_string_field_quotes(trimmed) {
        if let Ok(value) = serde_json::from_str(&repaired) {
            return Ok(value);
        }
    }
    // Strip markdown code fences and retry
    if let Some(stripped) = strip_markdown_code_fences(trimmed) {
        if let Ok(value) = serde_json::from_str(stripped) {
            return Ok(value);
        }
        if let Some(repaired) = repair_known_string_field_quotes(stripped) {
            if let Ok(value) = serde_json::from_str(&repaired) {
                return Ok(value);
            }
        }
    }
    let mut last_ok = None;
    for (idx, ch) in trimmed.char_indices() {
        if ch != '{' {
            continue;
        }
        let candidate = &trimmed[idx..];
        if let Ok(value) = serde_json::from_str(candidate) {
            if is_likely_response_envelope(&value) {
                last_ok = Some(value);
            }
        }
        if let Some(repaired) = repair_known_string_field_quotes(candidate) {
            if let Ok(value) = serde_json::from_str(&repaired) {
                if is_likely_response_envelope(&value) {
                    last_ok = Some(value);
                }
            }
        }
        if let Some(object_text) = extract_balanced_json_object(candidate) {
            if let Ok(value) = serde_json::from_str(&object_text) {
                if is_likely_response_envelope(&value) {
                    last_ok = Some(value);
                }
            }
            if let Some(repaired) = repair_known_string_field_quotes(&object_text) {
                if let Ok(value) = serde_json::from_str(&repaired) {
                    if is_likely_response_envelope(&value) {
                        last_ok = Some(value);
                    }
                }
            }
        }
    }
    if let Some(value) = last_ok {
        Ok(value)
    } else {
        if let Some(repaired) = repair_unescaped_quotes_in_values(trimmed) {
            if let Ok(value) = serde_json::from_str(&repaired) {
                return Ok(value);
            }
        }
        serde_json::from_str(trimmed)
    }
}

pub(crate) fn is_likely_response_envelope(value: &Value) -> bool {
    let normalized = unwrap_fields_envelope(value.clone());
    normalized.as_object().is_some_and(|object| {
        object.contains_key("report_job_progress")
            || object.contains_key("next_actions")
            || object.contains_key("final_answer")
            || object.contains_key("status")
            || object.contains_key("free_talk")
    })
}

fn extract_balanced_json_object(input: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(input[..idx + ch.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn repair_known_string_field_quotes(input: &str) -> Option<String> {
    let mut output = input.to_string();
    let mut changed = false;
    for key in [
        "report_job_progress",
        "free_talk",
        "intent",
        "query",
        "content",
        "command",
        "sql",
    ] {
        let (next, key_changed) = repair_unescaped_quotes_for_key(&output, key);
        output = next;
        changed |= key_changed;
    }
    changed.then_some(output)
}

fn repair_unescaped_quotes_for_key(input: &str, key: &str) -> (String, bool) {
    let marker = format!("\"{key}\"");
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut pos = 0;
    let mut changed = false;
    while let Some(rel) = input[pos..].find(&marker) {
        let marker_start = pos + rel;
        output.push_str(&input[pos..marker_start]);
        output.push_str(&marker);
        let mut cursor = marker_start + marker.len();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            output.push(bytes[cursor] as char);
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b':' {
            pos = cursor;
            continue;
        }
        output.push(':');
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            output.push(bytes[cursor] as char);
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'"' {
            pos = cursor;
            continue;
        }
        output.push('"');
        cursor += 1;
        let value_start = cursor;
        let mut segment = String::new();
        let mut ended = false;
        while cursor < input.len() {
            let Some(ch) = input[cursor..].chars().next() else {
                break;
            };
            let ch_len = ch.len_utf8();
            if ch == '\\' {
                segment.push(ch);
                cursor += ch_len;
                if cursor < input.len() {
                    if let Some(next_ch) = input[cursor..].chars().next() {
                        segment.push(next_ch);
                        cursor += next_ch.len_utf8();
                    }
                }
                continue;
            }
            if ch == '"' {
                let next = next_non_ws_char(input, cursor + ch_len);
                if matches!(next, Some(',') | Some('}') | Some(']') | None) {
                    output.push_str(&segment);
                    output.push('"');
                    cursor += ch_len;
                    ended = true;
                    break;
                }
                output.push_str(&segment);
                output.push('\\');
                output.push('"');
                segment.clear();
                cursor += ch_len;
                changed = true;
                continue;
            }
            segment.push(ch);
            cursor += ch_len;
        }
        if !ended {
            output.push_str(&input[value_start..cursor]);
        }
        pos = cursor;
    }
    output.push_str(&input[pos..]);
    (output, changed)
}

fn next_non_ws_char(input: &str, mut pos: usize) -> Option<char> {
    while pos < input.len() {
        let ch = input[pos..].chars().next()?;
        if !ch.is_whitespace() {
            return Some(ch);
        }
        pos += ch.len_utf8();
    }
    None
}

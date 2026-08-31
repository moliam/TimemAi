use serde_json::Value;

use super::{
    ParsedContextCompact, ParsedEnvelope, PromptBoundarySpec, ResponseProtocolSuite,
    BRACKETED_PROMPT_BOUNDARIES,
};
use crate::capability::CapabilityRegistry;

/// JSON envelope v1 response protocol.
pub struct JsonSuiteV1;

const JSON_RESPONSE_PROTOCOL_SECTION: &str =
    include_str!("../../../../resources/protocol/json/response_protocol.md");
const JSON_RESPONSE_SCHEMA_SUMMARY: &str =
    include_str!("../../../../resources/protocol/json/response_schema_summary.json");

impl ResponseProtocolSuite for JsonSuiteV1 {
    fn name(&self) -> &str {
        "json_v1"
    }
    fn prompt_boundaries(&self) -> &'static PromptBoundarySpec {
        &BRACKETED_PROMPT_BOUNDARIES
    }
    fn lang_format(&self) -> &str {
        "JSON"
    }
    fn action_result_heading(&self) -> Option<&str> {
        Some("The following are results of the actions generated in response:")
    }
    fn response_shape_hint(&self) -> &str {
        "one JSON object {...}"
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
    let _ = content;
    false
}

pub fn parse_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let value: Value = match parse_json_value_from_model_text(content) {
        Ok(value) => value,
        Err(_) => {
            return ParsedEnvelope {
                final_answer: String::new(),
                toolgen_retrospect: String::new(),
                continue_work: true,
                thought: String::new(),
                thought_keep_in_context: false,
                next_actions: vec![],
                action_groups: vec![],
                context_compacts: vec![],
                accepted_response: None,
                memory_candidates: vec![],
                runtime_note: None,
                recovered_issue: None,
                repair_issue: Some("invalid_json".to_string()),
            };
        }
    };
    if !value.is_object() {
        return ParsedEnvelope {
            final_answer: String::new(),
            toolgen_retrospect: String::new(),
            continue_work: true,
            thought: String::new(),
            thought_keep_in_context: false,
            next_actions: vec![],
            action_groups: vec![],
            context_compacts: vec![],
            accepted_response: None,
            memory_candidates: vec![],
            runtime_note: None,
            recovered_issue: None,
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
    let final_answer = value
        .get("final_answer")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let toolgen_retrospect = value
        .get("toolgen_retrospect")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = value.get("status").and_then(Value::as_str);
    let status_normalized = status.map(|raw| raw.trim().to_ascii_lowercase());
    let continue_work = match status_normalized.as_deref() {
        Some("working") => true,
        Some("all_finished") => false,
        Some(_) => {
            repair_issue =
                repair_issue.or_else(|| Some("status_must_be_working_or_all_finished".to_string()));
            true
        }
        None => true,
    };
    let thought = value
        .get("free_talk")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .unwrap_or_default();
    if value
        .get("free_talk")
        .is_some_and(|value| !value.is_string())
    {
        repair_issue = repair_issue.or_else(|| Some("free_talk_must_be_string".to_string()));
    }
    let thought_keep_in_context = !thought.is_empty();
    let runtime_note: Option<String> = None;
    let context_compacts = parse_context_compacts(&value, &mut repair_issue);

    let mut next_actions = Vec::new();
    let mut action_groups = Vec::new();
    let action_value = value.get("working_still_action");
    if let Some(action_value) = action_value {
        match super::parse_action_workflow_value(action_value, "actions", capabilities) {
            Ok(groups) => {
                next_actions.extend(groups.iter().flat_map(|group| group.actions.clone()));
                action_groups = groups;
            }
            Err(issue) => {
                repair_issue = repair_issue.or(Some(issue));
            }
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
        && !toolgen_retrospect.trim().is_empty()
        && (continue_work || final_answer.trim().is_empty())
    {
        repair_issue = Some("toolgen_retrospect_requires_final_answer".to_string());
    }
    if repair_issue.is_none()
        && continue_work
        && !matches!(
            status_normalized.as_deref(),
            Some("finished") | Some("all_finished")
        )
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
        final_answer,
        toolgen_retrospect,
        continue_work,
        thought,
        thought_keep_in_context,
        next_actions,
        action_groups,
        context_compacts,
        memory_candidates,
        accepted_response: None,
        runtime_note,
        recovered_issue: None,
        repair_issue,
    }
}

fn parse_context_compacts(
    value: &Value,
    repair_issue: &mut Option<String>,
) -> Vec<ParsedContextCompact> {
    let Some(raw) = value.get("context_compact") else {
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
        let discard_delta_ids = object
            .get("discard")
            .map(super::json_string_list)
            .unwrap_or_default();
        let offload_delta_ids = object
            .get("offload")
            .map(super::json_string_list)
            .unwrap_or_default();
        let mut delta_ids = discard_delta_ids.clone();
        delta_ids.extend(offload_delta_ids.iter().cloned());
        delta_ids.sort();
        delta_ids.dedup();
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
            discard_delta_ids,
            offload_delta_ids,
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
#[path = "../../tests/unit/response_protocol_json_suite_tests.rs"]
mod tests;

fn is_allowed_response_top_level_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "final_answer"
            | "toolgen_retrospect"
            | "working_still_action"
            | "free_talk"
            | "memory_candidates"
            | "context_compact"
    )
}

pub fn protocol_repair_instruction(issue: &str) -> &'static str {
    if matches!(
        issue,
        "unsupported_action:final_answer" | "unsupported_action:final_response"
    ) {
        return "检查到刚刚的输出格式有点问题：final_answer/final_response 不是工具 action。最终回答请使用 status:\"ALL_FINISHED\" 和 final_answer 顶层字段，不要放在 working_still_action/action 中。Return exactly one valid JSON object. Do not use markdown fences.";
    }
    match issue {
        "truncated_model_output" => {
            "检查到刚刚的输出被 max output token 截断，未形成完整 JSON。上一次已收到的截断回复和原生工具参数片段已附在上下文中，请不要再次生成同样长的整段内容。把工作拆成小块：本次只生成一个较小、完整的步骤或工具调用，拿到结果后再继续下一小块；如果是长报告，可分段写入文件，最后只在 final_answer 中给出简短总结和路径。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "context_compact_must_be_first" => {
            "检查到 context_compact 不是本次响应的第一个工具调用。请把 context_compact 放在所有其他 capability calls 之前；后续 calls 可以保留，它们只会在压缩成功后执行。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "context_compact_only_once" => {
            "检查到本次响应包含多个 context_compact。每次响应最多调用一次 context_compact，并将它放在第一个位置；后续可以继续提供其他 capability calls。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你提供了 final_answer，但缺少 status:\"ALL_FINISHED\"。如果所有用户的 open/pending 请求已经完成，请同时提供 status:\"ALL_FINISHED\" 和 final_answer；这不会关闭 Timem session。如果仍需要 runtime 继续工作，请去掉 final_answer，并提供 working_still_action。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你提供了 status:\"ALL_FINISHED\"，但缺少 final_answer。如果所有用户的 open/pending 请求已经完成，请同时提供 status:\"ALL_FINISHED\" 和 final_answer；这不会关闭 Timem session。如果仍需要 runtime 继续工作，请不要使用 status:\"ALL_FINISHED\"，并提供 working_still_action。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：status:\"ALL_FINISHED\" 表示所有用户的 open/pending 请求已完成，因此不能同时包含 working_still_action。如果还需要 runtime 执行动作，请使用 status:\"working\" 或省略 status，并提供 working_still_action；拿到 action result 后再用 status:\"ALL_FINISHED\" + final_answer 给最终答案。Return exactly one valid JSON object. Do not use markdown fences."
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：status:\"working\" 表示还需要 runtime 继续执行动作，因此必须提供 working_still_action。如果所有用户的 open/pending 请求已经完成，请改用 status:\"ALL_FINISHED\" 和 final_answer。Return exactly one valid JSON object. Do not use markdown fences."
        }
        _ => {
            "Return exactly one valid JSON object. Omitted status defaults to working; include working_still_action when working. Use status:\"ALL_FINISHED\" together with final_answer when all user's open and pending requests are complete. Do not use markdown fences."
        }
    }
}

pub fn protocol_repair_reason(issue: &str) -> &'static str {
    if matches!(
        issue,
        "unsupported_action:final_answer" | "unsupported_action:final_response"
    ) {
        return "The previous model response tried to use final_answer/final_response as a tool action, but final answers must use status:\"ALL_FINISHED\" with final_answer.";
    }
    match issue {
        "truncated_model_output" => {
            "The model output stopped before a complete response_v1 JSON object was produced."
        }
        "context_compact_must_be_first" => {
            "The response included context_compact after another capability call, but context compaction must be the first action."
        }
        "context_compact_only_once" => {
            "The response included more than one context_compact call."
        }
        "invalid_json" => "The previous model response could not be parsed as one JSON object.",
        "root_must_be_json_object" => {
            "The previous model response parsed as JSON, but the root value was not an object."
        }
        "final_answer_requires_status_finished" => {
            "The previous model response included final_answer without status:\"ALL_FINISHED\"."
        }
        "final_answer_required_when_status_finished" => {
            "The previous model response included status:\"ALL_FINISHED\" without final_answer."
        }
        "status_finished_must_not_include_next_actions" => {
            "The previous model response used status:\"ALL_FINISHED\" together with working_still_action. Finished responses must not request more runtime actions."
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
        "final_answer_required_when_status_finished" | "status_must_be_working_or_all_finished" => {
            "status"
        }
        issue if issue.starts_with("next_actions") || issue.starts_with("actions") => {
            "working_still_action"
        }
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

fn parse_json_value_from_model_text(content: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(content.trim())
}

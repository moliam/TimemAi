use serde_json::Value;

use super::{ParsedActionGroup, ParsedContextCompact, ParsedEnvelope, ResponseProtocolSuite};
use crate::capability::CapabilityRegistry;

pub struct MarkdownSuiteV1;

const MARKDOWN_RESPONSE_PROTOCOL_SECTION: &str =
    include_str!("../../../resources/protocol/markdown/response_protocol.md");
const MARKDOWN_RESPONSE_SCHEMA_SUMMARY: &str =
    include_str!("../../../resources/protocol/markdown/response_schema_summary.md");

impl ResponseProtocolSuite for MarkdownSuiteV1 {
    fn name(&self) -> &str {
        "markdown_v1"
    }
    fn lang_format(&self) -> &str {
        "Markdown"
    }
    fn protocol_schema(&self) -> &str {
        ""
    }
    fn protocol_examples(&self) -> &str {
        ""
    }
    fn response_schema_summary(&self) -> &str {
        MARKDOWN_RESPONSE_SCHEMA_SUMMARY
    }
    fn protocol_prompt_section(&self) -> String {
        MARKDOWN_RESPONSE_PROTOCOL_SECTION.to_string()
    }
    fn parse(&self, raw: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
        parse_markdown_envelope(raw, capabilities)
    }
    fn repair_instruction(&self, issue: &str) -> &'static str {
        md_repair_instruction(issue)
    }
    fn repair_reason(&self, issue: &str) -> &'static str {
        md_repair_reason(issue)
    }
    fn focused_repair_text(&self, issue: &str, text: &str) -> String {
        md_focused_repair_text(issue, text)
    }
    fn can_show_plain_text_after_repair_failure(&self, content: &str) -> bool {
        md_can_show_plain_text_after_repair_failure(content)
    }
}

#[derive(Debug)]
struct MdSection {
    heading: String,
    body: String,
}

fn split_sections(text: &str) -> Vec<MdSection> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_body = String::new();
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            current_body.push_str(line);
            current_body.push('\n');
            continue;
        }

        if !in_code_block
            && trimmed.starts_with("## ")
            && !is_terminal_text_heading(&current_heading)
        {
            if !current_heading.is_empty() || !current_body.trim().is_empty() {
                sections.push(MdSection {
                    heading: current_heading.clone(),
                    body: current_body.trim().to_string(),
                });
            }
            current_heading = trimmed[3..].trim().to_lowercase();
            current_body = String::new();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    if !current_heading.is_empty() || !current_body.trim().is_empty() {
        sections.push(MdSection {
            heading: current_heading,
            body: current_body.trim().to_string(),
        });
    }

    sections
}

fn is_terminal_text_heading(heading: &str) -> bool {
    matches!(
        heading.trim().to_ascii_lowercase().as_str(),
        "answer" | "final_answer" | "final answer"
    )
}

fn extract_action_blocks(body: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_action_block = false;
    let mut current_block = String::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if !in_action_block {
            if trimmed.starts_with("```action") || trimmed.starts_with("```json") {
                in_action_block = true;
                current_block = String::new();
            }
        } else if trimmed == "```" {
            in_action_block = false;
            let block = current_block.trim().to_string();
            if !block.is_empty() {
                blocks.push(block);
            }
        } else {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }

    if in_action_block {
        let block = current_block.trim().to_string();
        if !block.is_empty() {
            blocks.push(block);
        }
    }

    blocks
}

fn is_protocol_heading(heading: &str) -> bool {
    matches!(
        heading.trim().to_ascii_lowercase().as_str(),
        "status"
            | "answer"
            | "final_answer"
            | "final answer"
            | "free_talk"
            | "free talk"
            | "freetalk"
            | "toolgen_retrospect"
            | "toolgen retrospect"
            | "working_still_action"
            | "context compact"
            | "context_compact"
            | "compact"
    )
}

fn extract_markdown_protocol_candidate(text: &str) -> Option<&str> {
    let mut offset = 0usize;
    let mut in_code_block = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
        }
        if !in_code_block && trimmed.starts_with("## ") {
            let heading = trimmed[3..].trim();
            if is_protocol_heading(heading) {
                return Some(text[offset..].trim());
            }
        }
        offset += line.len();
    }
    None
}

fn parse_action_groups_value(
    value: &Value,
    capabilities: &CapabilityRegistry,
) -> Result<Vec<ParsedActionGroup>, String> {
    super::parse_action_workflow_value(value, "actions", capabilities)
}

fn extract_fenced_json(text: &str) -> Option<String> {
    let start_marker = "```json";
    let start = text.find(start_marker)?;
    let after_marker = start + start_marker.len();
    let rest = &text[after_marker..];
    let newline = rest.find("\n").map(|i| i + 1).unwrap_or(0);
    let json_start = after_marker + newline;
    let end_marker = "```";
    let end = text[json_start..].find(end_marker)?;
    let json_content = text[json_start..json_start + end].trim().to_string();
    if json_content.is_empty() {
        None
    } else {
        Some(json_content)
    }
}

fn fenced_json_looks_like_response_protocol(text: &str) -> bool {
    let Some(fenced) = extract_fenced_json(text) else {
        return false;
    };
    if fenced_json_contains_protocol_markers(&fenced) {
        return true;
    }
    let Ok(value) = serde_json::from_str::<Value>(&fenced) else {
        return false;
    };
    super::json_suite::is_likely_response_envelope(&value)
        || super::is_tool_action_object(&value)
        || value.as_array().is_some_and(|items| {
            !items.is_empty() && items.iter().all(super::is_tool_action_object)
        })
}

fn fenced_json_contains_protocol_markers(fenced: &str) -> bool {
    contains_protocol_json_markers(fenced)
}

fn contains_protocol_json_markers(text: &str) -> bool {
    [
        "\"status\"",
        "\"final_answer\"",
        "\"next_actions\"",
        "\"context_compact\"",
        "\"context_compacts\"",
        "\"memory_candidates\"",
        "\"action\"",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn has_unclosed_code_fence(text: &str) -> bool {
    text.matches("```").count() % 2 == 1
}

fn malformed_markdown_response(issue: &str) -> ParsedEnvelope {
    ParsedEnvelope {
        final_answer: String::new(),
        toolgen_retrospect: String::new(),
        continue_work: true,
        thought: String::new(),
        thought_keep_in_context: false,
        next_actions: vec![],
        action_groups: vec![],
        context_compacts: vec![],
        memory_candidates: vec![],
        runtime_note: None,
        repair_issue: Some(issue.to_string()),
    }
}

pub fn parse_markdown_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        return malformed_markdown_response("empty_response");
    }

    // JSON fallback
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return super::json_suite::parse_envelope(content, capabilities);
    }

    if looks_like_external_tool_call_protocol(trimmed) {
        return malformed_markdown_response("external_tool_call_protocol");
    }

    // Fenced-JSON extraction for legacy full-response JSON. Do not treat JSON
    // examples inside a Markdown-section response as the response envelope.
    if let Some(fenced) = extract_fenced_json(trimmed) {
        if (fenced.starts_with('{') || fenced.starts_with('['))
            && extract_markdown_protocol_candidate(trimmed).is_none()
            && fenced_json_looks_like_response_protocol(trimmed)
        {
            return super::json_suite::parse_envelope(&fenced, capabilities);
        }
    }

    let protocol_candidate = extract_markdown_protocol_candidate(trimmed);
    let candidate = protocol_candidate.unwrap_or(trimmed);
    let has_sections = candidate.contains("\n## ") || candidate.starts_with("## ");
    let has_action_blocks = candidate.contains("```action")
        || (has_sections
            && candidate.contains("```json")
            && fenced_json_looks_like_response_protocol(candidate));
    if protocol_candidate.is_none() && !has_action_blocks {
        if candidate.contains("```") {
            if candidate.contains("```json") && fenced_json_contains_protocol_markers(candidate) {
                return super::json_suite::parse_envelope(content, capabilities);
            }
            if fenced_json_looks_like_response_protocol(candidate) {
                return super::json_suite::parse_envelope(content, capabilities);
            }
            if has_unclosed_code_fence(candidate) {
                return malformed_markdown_response("unclosed_markdown_code_fence");
            }
            return ParsedEnvelope {
                final_answer: candidate.to_string(),
                toolgen_retrospect: String::new(),
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
        if candidate.contains('{') && contains_protocol_json_markers(candidate) {
            return super::json_suite::parse_envelope(content, capabilities);
        }
        return ParsedEnvelope {
            final_answer: candidate.to_string(),
            toolgen_retrospect: String::new(),
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

    if !has_sections && !has_action_blocks {
        if candidate.contains("```")
            || (candidate.contains('{') && contains_protocol_json_markers(candidate))
        {
            return super::json_suite::parse_envelope(content, capabilities);
        }
        return ParsedEnvelope {
            final_answer: candidate.to_string(),
            toolgen_retrospect: String::new(),
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

    let sections = split_sections(candidate);

    let mut status_raw = String::new();
    let mut final_answer = String::new();
    let mut toolgen_retrospect = String::new();
    let mut thought = String::new();
    let mut thought_keep_in_context = false;
    let mut actions_body = String::new();
    let mut context_compact_body = String::new();
    let mut repair_issue: Option<String> = None;

    for section in &sections {
        match section.heading.as_str() {
            "status" => status_raw = section.body.trim().to_lowercase(),
            "answer" | "final_answer" | "final answer" => {
                final_answer = section.body.clone();
            }
            "toolgen_retrospect" | "toolgen retrospect" => {
                toolgen_retrospect = section.body.trim().to_string();
            }
            "free_talk" | "free talk" | "freetalk" => {
                thought = section.body.trim().to_string();
                thought_keep_in_context = !thought.is_empty();
            }
            "working_still_action" => {
                actions_body = section.body.clone();
            }
            "context compact" | "context_compact" | "compact" => {
                context_compact_body = section.body.clone();
            }
            "" if !has_sections && has_action_blocks && actions_body.is_empty() => {
                actions_body = section.body.clone();
            }
            _ => {}
        }
    }

    let continue_work = match status_raw.as_str() {
        "finished" | "done" | "complete" => false,
        "working" | "in_progress" | "in progress" => true,
        "" => actions_body.is_empty() || final_answer.is_empty(),
        _ => {
            repair_issue = Some("status_must_be_working_or_finished".to_string());
            true
        }
    };

    let mut next_actions = Vec::new();
    let mut action_groups = Vec::new();
    let context_compacts = parse_context_compact_section(&context_compact_body, &mut repair_issue);
    if !actions_body.is_empty() {
        let blocks = extract_action_blocks(&actions_body);

        let trimmed_actions_body = actions_body.trim();
        if blocks.is_empty()
            && (trimmed_actions_body.starts_with('{') || trimmed_actions_body.starts_with('['))
        {
            if let Ok(value) = serde_json::from_str::<Value>(trimmed_actions_body) {
                match parse_action_groups_value(&value, capabilities) {
                    Ok(groups) => {
                        next_actions.extend(groups.iter().flat_map(|group| group.actions.clone()));
                        action_groups.extend(groups);
                    }
                    Err(issue) => repair_issue = Some(issue),
                }
            } else {
                repair_issue = Some("actions_section_invalid_json".to_string());
            }
        } else {
            for (idx, block) in blocks.iter().enumerate() {
                match serde_json::from_str::<Value>(block) {
                    Ok(value) => match parse_action_groups_value(&value, capabilities) {
                        Ok(groups) => {
                            next_actions
                                .extend(groups.iter().flat_map(|group| group.actions.clone()));
                            action_groups.extend(groups);
                        }
                        Err(issue) => {
                            repair_issue = Some(issue);
                            break;
                        }
                    },
                    Err(_) => {
                        repair_issue = Some(format!("actions[{idx}].invalid_json"));
                        break;
                    }
                }
            }
        }
    }

    // Validation
    if repair_issue.is_none() && !continue_work && final_answer.trim().is_empty() {
        repair_issue = Some("final_answer_required_when_status_finished".to_string());
    }
    if repair_issue.is_none() && !toolgen_retrospect.trim().is_empty() {
        let retrospect_index = sections.iter().position(|section| {
            matches!(
                section.heading.as_str(),
                "toolgen_retrospect" | "toolgen retrospect"
            )
        });
        let final_index = sections.iter().position(|section| {
            matches!(
                section.heading.as_str(),
                "answer" | "final_answer" | "final answer"
            )
        });
        if continue_work || final_answer.trim().is_empty() {
            repair_issue = Some("toolgen_retrospect_requires_final_answer".to_string());
        } else if match retrospect_index.zip(final_index) {
            Some((retrospect, final_answer)) => retrospect + 1 != final_answer,
            None => true,
        } {
            repair_issue = Some("toolgen_retrospect_must_precede_final_answer".to_string());
        }
    }
    if repair_issue.is_none()
        && !final_answer.trim().is_empty()
        && status_raw != "finished"
        && status_raw != "done"
        && status_raw != "complete"
    {
        repair_issue = Some("final_answer_requires_status_finished".to_string());
    }
    let runtime_note: Option<String> = None;
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
        memory_candidates: vec![],
        runtime_note,
        repair_issue,
    }
}

fn looks_like_external_tool_call_protocol(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("<tool_call")
        || lower.contains("</tool_call>")
        || lower.contains("<function_call")
        || lower.contains("</function_call>")
}

fn parse_context_compact_section(
    body: &str,
    repair_issue: &mut Option<String>,
) -> Vec<ParsedContextCompact> {
    if body.trim().is_empty() {
        return Vec::new();
    }
    let mut discard_delta_ids = Vec::new();
    let mut offload_delta_ids = Vec::new();
    let mut summary_lines = Vec::new();
    let mut in_summary = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("discard:") {
            discard_delta_ids = split_id_list(rest);
            in_summary = false;
        } else if let Some(rest) = trimmed.strip_prefix("offload:") {
            offload_delta_ids = split_id_list(rest);
            in_summary = false;
        } else if let Some(rest) = trimmed.strip_prefix("delta_ids:") {
            discard_delta_ids = split_id_list(rest);
            in_summary = false;
        } else if let Some(rest) = trimmed.strip_prefix("summary:") {
            in_summary = true;
            if !rest.trim().is_empty() {
                summary_lines.push(rest.trim().to_string());
            }
        } else if in_summary {
            summary_lines.push(line.to_string());
        }
    }
    let summary = summary_lines.join("\n").trim().to_string();
    let mut delta_ids = discard_delta_ids.clone();
    delta_ids.extend(offload_delta_ids.iter().cloned());
    delta_ids.sort();
    delta_ids.dedup();
    if delta_ids.is_empty() {
        if repair_issue.is_none() {
            *repair_issue = Some("context_compact.ids_required".to_string());
        }
        return Vec::new();
    }
    if summary.is_empty() {
        if repair_issue.is_none() {
            *repair_issue = Some("context_compact.summary_required".to_string());
        }
        return Vec::new();
    }
    vec![ParsedContextCompact {
        discard_delta_ids,
        offload_delta_ids,
        delta_ids,
        slice_ids: Vec::new(),
        summary,
    }]
}

fn split_id_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .map(|item| item.trim_matches(['"', '\'', '[', ']']))
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn md_repair_instruction(issue: &str) -> &'static str {
    if matches!(
        issue,
        "unsupported_action:final_answer" | "unsupported_action:final_response"
    ) {
        return "检查到刚刚的输出格式有点问题：final_answer/final_response 不是工具 action。最终回答请使用 Markdown response protocol：写 `## Status` 为 `finished`，并写 `## Final_Answer`。不要把最终回答放进 `## Working_Still_Action`。";
    }
    match issue {
        "truncated_model_output" => {
            "检查到刚刚的输出被 max output token 截断。请继续使用 Markdown response protocol，输出更短的 `## Free_talk`/`## Final_Answer`，长报告可用 `run_bash` 写入文件后在回答中给出路径。不要切换成顶层 JSON。"
        }
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你给了最终回答内容，但没有明确完成状态。如果当前用户请求已经完成，请写 `## Status` 为 `finished`，并写 `## Final_Answer`；finished 不会关闭 Timem session。如果仍需要 runtime 继续工作，请不要写 `## Final_Answer`，改写 `## Free_talk` 和 `## Working_Still_Action`。"
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你写了 `## Status` 为 `finished`，但缺少 `## Final_Answer`。如果当前用户请求已经完成，请同时提供 `## Status` 和 `## Final_Answer`；finished 不会关闭 Timem session。如果仍需要 runtime 继续工作，请不要写 finished，并提供 `## Free_talk` 和需要的 `## Working_Still_Action`。"
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：`## Status` finished 表示当前用户请求已完成，因此不能同时包含 `## Working_Still_Action`。如果还需要 runtime 执行动作，请保持 working，用 `## Free_talk` 和 `## Working_Still_Action` 继续；拿到 action result 后再写 finished 和 `## Final_Answer`。"
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：请继续使用 Markdown response protocol。`## Status` working 表示还需要 runtime 继续执行动作，因此必须提供 `## Free_talk` 和 `## Working_Still_Action`。如果当前用户请求已经完成，请改用 `## Status` finished 和 `## Final_Answer`；finished 不会关闭 Timem session。"
        }
        "external_tool_call_protocol" => {
            "检查到刚刚的输出用了外部 tool_call/function_call 格式。Timem 不能执行这种格式。请继续使用 Markdown response protocol：需要动作时写 `## Free_talk` 和 `## Working_Still_Action`，动作放在 action JSON block 中；完成时写 `## Status` finished 和 `## Final_Answer`。"
        }
        _ => {
            "Use the Markdown response protocol. If work still needs runtime action, write `## Free_talk` and concrete `## Working_Still_Action`. If the current user request is complete, write `## Status` with `finished` and provide `## Final_Answer`; this does not close the Timem session. Do not switch to a top-level JSON response."
        }
    }
}

pub fn md_repair_reason(issue: &str) -> &'static str {
    super::json_suite::protocol_repair_reason(issue)
}

pub fn md_focused_repair_text(issue: &str, original: &str) -> String {
    super::json_suite::focused_repair_response_text(issue, original)
}

pub fn md_can_show_plain_text_after_repair_failure(content: &str) -> bool {
    super::json_suite::can_show_plain_text_after_repair_failure(content)
}

#[cfg(test)]
#[path = "../../tests/unit/response_protocol_markdown_suite_tests.rs"]
mod tests;

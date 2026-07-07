use serde_json::Value;

use super::{
    ActionGroupOrder, ParsedAction, ParsedActionGroup, ParsedContextCompact, ParsedEnvelope,
    ResponseProtocolSuite,
};
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
            | "progress"
            | "report"
            | "report_job_progress"
            | "answer"
            | "final_answer"
            | "final answer"
            | "free_talk"
            | "free talk"
            | "freetalk"
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

fn parse_single_action(
    value: &Value,
    idx: usize,
    capabilities: &CapabilityRegistry,
) -> Result<ParsedAction, String> {
    parse_single_action_with_fallback(value, idx, capabilities, None)
}

fn parse_single_action_with_fallback(
    value: &Value,
    idx: usize,
    capabilities: &CapabilityRegistry,
    fallback_intent: Option<&str>,
) -> Result<ParsedAction, String> {
    let name = value
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if name.is_empty() {
        return Err(format!("actions[{idx}].action_missing"));
    }

    let action_args = value.get("args");
    let input = match action_args {
        Some(Value::Object(_)) => action_args.cloned().unwrap_or(Value::Null),
        Some(_) => return Err(format!("actions[{idx}].args_must_be_object")),
        None => return Err(format!("actions[{idx}].args_required")),
    };

    let intent = value
        .get("intent")
        .or_else(|| input.get("intent"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let parent_intent = if intent.is_empty() {
        fallback_intent
            .map(str::trim)
            .filter(|intent| !intent.is_empty())
            .map(ToString::to_string)
    } else {
        None
    };

    if !capabilities.contains_tool(&name) {
        return Err(format!("unsupported_action:{name}"));
    }

    if let Err(issue) = capabilities.validate_action_input(&name, &input) {
        return Err(format!("actions[{idx}].{issue}"));
    }

    Ok(ParsedAction {
        action: name,
        intent,
        parent_intent,
        raw_input: input,
    })
}

fn parse_action_groups_value(
    value: &Value,
    capabilities: &CapabilityRegistry,
) -> Result<Vec<ParsedActionGroup>, String> {
    if value.is_object() && value.get("action").is_some() {
        return Ok(vec![ParsedActionGroup {
            order: ActionGroupOrder::Sequential,
            actions: vec![parse_single_action(value, 0, capabilities)?],
        }]);
    }
    let Some(items) = value.as_array() else {
        return Err("actions_section_must_be_action_or_array".to_string());
    };
    let mut groups = Vec::new();
    for (group_idx, group) in items.iter().enumerate() {
        if group.get("actions").is_some() || group.get("order").is_some() {
            let order = group
                .get("order")
                .and_then(Value::as_str)
                .map(ActionGroupOrder::from_name)
                .unwrap_or(ActionGroupOrder::Sequential);
            let group_intent = group.get("intent").and_then(Value::as_str);
            let Some(actions) = group.get("actions").and_then(Value::as_array) else {
                return Err(format!("action_groups[{group_idx}].actions_required"));
            };
            let mut parsed_actions = Vec::new();
            for (action_idx, action) in actions.iter().enumerate() {
                parsed_actions.push(parse_single_action_with_fallback(
                    action,
                    action_idx,
                    capabilities,
                    group_intent,
                )?);
            }
            groups.push(ParsedActionGroup {
                order,
                actions: parsed_actions,
            });
        } else {
            groups.push(ParsedActionGroup {
                order: ActionGroupOrder::Sequential,
                actions: vec![parse_single_action(group, group_idx, capabilities)?],
            });
        }
    }
    Ok(groups)
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
        || value
            .as_object()
            .is_some_and(|object| object.contains_key("action"))
        || value.as_array().is_some_and(|items| {
            !items.is_empty()
                && items.iter().all(|item| {
                    item.as_object()
                        .is_some_and(|object| object.contains_key("action"))
                })
        })
}

fn fenced_json_contains_protocol_markers(fenced: &str) -> bool {
    contains_protocol_json_markers(fenced)
}

fn contains_protocol_json_markers(text: &str) -> bool {
    [
        "\"status\"",
        "\"report_job_progress\"",
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
        repair_issue: Some(issue.to_string()),
    }
}

pub fn parse_markdown_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let trimmed = content.trim();

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
                report_job_progress: String::new(),
                final_answer: candidate.to_string(),
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
            report_job_progress: String::new(),
            final_answer: candidate.to_string(),
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
            report_job_progress: String::new(),
            final_answer: candidate.to_string(),
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
    let mut report_job_progress = String::new();
    let mut final_answer = String::new();
    let mut thought = String::new();
    let mut thought_keep_in_context = false;
    let mut actions_body = String::new();
    let mut context_compact_body = String::new();
    let mut repair_issue: Option<String> = None;

    for section in &sections {
        match section.heading.as_str() {
            "status" => status_raw = section.body.trim().to_lowercase(),
            "progress" | "report" | "report_job_progress" => {
                report_job_progress = section.body.clone();
            }
            "answer" | "final_answer" | "final answer" => {
                final_answer = section.body.clone();
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
            "" => {
                if !has_sections && has_action_blocks && actions_body.is_empty() {
                    actions_body = section.body.clone();
                } else if !section.body.is_empty()
                    && sections.len() > 1
                    && report_job_progress.is_empty()
                {
                    report_job_progress = section.body.clone();
                }
            }
            _ => {}
        }
    }

    let continue_work = match status_raw.as_str() {
        "finished" | "done" | "complete" => false,
        "working" | "in_progress" | "in progress" => true,
        "" => {
            if !actions_body.is_empty() {
                true
            } else if !final_answer.is_empty() {
                false
            } else {
                true
            }
        }
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
    if repair_issue.is_none() && !final_answer.trim().is_empty() {
        if status_raw != "finished" && status_raw != "done" && status_raw != "complete" {
            repair_issue = Some("final_answer_requires_status_finished".to_string());
        }
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
        report_job_progress,
        final_answer,
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
    let mut delta_ids = Vec::new();
    let mut summary_lines = Vec::new();
    let mut in_summary = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("delta_ids:") {
            delta_ids = split_id_list(rest);
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
            "检查到刚刚的输出被 max output token 截断。请继续使用 Markdown response protocol，输出更短的 `## Progress`/`## Final_Answer`，长报告可用 `run_bash` 写入文件后在回答中给出路径。不要切换成顶层 JSON。"
        }
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你给了最终回答内容，但没有明确完成状态。如果当前用户请求已经完成，请写 `## Status` 为 `finished`，并写 `## Final_Answer`；finished 不会关闭 Timem session。如果仍需要 runtime 继续工作，请不要写 `## Final_Answer`，改写 `## Progress` 和 `## Working_Still_Action`。"
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你写了 `## Status` 为 `finished`，但缺少 `## Final_Answer`。如果当前用户请求已经完成，请同时提供 `## Status` 和 `## Final_Answer`；finished 不会关闭 Timem session。如果仍需要 runtime 继续工作，请不要写 finished，并提供 `## Progress` 和需要的 `## Working_Still_Action`。"
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：`## Status` finished 表示当前用户请求已完成，因此不能同时包含 `## Working_Still_Action`。如果还需要 runtime 执行动作，请保持 working，用 `## Progress` 和 `## Working_Still_Action` 继续；拿到 action result 后再写 finished 和 `## Final_Answer`。"
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：请继续使用 Markdown response protocol。`## Status` working 表示还需要 runtime 继续执行动作，因此必须提供 `## Progress` 和 `## Working_Still_Action`。如果当前用户请求已经完成，请改用 `## Status` finished 和 `## Final_Answer`；finished 不会关闭 Timem session。"
        }
        "external_tool_call_protocol" => {
            "检查到刚刚的输出用了外部 tool_call/function_call 格式。Timem 不能执行这种格式。请继续使用 Markdown response protocol：需要动作时写 `## Progress` 和 `## Working_Still_Action`，动作放在 action JSON block 中；完成时写 `## Status` finished 和 `## Final_Answer`。"
        }
        _ => {
            "Use the Markdown response protocol. If work still needs runtime action, write `## Progress` and concrete `## Working_Still_Action`. If the current user request is complete, write `## Status` with `finished` and provide `## Final_Answer`; this does not close the Timem session. Do not switch to a top-level JSON response."
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
mod tests {
    use super::*;
    use crate::capability::CapabilityRegistry;

    fn caps() -> CapabilityRegistry {
        CapabilityRegistry::builtin()
    }

    #[test]
    fn plain_prose_becomes_final_answer() {
        let env = parse_markdown_envelope("Hello world", &caps());
        assert_eq!(env.final_answer, "Hello world");
        assert!(!env.continue_work);
        assert!(env.repair_issue.is_none());
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
{"action":"run_bash","args":{}}
```

## Progress
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
            "```json\n{\"report_job_progress\":\"bad dangling \\ path and raw \n newline\n```";
        let env = parse_markdown_envelope(input, &caps());

        assert_eq!(env.repair_issue.as_deref(), Some("invalid_json"));
        assert!(env.final_answer.is_empty());
    }

    #[test]
    fn unclosed_fenced_json_with_protocol_markers_requests_repair() {
        let input = "```json\n{\"report_job_progress\":\"bad dangling \\ path";
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
        let input = "## Status\nfinished\n\n## Progress\nDone";
        let env = parse_markdown_envelope(input, &caps());
        assert_eq!(
            env.repair_issue.as_deref(),
            Some("final_answer_required_when_status_finished")
        );
    }

    #[test]
    fn parses_context_compact_section() {
        let input = "## Progress\n整理上下文\n\n## Context Compact\ndelta_ids: pd_a, pd_b\nsummary:\n保留当前任务结论。\n下一步继续验证。\n\n## Working_Still_Action\n```action\n{\"action\":\"run_bash\",\"intent\":\"Check files.\",\"args\":{\"cmd\":\"pwd\"}}\n```";
        let env = parse_markdown_envelope(input, &caps());

        assert!(env.repair_issue.is_none());
        assert_eq!(env.context_compacts.len(), 1);
        assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a", "pd_b"]);
        assert!(env.context_compacts[0].slice_ids.is_empty());
        assert!(env.context_compacts[0].summary.contains("保留当前任务结论"));
        assert_eq!(env.next_actions.len(), 1);
    }

    #[test]
    fn actions_section_json_fence_still_parses_action() {
        let input = "## Progress\nchecking\n\n## Working_Still_Action\n```json\n{\"action\":\"run_bash\",\"intent\":\"Check files.\",\"args\":{\"cmd\":\"pwd\"}}\n```";
        let env = parse_markdown_envelope(input, &caps());

        assert!(env.repair_issue.is_none());
        assert!(env.continue_work);
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].action, "run_bash");
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
    }

    #[test]
    fn actions_section_accepts_group_array() {
        let input = r#"## Progress
checking

## Working_Still_Action
```action
[
  {
    "order": "parallel",
    "actions": [
      {"action":"run_bash","intent":"Check A.","args":{"cmd":"printf a"}},
      {"action":"run_bash","intent":"Check B.","args":{"cmd":"printf b"}}
    ]
  },
  {
    "order": "sequential",
    "actions": [
      {"action":"memmgr","intent":"Query durable memory.","args":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}}
    ]
  }
]
```"#;
        let env = parse_markdown_envelope(input, &caps());

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 2);
        assert_eq!(env.action_groups[0].order, ActionGroupOrder::Parallel);
        assert_eq!(env.action_groups[0].actions.len(), 2);
        assert_eq!(env.action_groups[1].order, ActionGroupOrder::Sequential);
        assert_eq!(env.next_actions.len(), 3);
    }

    #[test]
    fn actions_section_accepts_mixed_groups_and_actions_with_optional_intent() {
        let input = r#"## Progress
checking

## Working_Still_Action
```action
[
  {
    "order": "parallel",
    "intent": "Check both files.",
    "actions": [
      {"action":"run_bash","args":{"cmd":"printf a","timeout_ms":5000}},
      {"action":"run_bash","intent":"Check B.","args":{"cmd":"printf b","timeout_ms":5000}}
    ]
  },
  {"action":"run_bash","args":{"cmd":"pwd","timeout_ms":5000}}
]
```"#;
        let env = parse_markdown_envelope(input, &caps());

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 2);
        assert_eq!(env.action_groups[0].order, ActionGroupOrder::Parallel);
        assert_eq!(env.action_groups[0].actions[0].intent, "");
        assert_eq!(
            env.action_groups[0].actions[0].parent_intent.as_deref(),
            Some("Check both files.")
        );
        assert_eq!(env.action_groups[0].actions[1].intent, "Check B.");
        assert_eq!(env.action_groups[0].actions[1].parent_intent, None);
        assert_eq!(env.action_groups[1].order, ActionGroupOrder::Sequential);
        assert_eq!(env.action_groups[1].actions[0].intent, "");
        assert_eq!(env.action_groups[1].actions[0].parent_intent, None);
        assert_eq!(env.next_actions.len(), 3);
    }

    #[test]
    fn mixed_actions_preserve_model_order() {
        let input = r#"## Progress
checking

## Working_Still_Action
```action
[
  {"action":"run_bash","intent":"First.","args":{"cmd":"printf first","timeout_ms":5000}},
  {
    "order": "parallel",
    "intent": "Middle group.",
    "actions": [
      {"action":"run_bash","args":{"cmd":"printf middle-a","timeout_ms":5000}},
      {"action":"run_bash","args":{"cmd":"printf middle-b","timeout_ms":5000}}
    ]
  },
  {"action":"run_bash","intent":"Last.","args":{"cmd":"printf last","timeout_ms":5000}}
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
        let input = "我先说明一下处理计划。\n\n## Progress\nchecking\n\n## Working_Still_Action\n```action\n{\"action\":\"run_bash\",\"intent\":\"Check files.\",\"args\":{\"cmd\":\"pwd\"}}\n```";
        let env = parse_markdown_envelope(input, &caps());

        assert!(env.repair_issue.is_none());
        assert_eq!(env.report_job_progress, "checking");
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].action, "run_bash");
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
    }

    #[test]
    fn action_block_without_sections_is_working_protocol() {
        let input = "```action\n{\"action\":\"run_bash\",\"intent\":\"Check files.\",\"args\":{\"cmd\":\"pwd\"}}\n```";
        let env = parse_markdown_envelope(input, &caps());

        assert!(env.repair_issue.is_none());
        assert!(env.continue_work);
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].action, "run_bash");
    }

    #[test]
    fn actions_section_accepts_bare_json_array() {
        let input = "## Progress\nchecking\n\n## Working_Still_Action\n[{\"action\":\"run_bash\",\"intent\":\"Check files.\",\"args\":{\"cmd\":\"pwd\"}},{\"action\":\"memmgr\",\"intent\":\"Query durable memory.\",\"args\":{\"type\":\"durable\",\"op\":\"sql\",\"sql\":\"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5\",\"params\":[\"%project%\"],\"limit\":5}}]";
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
        let input = "some preface\n```action\n{\"action\":\"run_bash\",\"intent\":\"Check\"}\n```";
        let env = parse_markdown_envelope(input, &caps());

        assert_eq!(
            env.repair_issue.as_deref(),
            Some("actions[0].args_required")
        );
        assert!(env.final_answer.is_empty());
    }

    #[test]
    fn markdown_repair_instruction_stays_markdown_protocol() {
        let instruction = md_repair_instruction("next_actions_required_when_status_working");

        assert!(instruction.contains("Markdown response protocol"));
        assert!(instruction.contains("## Progress"));
        assert!(instruction.contains("## Working_Still_Action"));
        assert!(instruction.contains("## Status"));
        assert!(!instruction.contains("Return exactly one valid JSON object"));
        assert!(!instruction.contains("Do not use markdown fences"));

        let truncated = md_repair_instruction("truncated_model_output");
        assert!(truncated.contains("Markdown response protocol"));
        assert!(truncated.contains("max output token"));
        assert!(!truncated.contains("JSON object"));
    }
}

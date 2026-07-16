use super::{
    ParsedAction, ParsedActionGroup, ParsedContextCompact, ParsedEnvelope, ResponseProtocolSuite,
};
use crate::capability::CapabilityRegistry;
use serde_json::Value;

pub struct XmlSuiteV1;

const XML_RESPONSE_PROTOCOL_SECTION: &str =
    include_str!("../../../resources/protocol/xml/response_protocol.md");
const XML_RESPONSE_SCHEMA_SUMMARY: &str =
    include_str!("../../../resources/protocol/xml/response_schema_summary.md");

impl ResponseProtocolSuite for XmlSuiteV1 {
    fn name(&self) -> &str {
        "xml_v1"
    }
    fn lang_format(&self) -> &str {
        "XML"
    }
    fn protocol_schema(&self) -> &str {
        ""
    }
    fn protocol_examples(&self) -> &str {
        ""
    }
    fn response_schema_summary(&self) -> &str {
        XML_RESPONSE_SCHEMA_SUMMARY
    }
    fn protocol_prompt_section(&self) -> String {
        XML_RESPONSE_PROTOCOL_SECTION.to_string()
    }
    fn parse(&self, raw: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
        parse_xml_envelope(raw, capabilities)
    }
    fn repair_instruction(&self, issue: &str) -> &str {
        xml_repair_instruction(issue)
    }
    fn repair_instruction_for_response(&self, issue: &str, raw_response: &str) -> String {
        xml_repair_instruction_for_response(issue, raw_response)
    }
    fn repair_reason(&self, issue: &str) -> &str {
        xml_repair_reason(issue)
    }
    fn focused_repair_text(&self, issue: &str, text: &str) -> String {
        super::json_suite::focused_repair_response_text(issue, text)
    }
    fn can_show_plain_text_after_repair_failure(&self, content: &str) -> bool {
        xml_can_show_plain_text_after_repair_failure(content)
    }
}

pub fn parse_xml_envelope(content: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope {
    let trimmed = content.trim();
    let protocol_text = strip_surrounding_xml_fence(trimmed).unwrap_or(trimmed);
    if protocol_text.starts_with('{')
        || protocol_text.starts_with('[')
        || starts_with_markdown_protocol(protocol_text)
    {
        return malformed_xml_response("xml_response_root_missing");
    }
    if looks_like_external_tool_call_protocol(protocol_text) {
        return malformed_xml_response("external_tool_call_protocol");
    }
    if has_adjacent_response_roots(protocol_text) {
        return malformed_xml_response("xml_content_after_response");
    }

    let Some(response) = parse_response_fields(protocol_text) else {
        if protocol_text.is_empty() {
            return malformed_xml_response("empty_response");
        }
        if protocol_text.starts_with('<') {
            return malformed_xml_response(classify_xml_root_issue(protocol_text));
        }
        return malformed_xml_response("xml_response_root_missing");
    };

    let mut repair_issue = response.flow_issue.clone();
    let has_status = response.has_status;
    let final_answer = response.final_answer.clone();
    let thought = response.free_talk.clone();
    let thought_keep_in_context = !thought.trim().is_empty();

    let continue_work = final_answer.trim().is_empty();

    let context_compacts = if repair_issue.is_none() {
        parse_context_compacts_from_fields(&response, &mut repair_issue)
    } else {
        Vec::new()
    };
    let (next_actions, action_groups) = if repair_issue.is_none() {
        parse_action_blocks(
            response.action_json_blocks.clone(),
            capabilities,
            &mut repair_issue,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    if repair_issue.is_none() && has_status {
        repair_issue = Some("status_tag_not_supported".to_string());
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
        continue_work,
        thought,
        thought_keep_in_context,
        next_actions,
        action_groups,
        context_compacts,
        memory_candidates: vec![],
        runtime_note: None,
        repair_issue,
    }
}

fn has_adjacent_response_roots(text: &str) -> bool {
    let mut cursor = 0usize;
    while let Some(close_rel) = text[cursor..].find("</response>") {
        let close_start = cursor + close_rel;
        let after = close_start + "</response>".len();
        if !is_inside_cdata(text, close_start)
            && !is_inside_outer_text_field(text, close_start, "free_talk")
            && !is_inside_outer_text_field(text, close_start, "final_answer")
            && find_open_tag(text[after..].trim_start(), "response") == Some(0)
        {
            return true;
        }
        cursor = after;
    }
    false
}

fn is_inside_outer_text_field(text: &str, pos: usize, tag: &str) -> bool {
    let before = &text[..pos];
    let mut open_count = 0usize;
    let mut cursor = 0usize;
    while let Some(open_rel) = find_open_tag(&before[cursor..], tag) {
        open_count += 1;
        cursor += open_rel + tag.len() + 1;
    }
    let close_count = before.matches(&format!("</{tag}>")).count();
    open_count > close_count
}

fn strip_surrounding_xml_fence(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text.strip_prefix("```")?;
    let newline = rest.find('\n')?;
    let lang = rest[..newline].trim().to_ascii_lowercase();
    if lang != "xml" {
        return None;
    }
    let body = &rest[newline + 1..];
    let closing = body.rfind("```")?;
    if !body[closing + 3..].trim().is_empty() {
        return None;
    }
    Some(body[..closing].trim())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResponseFields {
    free_talk: String,
    final_answer: String,
    action_json_blocks: Vec<String>,
    context_compacts: Vec<ContextCompactFields>,
    has_status: bool,
    flow_issue: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ContextCompactFields {
    discard: String,
    offload: String,
    summary: String,
}

fn parse_response_fields(text: &str) -> Option<ResponseFields> {
    let text = text.trim();
    let open_start = find_open_tag(text, "response")?;
    if !text[..open_start].trim().is_empty() {
        return None;
    }
    let open_end = find_tag_end(text, open_start)?;
    if is_self_closing_start_tag(&text[open_start..=open_end]) {
        return None;
    }
    let close_start = find_last_close_tag(text, open_end + 1, "response")?;
    let close_end = close_start + "</response>".len();
    if !text[close_end..].trim().is_empty() {
        return None;
    }
    let body = &text[open_end + 1..close_start];
    Some(scan_response_body(body))
}

fn classify_xml_root_issue(text: &str) -> &'static str {
    let text = text.trim();
    let Some(open_start) = find_open_tag(text, "response") else {
        return "xml_response_root_missing";
    };
    if !text[..open_start].trim().is_empty() {
        return "xml_content_before_response";
    }
    let Some(open_end) = find_tag_end(text, open_start) else {
        return "xml_response_root_unclosed";
    };
    if is_self_closing_start_tag(&text[open_start..=open_end]) {
        return "xml_response_root_self_closing";
    }
    let Some(close_start) = find_last_close_tag(text, open_end + 1, "response") else {
        return "xml_response_root_unclosed";
    };
    let close_end = close_start + "</response>".len();
    if !text[close_end..].trim().is_empty() {
        return "xml_content_after_response";
    }
    "invalid_xml_response_root"
}

fn scan_response_body(body: &str) -> ResponseFields {
    const TOP_LEVEL_TAGS: &[&str] = &[
        "free_talk",
        "free-talk",
        "freetalk",
        "working_still_action",
        "context_compact",
        "final_answer",
        "status",
    ];
    let mut fields = ResponseFields::default();
    let mut cursor = 0usize;
    let mut last_order = 0usize;
    let mut state_branch_count = 0usize;
    let mut has_working_action = false;
    let mut has_final_answer = false;
    let mut has_free_talk = false;

    while let Some((open_start, tag)) = find_next_open_raw_tag(body, cursor, TOP_LEVEL_TAGS) {
        if fields.flow_issue.is_none() && !body[cursor..open_start].trim().is_empty() {
            fields.flow_issue = Some("xml_unexpected_content_inside_response".to_string());
        }
        let Some(open_end) = find_tag_end(body, open_start) else {
            fields
                .flow_issue
                .get_or_insert_with(|| format!("xml_malformed_tag:{tag}"));
            break;
        };
        let tag_order = if matches!(tag, "free_talk" | "free-talk" | "freetalk") {
            if has_free_talk && fields.flow_issue.is_none() {
                fields.flow_issue = Some("xml_duplicate_free_talk".to_string());
            }
            has_free_talk = true;
            1
        } else {
            state_branch_count += 1;
            if tag == "working_still_action" {
                has_working_action = true;
            }
            if tag == "final_answer" {
                has_final_answer = true;
            }
            2
        };
        if fields.flow_issue.is_none() && tag_order < last_order {
            fields.flow_issue = Some("xml_tags_out_of_order".to_string());
        }
        last_order = tag_order;

        if is_self_closing_start_tag(&body[open_start..=open_end]) {
            if tag == "status" {
                fields.has_status = true;
            }
            cursor = open_end + 1;
            continue;
        }

        let close_start = if tag == "final_answer" {
            find_last_close_tag(body, open_end + 1, tag)
        } else if tag == "working_still_action" || tag == "context_compact" {
            find_close_tag_outside_cdata(body, open_end + 1, tag)
        } else {
            find_close_tag(body, open_end + 1, tag)
        };
        let Some(close_start) = close_start else {
            fields
                .flow_issue
                .get_or_insert_with(|| format!("xml_unclosed_tag:{tag}"));
            break;
        };
        let inner = &body[open_end + 1..close_start];
        match tag {
            "free_talk" | "free-talk" | "freetalk" => {
                fields.free_talk = decode_xml_text(&unwrap_cdata_text(inner));
            }
            "final_answer" => {
                fields.final_answer = decode_xml_text(&unwrap_cdata_text(inner));
            }
            "status" => {
                fields.has_status = true;
            }
            "context_compact" => {
                fields
                    .context_compacts
                    .push(parse_context_compact_fields(inner));
            }
            "working_still_action" => {
                fields
                    .action_json_blocks
                    .extend(extract_action_json_blocks(inner));
            }
            _ => {}
        }
        cursor = close_start + close_tag_len(tag);
    }

    if fields.flow_issue.is_none() && !body[cursor..].trim().is_empty() {
        fields.flow_issue = Some("xml_unexpected_content_inside_response".to_string());
    }

    if fields.flow_issue.is_none() && has_working_action && has_final_answer {
        fields.flow_issue = Some("status_finished_must_not_include_next_actions".to_string());
    }
    if fields.flow_issue.is_none() && state_branch_count > 1 {
        fields.flow_issue = Some("state_branch_must_choose_one".to_string());
    }
    fields
}

fn parse_context_compact_fields(body: &str) -> ContextCompactFields {
    ContextCompactFields {
        discard: extract_tag_text(body, "discard", false)
            .or_else(|| extract_tag_text(body, "delta_ids", false))
            .unwrap_or_default(),
        offload: extract_tag_text(body, "offload", false).unwrap_or_default(),
        summary: extract_tag_text(body, "summary", true).unwrap_or_default(),
    }
}

fn extract_action_json_blocks(body: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(open_start) = find_open_tag(&body[cursor..], "action_json") {
        let open_start = cursor + open_start;
        let Some(open_end) = find_tag_end(body, open_start) else {
            break;
        };
        if is_self_closing_start_tag(&body[open_start..=open_end]) {
            cursor = open_end + 1;
            continue;
        }
        let content_start = open_end + 1;
        let after_open = body[content_start..].trim_start();
        let skipped_ws = body[content_start..].len() - after_open.len();
        if after_open.starts_with("<![CDATA[") {
            let cdata_start = content_start + skipped_ws + "<![CDATA[".len();
            if let Some((cdata_end, close_start)) =
                find_cdata_end_before_close_tag(body, cdata_start, "action_json")
            {
                blocks.push(body[cdata_start..cdata_end].to_string());
                cursor = close_start + close_tag_len("action_json");
                continue;
            }
        }
        let Some(close_start) = find_close_tag(body, content_start, "action_json") else {
            break;
        };
        blocks.push(decode_xml_text(body[content_start..close_start].trim()));
        cursor = close_start + close_tag_len("action_json");
    }
    blocks
}

fn find_cdata_end_before_close_tag(
    haystack: &str,
    from: usize,
    tag: &str,
) -> Option<(usize, usize)> {
    let mut cursor = from;
    while let Some(close_start) = find_close_tag(haystack, cursor, tag) {
        let before_close = haystack[from..close_start].trim_end();
        if let Some(cdata_end_rel) = before_close.rfind("]]>") {
            return Some((from + cdata_end_rel, close_start));
        }
        cursor = close_start + close_tag_len(tag);
    }
    None
}

fn extract_tag_text(body: &str, tag: &str, use_last_close: bool) -> Option<String> {
    let open_start = find_open_tag(body, tag)?;
    let open_end = find_tag_end(body, open_start)?;
    if is_self_closing_start_tag(&body[open_start..=open_end]) {
        return Some(String::new());
    }
    let close_start = if use_last_close {
        find_last_close_tag(body, open_end + 1, tag)?
    } else {
        find_close_tag(body, open_end + 1, tag)?
    };
    Some(decode_xml_text(&unwrap_cdata_text(
        &body[open_end + 1..close_start],
    )))
}

fn unwrap_cdata_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
        trimmed["<![CDATA[".len()..trimmed.len() - "]]>".len()].to_string()
    } else {
        raw.to_string()
    }
}

fn find_close_tag(haystack: &str, from: usize, tag: &str) -> Option<usize> {
    let lower = haystack.to_ascii_lowercase();
    lower[from..]
        .find(&format!("</{}>", tag.to_ascii_lowercase()))
        .map(|pos| from + pos)
}

fn find_close_tag_outside_cdata(haystack: &str, from: usize, tag: &str) -> Option<usize> {
    let lower = haystack.to_ascii_lowercase();
    let needle = format!("</{}>", tag.to_ascii_lowercase());
    let mut cursor = from;
    while let Some(rel) = lower[cursor..].find(&needle) {
        let pos = cursor + rel;
        if !is_inside_cdata(haystack, pos) {
            return Some(pos);
        }
        cursor = pos + needle.len();
    }
    None
}

fn find_last_close_tag(haystack: &str, from: usize, tag: &str) -> Option<usize> {
    let lower = haystack.to_ascii_lowercase();
    lower[from..]
        .rfind(&format!("</{}>", tag.to_ascii_lowercase()))
        .map(|pos| from + pos)
}

fn close_tag_len(tag: &str) -> usize {
    format!("</{tag}>").len()
}

fn malformed_xml_response(issue: &str) -> ParsedEnvelope {
    ParsedEnvelope {
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

fn starts_with_markdown_protocol(text: &str) -> bool {
    text.starts_with("## ")
}

fn looks_like_external_tool_call_protocol(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("<tool_call")
        || lower.contains("</tool_call>")
        || lower.contains("<function_call")
        || lower.contains("</function_call>")
}

fn find_next_open_raw_tag<'a>(
    haystack: &str,
    from: usize,
    tags: &'a [&str],
) -> Option<(usize, &'a str)> {
    tags.iter()
        .filter_map(|tag| find_open_tag(&haystack[from..], tag).map(|pos| (from + pos, *tag)))
        .min_by_key(|(pos, _)| *pos)
}

fn find_open_tag(haystack: &str, tag: &str) -> Option<usize> {
    let lower = haystack.to_ascii_lowercase();
    let needle = format!("<{}", tag.to_ascii_lowercase());
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find(&needle) {
        let pos = cursor + rel;
        if is_inside_cdata(haystack, pos) {
            cursor = pos + needle.len();
            continue;
        }
        let after = lower.as_bytes().get(pos + needle.len()).copied();
        if matches!(
            after,
            Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        ) {
            return Some(pos);
        }
        cursor = pos + needle.len();
    }
    None
}

fn is_inside_cdata(text: &str, pos: usize) -> bool {
    let before = &text[..pos];
    let Some(open) = before.rfind("<![CDATA[") else {
        return false;
    };
    match before.rfind("]]>") {
        Some(close) => close < open,
        None => true,
    }
}

fn find_tag_end(text: &str, open_start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (offset, byte) in text.as_bytes()[open_start..].iter().copied().enumerate() {
        match (quote, byte) {
            (Some(q), b) if b == q => quote = None,
            (None, b'"') | (None, b'\'') => quote = Some(byte),
            (None, b'>') => return Some(open_start + offset),
            _ => {}
        }
    }
    None
}

fn is_self_closing_start_tag(tag_text: &str) -> bool {
    tag_text.trim_end_matches('>').trim_end().ends_with('/')
}

fn decode_xml_text(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_action_blocks(
    action_blocks: Vec<String>,
    capabilities: &CapabilityRegistry,
    repair_issue: &mut Option<String>,
) -> (Vec<ParsedAction>, Vec<ParsedActionGroup>) {
    if action_blocks.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut action_groups = Vec::new();
    for (block_idx, block) in action_blocks.iter().enumerate() {
        match serde_json::from_str::<Value>(block.trim()) {
            Ok(value) => {
                if !value.is_array() {
                    if value.as_object().is_some_and(|object| {
                        object.contains_key("order") || object.contains_key("actions")
                    }) {
                        match super::parse_action_workflow_value(
                            &value,
                            &format!("actions[{block_idx}]"),
                            capabilities,
                        ) {
                            Ok(_) => {}
                            Err(issue) => {
                                *repair_issue = Some(issue);
                                return (Vec::new(), Vec::new());
                            }
                        }
                    }
                    *repair_issue = Some(format!("actions[{block_idx}].array_required"));
                    return (Vec::new(), Vec::new());
                }
                match super::parse_action_workflow_value(
                    &value,
                    &format!("actions[{block_idx}]"),
                    capabilities,
                ) {
                    Ok(groups) => action_groups.extend(groups),
                    Err(issue) => {
                        *repair_issue = Some(issue);
                        return (Vec::new(), Vec::new());
                    }
                }
            }
            Err(_) => {
                *repair_issue = Some(format!("actions[{block_idx}].invalid_json"));
                return (Vec::new(), Vec::new());
            }
        }
    }
    let next_actions = action_groups
        .iter()
        .flat_map(|group| group.actions.clone())
        .collect::<Vec<_>>();
    (next_actions, action_groups)
}

fn parse_context_compacts_from_fields(
    response: &ResponseFields,
    repair_issue: &mut Option<String>,
) -> Vec<ParsedContextCompact> {
    let mut compacts = Vec::new();
    for (idx, item) in response.context_compacts.iter().enumerate() {
        let discard_delta_ids = split_id_list(&item.discard);
        let offload_delta_ids = split_id_list(&item.offload);
        let mut delta_ids = discard_delta_ids.clone();
        delta_ids.extend(offload_delta_ids.iter().cloned());
        delta_ids.sort();
        delta_ids.dedup();
        let summary = item.summary.trim().to_string();
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

fn split_id_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .map(|item| item.trim_matches(['"', '\'', '[', ']']))
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub fn xml_repair_instruction(issue: &str) -> &'static str {
    match issue {
        "empty_response" => {
            "检查到模型没有生成可解析的内容。请重新输出一个完整的 <response>...</response>；需要继续执行动作时在其中提供 <working_still_action>，已经完成时提供 <final_answer>。"
        }
        "truncated_model_output" => {
            "检查到刚刚的输出被 max output token 截断。请继续使用 XML response protocol，输出更短的 <free_talk> 或 <final_answer>；长报告可用 run_bash 写入文件后在回答中给出路径。"
        }
        "external_tool_call_protocol" => {
            "检查到刚刚的输出用了外部 tool_call/function_call 格式。Timem 不能执行这种格式。请继续使用 XML response protocol：需要动作时写 <free_talk> 和 <working_still_action><action_json><![CDATA[[...]]></action_json></working_still_action>；完成时直接写 <final_answer>...</final_answer>。"
        }
        "status_tag_not_supported" => {
            "检查到刚刚的输出格式有点问题：当前 XML response protocol 不使用 <status>。如果当前用户请求已经完成，请直接提供 <final_answer>；如果仍需 runtime 继续工作，请提供 <free_talk> 和 <working_still_action>。"
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：<final_answer> 表示当前用户请求已完成，因此不能同时包含 <working_still_action>。如果还需要 runtime 执行动作，请用 <free_talk> 和 <working_still_action> 继续；拿到 action result 后再写 <final_answer>。"
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：如果仍需 runtime 继续执行动作，必须提供 <working_still_action>；如果当前用户请求已经完成，请改用 <final_answer>。"
        }
        "invalid_xml_response_root" => {
            "The response must be exactly one <response>...</response> root element, with no text or tags before <response> or after </response>. Put <free_talk> and the selected state branch inside that root."
        }
        "xml_response_root_missing" => {
            "The required <response> root is missing. Return XML only, beginning with <response> and ending with </response>."
        }
        "xml_response_root_unclosed" => {
            "The <response> root is not completely closed. Return one complete <response>...</response> document."
        }
        "xml_response_root_self_closing" => {
            "A self-closing <response/> cannot contain the required response branch. Use <response>...</response> with exactly one state branch inside."
        }
        "xml_content_before_response" => {
            "The response contains text or tags before <response>. Move all response fields inside the single <response> root."
        }
        "xml_content_after_response" => {
            "The response contains text or tags after </response>. Return exactly one XML root and remove all trailing content."
        }
        "xml_unexpected_content_inside_response" => {
            "The <response> body contains text or an unknown top-level tag outside a supported field. Put text inside <free_talk> or <final_answer>, and use only one supported state branch."
        }
        "xml_duplicate_free_talk" => {
            "The response contains more than one <free_talk> field. Merge them into one optional <free_talk> before the state branch."
        }
        issue if issue.starts_with("xml_unclosed_tag:") => {
            "A response field tag is not closed. Close the named tag before writing the next field or </response>."
        }
        issue if issue.starts_with("xml_malformed_tag:") => {
            "A response field opening tag is malformed. Rewrite that field with a complete opening tag, matching closing tag, and no broken attributes."
        }
        "xml_tags_out_of_order" => {
            "The XML tags are out of order. Inside <response>, put optional <free_talk> first, followed by exactly one of <working_still_action>, <context_compact>, or <final_answer>."
        }
        "state_branch_must_choose_one" => {
            "The response selected more than one state branch. Inside <response>, use exactly one of <working_still_action>, <context_compact>, or <final_answer>."
        }
        issue if issue.ends_with(".invalid_json") => {
            "The <action_json> content is not valid JSON. Keep it inside <![CDATA[...]]>, use one top-level JSON array, and ensure every string and special character is valid JSON."
        }
        issue if issue.ends_with(".action_missing") => {
            "An action entry is missing its tool-name key. In the top-level workflow array, write each sequential action as {\"tool_name\":{...}}; write a parallel stage as an inner array of those tool objects."
        }
        issue if issue.ends_with(".args_must_be_object") => {
            "A tool value is not a JSON object. Write each action as {\"tool_name\":{\"argument\":\"value\"}}, even when the tool has no arguments."
        }
        issue if issue.ends_with(".old_group_object_not_supported") => {
            "The action payload used the removed {\"order\":...,\"actions\":[...]} group shape. Use an inner JSON array for a parallel stage and preserve outer-array order for sequential stages."
        }
        issue if issue.ends_with(".actions_required") => {
            "The action workflow contains an empty or incomplete stage. Provide at least one {\"tool_name\":{...}} action object in every stage."
        }
        issue if issue.starts_with("unsupported_action:") => {
            "The response requested a tool that is not in the available capability catalog. Choose an available tool name and keep its arguments inside that tool's JSON object."
        }
        issue if issue.contains(".input.") => {
            "The tool arguments do not satisfy the capability specification. Keep the same XML/action-array structure, then correct the missing, invalid, or conditionally required argument named in error."
        }
        issue if issue.starts_with("context_compact[") && issue.ends_with(".ids_required") => {
            "The <context_compact> block must contain at least one non-empty <discard> or <offload> delta-id list, followed by <summary>."
        }
        issue if issue.starts_with("context_compact[") && issue.ends_with(".summary_required") => {
            "The <context_compact> block is missing a non-empty <summary> describing the essential retained task state."
        }
        issue if issue.ends_with(".array_required") => {
            "检查到刚刚的 action_json 格式有点问题：<action_json> 的内容必须是 JSON array。单个工具调用也请写成数组，例如 <![CDATA[[{\"run_bash\":{\"cmd\":\"pwd\"}}]]]>。"
        }
        _ => {
            "Use the XML response protocol. If work still needs runtime action, write <free_talk> and concrete <working_still_action>. If the current user request is complete, write <final_answer>; this does not close the Timem session."
        }
    }
}

pub fn xml_repair_instruction_for_response(issue: &str, raw_response: &str) -> String {
    if !matches!(
        issue,
        "invalid_xml_response_root"
            | "xml_response_root_missing"
            | "xml_response_root_unclosed"
            | "xml_response_root_self_closing"
            | "xml_content_before_response"
            | "xml_content_after_response"
    ) {
        return xml_repair_instruction(issue).to_string();
    }

    let trimmed = raw_response.trim();
    let protocol_text = strip_surrounding_xml_fence(trimmed).unwrap_or(trimmed);
    let response_start = find_open_tag(protocol_text, "response");
    let has_content_before_root = response_start
        .map(|start| !protocol_text[..start].trim().is_empty())
        .unwrap_or(false);
    let branch = if protocol_text.contains("<working_still_action") {
        "<working_still_action>...</working_still_action>"
    } else if protocol_text.contains("<context_compact") {
        "<context_compact>...</context_compact>"
    } else if protocol_text.contains("<final_answer") {
        "<final_answer>...</final_answer>"
    } else {
        "<working_still_action>...</working_still_action>"
    };
    let free_talk = if protocol_text.contains("<free_talk")
        || protocol_text.contains("<free-talk")
        || protocol_text.contains("<freetalk")
    {
        "<free_talk>...</free_talk>"
    } else {
        ""
    };
    let expected = format!("<response>{free_talk}{branch}</response>");

    if issue == "xml_content_before_response" || has_content_before_root {
        return format!(
            "The previous output placed content before the <response> root. The response must be in format '{expected}'. Move every tag, including <free_talk>, inside <response>; output nothing before <response> or after </response>."
        );
    }
    if issue == "xml_content_after_response" {
        return format!(
            "The previous output placed content after the </response> root. The response must be in format '{expected}'. Output nothing before <response> or after </response>."
        );
    }
    if issue == "xml_response_root_unclosed" || response_start.is_some() {
        return format!(
            "The previous output did not form one complete <response>...</response> root. The response must be in format '{expected}'. Output nothing before <response> or after </response>."
        );
    }
    format!(
        "The previous output did not contain the required <response> root. The response must be in format '{expected}'."
    )
}

pub fn xml_repair_reason(issue: &str) -> &'static str {
    super::json_suite::protocol_repair_reason(issue)
}

pub fn xml_can_show_plain_text_after_repair_failure(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.starts_with('<') {
        return false;
    }
    super::json_suite::can_show_plain_text_after_repair_failure(content)
}

#[cfg(test)]
#[path = "../../tests/response_protocol/xml_suite_tests.rs"]
mod tests;

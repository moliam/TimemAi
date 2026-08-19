use super::{
    ParsedAction, ParsedActionGroup, ParsedContextCompact, ParsedEnvelope, PromptBoundarySpec,
    ResponseProtocolSuite, XML_PROMPT_BOUNDARIES,
};
use crate::capability::CapabilityRegistry;
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

pub struct XmlSuiteV1;

const XML_RESPONSE_PROTOCOL_SECTION: &str =
    include_str!("../../../resources/protocol/xml/response_protocol.md");

pub const FINISH_CONFIRM_PREFIX: &str = "Now let me think seriously twice before I announce stop. Review user's task list. Is my delivery consistent with user's demand?";
impl ResponseProtocolSuite for XmlSuiteV1 {
    fn name(&self) -> &str {
        "xml_v1"
    }
    fn prompt_boundaries(&self) -> &'static PromptBoundarySpec {
        &XML_PROMPT_BOUNDARIES
    }
    fn lang_format(&self) -> &str {
        "XML"
    }
    fn action_result_heading(&self) -> Option<&str> {
        None
    }
    fn response_shape_hint(&self) -> &str {
        "one-root label <ASSISTANT>...</ASSISTANT>"
    }
    fn protocol_schema(&self) -> &str {
        ""
    }
    fn protocol_examples(&self) -> &str {
        ""
    }
    fn response_schema_summary(&self) -> &str {
        ""
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
    let (root_was_extracted, protocol_text) = split_outer_response_text(trimmed);
    if protocol_text.starts_with('{')
        || protocol_text.starts_with('[')
        || starts_with_markdown_protocol(protocol_text)
    {
        return malformed_xml_response("xml_response_root_missing");
    }
    if looks_like_external_tool_call_protocol(protocol_text) {
        return malformed_xml_response("external_tool_call_protocol");
    }
    let original_protocol_text = protocol_text.to_string();
    let response = if let Some(response) = parse_response_fields(&original_protocol_text) {
        response
    } else {
        if original_protocol_text.is_empty() {
            return malformed_xml_response("empty_response");
        }

        if let Some(candidate) = repair_missing_response_root(&original_protocol_text) {
            let mut repaired = parse_xml_envelope(&candidate, capabilities);
            if repaired.repair_issue.is_none() {
                repaired.recovered_issue = Some("runtime_root_repair_help".to_string());
                return repaired;
            }
        }

        if original_protocol_text.starts_with('<') {
            return malformed_xml_response(classify_xml_root_issue(&original_protocol_text));
        }
        return malformed_xml_response("xml_response_root_missing");
    };
    let protocol_text = original_protocol_text;

    let response_was_recovered = root_was_extracted;
    let mut repair_issue = response.flow_issue.clone();
    let has_status = response.has_status;
    let has_finish_confirm = response.has_finish_confirm;
    let mut final_answer = response.final_answer.clone();
    let toolgen_retrospect = response.toolgen_retrospect.clone();
    let thought = response.free_talk.clone();
    let thought_keep_in_context = !thought.trim().is_empty();

    if response_was_recovered && !final_answer.trim().is_empty() {
        // Terminal completion is a hard protocol boundary. Even if another XML
        // error was found first, never allow a final answer obtained from a
        // runtime-completed or runtime-extracted root to reach the host.
        repair_issue = Some("xml_recovered_final_answer_requires_retry".to_string());
        // A recovered final answer must never cross the runtime's terminal boundary,
        // including after the protocol-repair retry budget is exhausted.
        final_answer.clear();
    }

    if !final_answer.trim().is_empty() && !response.finish_confirm_accepted {
        repair_issue
            .get_or_insert_with(|| "finish_confirm_required_before_final_answer".to_string());
        // The confirmation is a terminal safety boundary. Never expose a final
        // answer that did not pass it, even after the repair budget is exhausted.
        final_answer.clear();
    }

    let continue_work = final_answer.trim().is_empty();

    let context_compacts = if repair_issue.is_none() {
        parse_context_compacts_from_fields(&response, &mut repair_issue)
    } else {
        Vec::new()
    };
    let (next_actions, action_groups) = if repair_issue.is_none() {
        parse_response_actions(&response, capabilities, &mut repair_issue)
    } else {
        (Vec::new(), Vec::new())
    };

    if repair_issue.is_none() && has_status {
        repair_issue = Some("status_tag_not_supported".to_string());
    }
    debug_assert!(continue_work || has_finish_confirm);
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
        accepted_response: Some(protocol_text.clone()),
        // A complete recovered response is already the canonical replay value.
        // Do not turn successfully discarded outer noise into a new prompt
        // error; genuine structural failures still use `repair_issue` below.
        runtime_note: None,
        recovered_issue: None,
        repair_issue,
    }
}

fn repair_missing_response_root(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.starts_with('<') || !text.ends_with('>') {
        return None;
    }

    let needs_open = !text.starts_with("<ASSISTANT>");
    let needs_close = !text.ends_with("</ASSISTANT>");
    if !needs_open && !needs_close {
        return None;
    }

    let mut repaired = String::with_capacity(
        text.len()
            + usize::from(needs_open) * "<ASSISTANT>".len()
            + usize::from(needs_close) * "</ASSISTANT>".len(),
    );
    if needs_open {
        repaired.push_str("<ASSISTANT>");
    }
    repaired.push_str(text);
    if needs_close {
        repaired.push_str("</ASSISTANT>");
    }
    Some(repaired)
}

fn split_outer_response_text(text: &str) -> (bool, &str) {
    let text = text.trim();
    if let Some((start, end)) = largest_complete_response_root(text) {
        let extracted = start != 0 || end != text.len();
        return (extracted, text[start..end].trim());
    }

    // Recovery is extraction-only. Missing opening or closing response
    // boundaries remain protocol deviations and are never synthesized.
    (false, text)
}

fn largest_complete_response_root(text: &str) -> Option<(usize, usize)> {
    let mut open_cursor = 0usize;
    let mut best_clean: Option<(usize, usize)> = None;
    let mut best_repairable: Option<(usize, usize)> = None;

    while open_cursor < text.len() {
        let Some(open_rel) = find_open_tag(&text[open_cursor..], "ASSISTANT") else {
            break;
        };
        let open_start = open_cursor + open_rel;
        open_cursor = open_start.saturating_add("<ASSISTANT".len());

        if is_inside_cdata(text, open_start)
            || [
                "free_talk",
                "finish_confirm",
                "final_answer",
                "toolgen_retrospect",
                "summary",
            ]
            .iter()
            .any(|tag| is_inside_outer_text_field(text, open_start, tag))
        {
            continue;
        }

        let Some(open_end) = find_tag_end(text, open_start) else {
            continue;
        };
        if is_self_closing_start_tag(&text[open_start..=open_end]) {
            continue;
        }

        let mut close_cursor = open_end + 1;
        while let Some(close_start) = find_close_tag(text, close_cursor, "ASSISTANT") {
            close_cursor = close_start + "</ASSISTANT>".len();
            if is_inside_cdata(text, close_start) {
                continue;
            }

            let end = close_start + "</ASSISTANT>".len();
            let candidate = &text[open_start..end];
            let Some(fields) = parse_response_fields(candidate) else {
                continue;
            };

            // A literal </ASSISTANT> inside a text field can produce a
            // repairable-but-truncated candidate. Keep scanning: a later close
            // may form the actual outer root. Structurally clean candidates
            // outrank repairable candidates; within each class, select the
            // largest complete root and keep the first on equal length.
            let slot = if fields.flow_issue.is_none() {
                &mut best_clean
            } else {
                &mut best_repairable
            };
            let candidate_len = end.saturating_sub(open_start);
            let current_len = slot
                .map(|(start, end)| end.saturating_sub(start))
                .unwrap_or_default();
            if candidate_len > current_len {
                *slot = Some((open_start, end));
            }
        }
    }

    best_clean.or(best_repairable)
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResponseFields {
    free_talk: String,
    finish_confirm: String,
    toolgen_retrospect: String,
    final_answer: String,
    actions_xml: Vec<String>,
    context_compacts: Vec<ContextCompactFields>,
    has_status: bool,
    has_finish_confirm: bool,
    finish_confirm_valid: bool,
    finish_confirm_accepted: bool,
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
    let open_start = find_open_tag(text, "ASSISTANT")?;
    if !text[..open_start].trim().is_empty() {
        return None;
    }
    let open_end = find_tag_end(text, open_start)?;
    if is_self_closing_start_tag(&text[open_start..=open_end]) {
        return None;
    }
    let close_start = find_last_close_tag(text, open_end + 1, "ASSISTANT")?;
    let close_end = close_start + "</ASSISTANT>".len();
    if !text[close_end..].trim().is_empty() {
        return None;
    }
    let body = &text[open_end + 1..close_start];
    Some(scan_response_body(body))
}

fn classify_xml_root_issue(text: &str) -> &'static str {
    let text = text.trim();
    let Some(open_start) = find_open_tag(text, "ASSISTANT") else {
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
    let Some(close_start) = find_last_close_tag(text, open_end + 1, "ASSISTANT") else {
        return "xml_response_root_unclosed";
    };
    let close_end = close_start + "</ASSISTANT>".len();
    if !text[close_end..].trim().is_empty() {
        return "xml_content_after_response";
    }
    "invalid_xml_response_root"
}

fn scan_response_body(body: &str) -> ResponseFields {
    const TOP_LEVEL_TAGS: &[&str] = &[
        "free_talk",
        "finish_confirm",
        "actions",
        "context_compact",
        "toolgen_retrospect",
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
    let mut has_finish_confirm = false;
    let mut finish_confirm_count = 0usize;

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
        let tag_order = if tag == "free_talk" {
            if has_free_talk && fields.flow_issue.is_none() {
                fields.flow_issue = Some("xml_duplicate_free_talk".to_string());
            }
            has_free_talk = true;
            1
        } else if tag == "finish_confirm" {
            if has_finish_confirm && fields.flow_issue.is_none() {
                fields.flow_issue = Some("xml_duplicate_finish_confirm".to_string());
            }
            has_finish_confirm = true;
            finish_confirm_count += 1;
            2
        } else if tag == "toolgen_retrospect" {
            3
        } else {
            state_branch_count += 1;
            if tag == "actions" {
                has_working_action = true;
            }
            if tag == "final_answer" {
                has_final_answer = true;
            }
            4
        };
        if fields.flow_issue.is_none() && tag_order < last_order {
            fields.flow_issue = Some("xml_tags_out_of_order".to_string());
        }
        last_order = tag_order;

        if is_self_closing_start_tag(&body[open_start..=open_end]) {
            if tag == "status" {
                fields.has_status = true;
            } else if tag == "finish_confirm" {
                fields.has_finish_confirm = true;
                fields
                    .flow_issue
                    .get_or_insert_with(|| "finish_confirm_prefix_invalid".to_string());
            }
            cursor = open_end + 1;
            continue;
        }

        let close_start = if matches!(tag, "final_answer" | "toolgen_retrospect") {
            find_last_close_tag(body, open_end + 1, tag)
        } else if matches!(tag, "actions" | "context_compact") {
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
            "free_talk" => {
                fields.free_talk = decode_xml_field_text(inner);
            }
            "finish_confirm" => {
                fields.has_finish_confirm = true;
                fields.finish_confirm = decode_xml_field_text(inner);
                fields.finish_confirm_valid =
                    fields.finish_confirm.starts_with(FINISH_CONFIRM_PREFIX);
                if fields.flow_issue.is_none() && !fields.finish_confirm_valid {
                    fields.flow_issue = Some("finish_confirm_prefix_invalid".to_string());
                }
            }
            "final_answer" => {
                fields.finish_confirm_accepted =
                    finish_confirm_count == 1 && fields.finish_confirm_valid;
                fields.final_answer = decode_xml_field_text(inner);
            }
            "toolgen_retrospect" => {
                fields.toolgen_retrospect = decode_xml_field_text(inner);
            }
            "status" => {
                fields.has_status = true;
            }
            "context_compact" => {
                fields
                    .context_compacts
                    .push(parse_context_compact_fields(inner));
            }
            "actions" => fields.actions_xml.push(inner.to_string()),
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
    if fields.flow_issue.is_none()
        && !fields.toolgen_retrospect.trim().is_empty()
        && !has_final_answer
    {
        fields.flow_issue = Some("toolgen_retrospect_requires_final_answer".to_string());
    }
    fields
}

fn parse_context_compact_fields(body: &str) -> ContextCompactFields {
    ContextCompactFields {
        discard: extract_tag_text(body, "discard", false).unwrap_or_default(),
        offload: extract_tag_text(body, "offload", false).unwrap_or_default(),
        summary: extract_tag_text(body, "summary", true).unwrap_or_default(),
    }
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
    Some(decode_xml_field_text(&body[open_end + 1..close_start]))
}

fn decode_xml_field_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
        trimmed["<![CDATA[".len()..trimmed.len() - "]]>".len()].to_string()
    } else {
        decode_xml_text(raw)
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
        toolgen_retrospect: String::new(),
        continue_work: true,
        thought: String::new(),
        thought_keep_in_context: false,
        next_actions: vec![],
        action_groups: vec![],
        context_compacts: vec![],
        memory_candidates: vec![],
        accepted_response: None,
        runtime_note: None,
        recovered_issue: None,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlActionElement {
    name: String,
    attributes: BTreeMap<String, String>,
    children: Vec<XmlActionElement>,
    text: String,
    text_has_cdata: bool,
    self_closing: bool,
}

const MAX_XML_ACTION_DEPTH: usize = 64;
const MAX_XML_ACTION_ELEMENTS: usize = 4096;

fn parse_response_actions(
    response: &ResponseFields,
    capabilities: &CapabilityRegistry,
    repair_issue: &mut Option<String>,
) -> (Vec<ParsedAction>, Vec<ParsedActionGroup>) {
    let mut action_groups = Vec::new();
    for (block_idx, block) in response.actions_xml.iter().enumerate() {
        match parse_xml_action_groups(block, capabilities, block_idx) {
            Ok(groups) => action_groups.extend(groups),
            Err(issue) => {
                *repair_issue = Some(issue);
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

fn parse_xml_action_groups(
    body: &str,
    capabilities: &CapabilityRegistry,
    block_idx: usize,
) -> Result<Vec<ParsedActionGroup>, String> {
    let elements =
        parse_xml_action_fragment(body).map_err(|issue| format!("actions[{block_idx}].{issue}"))?;
    if elements.is_empty() {
        return Err(format!("actions[{block_idx}].actions_required"));
    }
    let mut groups = Vec::new();
    for (idx, element) in elements.iter().enumerate() {
        let label = format!("actions[{block_idx}][{idx}]");
        if element.name == "parallel" {
            if !element.attributes.is_empty() || !element.text.trim().is_empty() {
                return Err(format!("{label}.parallel_content_invalid"));
            }
            if element.children.is_empty() {
                return Err(format!("{label}.actions_required"));
            }
            let actions = element
                .children
                .iter()
                .enumerate()
                .map(|(child_idx, child)| {
                    if child.name == "parallel" {
                        return Err(format!("{label}[{child_idx}].parallel_nested"));
                    }
                    parse_xml_tool_action(child, &format!("{label}[{child_idx}]"), capabilities)
                })
                .collect::<Result<Vec<_>, _>>()?;
            groups.push(ParsedActionGroup {
                order: super::ActionGroupOrder::Parallel,
                actions,
            });
        } else {
            groups.push(ParsedActionGroup {
                order: super::ActionGroupOrder::Sequential,
                actions: vec![parse_xml_tool_action(element, &label, capabilities)?],
            });
        }
    }
    Ok(groups)
}

const MAX_XML_ACTION_NAME_CHARS: usize = 128;

fn parse_xml_tool_action(
    element: &XmlActionElement,
    label: &str,
    capabilities: &CapabilityRegistry,
) -> Result<ParsedAction, String> {
    if !capabilities.contains_tool(&element.name) {
        return Err(format!("unsupported_action:{}", element.name));
    }
    if !element.text.trim().is_empty() {
        return Err(format!("{label}.tool_text_not_allowed"));
    }
    let action_name = element
        .attributes
        .get("name")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut input = Map::new();
    for (name, raw) in &element.attributes {
        if name == "name" {
            continue;
        }
        let schema = capabilities.tool_input_property_schema(&element.name, name);
        input.insert(
            name.clone(),
            xml_scalar_value(raw, schema, label, name, false)?,
        );
    }
    for child in &element.children {
        if input.contains_key(&child.name) {
            return Err(format!("{label}.input.{}_duplicate", child.name));
        }
        let schema = capabilities.tool_input_property_schema(&element.name, &child.name);
        input.insert(child.name.clone(), xml_element_value(child, schema, label)?);
    }
    let action_value = Value::Object(Map::from_iter([(
        element.name.clone(),
        Value::Object(input),
    )]));
    let mut action = super::parse_action_object(&action_value, label, capabilities)?;
    let Some(action_name) = action_name else {
        return Err(format!("{label}.name_required"));
    };
    if action_name.chars().count() > MAX_XML_ACTION_NAME_CHARS {
        return Err(format!("{label}.name_too_long"));
    }
    action.name = Some(action_name);
    Ok(action)
}

fn xml_element_value(
    element: &XmlActionElement,
    schema: Option<&Value>,
    label: &str,
) -> Result<Value, String> {
    if element.self_closing && xml_schema_kinds(schema).contains(&XmlSchemaKind::Null) {
        return Ok(Value::Null);
    }
    let schema_type = xml_schema_kind(schema);
    match schema_type {
        Some(XmlSchemaKind::Array) => {
            if !element.attributes.is_empty() || !element.text.trim().is_empty() {
                return Err(format!(
                    "{label}.input.{}_array_content_invalid",
                    element.name
                ));
            }
            element
                .children
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    if item.name != "item" {
                        return Err(format!("{label}.input.{}_item_required", element.name));
                    }
                    let item_schema = schema.and_then(|value| {
                        value
                            .get("prefixItems")
                            .and_then(Value::as_array)
                            .and_then(|items| items.get(idx))
                            .or_else(|| value.get("items").filter(|items| items.is_object()))
                    });
                    xml_element_value(item, item_schema, label)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        Some(XmlSchemaKind::Object) => {
            if !element.text.trim().is_empty() {
                return Err(format!(
                    "{label}.input.{}_object_text_invalid",
                    element.name
                ));
            }
            let properties = schema.and_then(|value| value.get("properties"));
            let mut object = Map::new();
            for (name, raw) in &element.attributes {
                let field_schema = xml_object_property_schema(schema, properties, name);
                object.insert(
                    name.clone(),
                    xml_scalar_value(raw, field_schema, label, name, false)?,
                );
            }
            for child in &element.children {
                if object.contains_key(&child.name) {
                    return Err(format!("{label}.input.{}_duplicate", child.name));
                }
                let field_schema = xml_object_property_schema(schema, properties, &child.name);
                object.insert(
                    child.name.clone(),
                    xml_element_value(child, field_schema, label)?,
                );
            }
            Ok(Value::Object(object))
        }
        _ if !element.children.is_empty() || !element.attributes.is_empty() => {
            let mut object = Map::new();
            for (name, raw) in &element.attributes {
                object.insert(name.clone(), Value::String(raw.clone()));
            }
            for child in &element.children {
                if object.contains_key(&child.name) {
                    return Err(format!("{label}.input.{}_duplicate", child.name));
                }
                object.insert(child.name.clone(), xml_element_value(child, None, label)?);
            }
            Ok(Value::Object(object))
        }
        _ => xml_scalar_value(
            &element.text,
            schema,
            label,
            &element.name,
            element.text_has_cdata,
        ),
    }
}

fn xml_object_property_schema<'a>(
    schema: Option<&'a Value>,
    properties: Option<&'a Value>,
    name: &str,
) -> Option<&'a Value> {
    properties.and_then(|value| value.get(name)).or_else(|| {
        schema?
            .get("additionalProperties")
            .filter(|value| value.is_object())
    })
}

fn xml_scalar_value(
    raw: &str,
    schema: Option<&Value>,
    label: &str,
    name: &str,
    preserve_whitespace: bool,
) -> Result<Value, String> {
    let text = if preserve_whitespace { raw } else { raw.trim() };
    match xml_schema_kind(schema) {
        Some(XmlSchemaKind::Integer) => parse_xml_integer(text)
            .map(Value::Number)
            .ok_or_else(|| format!("{label}.input.{name}_must_be_integer")),
        Some(XmlSchemaKind::Number) => text
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| format!("{label}.input.{name}_must_be_number")),
        Some(XmlSchemaKind::Boolean) => match text.to_ascii_lowercase().as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("{label}.input.{name}_must_be_boolean")),
        },
        Some(XmlSchemaKind::Null) if text.is_empty() => Ok(Value::Null),
        Some(XmlSchemaKind::Null) => Err(format!("{label}.input.{name}_must_be_null_or_empty")),
        None => Ok(xml_ambiguous_scalar_value(text, schema)),
        Some(_) => Ok(Value::String(text.to_string())),
    }
}

fn xml_ambiguous_scalar_value(text: &str, schema: Option<&Value>) -> Value {
    let kinds = xml_schema_kinds(schema);
    if kinds.contains(&XmlSchemaKind::Boolean) {
        match text.to_ascii_lowercase().as_str() {
            "true" => return Value::Bool(true),
            "false" => return Value::Bool(false),
            _ => {}
        }
    }
    if kinds.contains(&XmlSchemaKind::Integer) {
        if let Some(number) = parse_xml_integer(text) {
            return Value::Number(number);
        }
    }
    if kinds.contains(&XmlSchemaKind::Number) {
        if let Some(number) = text.parse::<f64>().ok().and_then(Number::from_f64) {
            return Value::Number(number);
        }
    }
    Value::String(text.to_string())
}

fn parse_xml_integer(text: &str) -> Option<Number> {
    if text.starts_with('-') {
        text.parse::<i64>().ok().map(Number::from)
    } else {
        text.parse::<u64>().ok().map(Number::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XmlSchemaKind {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
    Null,
}

fn xml_schema_kind(schema: Option<&Value>) -> Option<XmlSchemaKind> {
    let schema = schema?;
    if let Some(raw) = schema.get("type").and_then(Value::as_str) {
        return xml_schema_kind_name(raw);
    }
    if let Some(types) = schema.get("type").and_then(Value::as_array) {
        return one_effective_xml_kind(
            types
                .iter()
                .filter_map(|value| value.as_str().and_then(xml_schema_kind_name)),
        );
    }
    for alternatives in ["anyOf", "oneOf"] {
        if let Some(items) = schema.get(alternatives).and_then(Value::as_array) {
            if let Some(kind) =
                one_effective_xml_kind(items.iter().filter_map(|item| xml_schema_kind(Some(item))))
            {
                return Some(kind);
            }
        }
    }
    if let Some(value) = schema.get("const") {
        return xml_json_value_kind(value);
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return one_effective_xml_kind(values.iter().filter_map(xml_json_value_kind));
    }
    if schema.get("properties").is_some() || schema.get("additionalProperties").is_some() {
        return Some(XmlSchemaKind::Object);
    }
    if schema.get("items").is_some() || schema.get("prefixItems").is_some() {
        return Some(XmlSchemaKind::Array);
    }
    None
}

fn xml_schema_kinds(schema: Option<&Value>) -> Vec<XmlSchemaKind> {
    let Some(schema) = schema else {
        return Vec::new();
    };
    let mut kinds = Vec::new();
    if let Some(raw) = schema.get("type").and_then(Value::as_str) {
        kinds.extend(xml_schema_kind_name(raw));
    } else if let Some(types) = schema.get("type").and_then(Value::as_array) {
        kinds.extend(
            types
                .iter()
                .filter_map(Value::as_str)
                .filter_map(xml_schema_kind_name),
        );
    } else {
        for alternatives in ["anyOf", "oneOf"] {
            if let Some(items) = schema.get(alternatives).and_then(Value::as_array) {
                for item in items {
                    for kind in xml_schema_kinds(Some(item)) {
                        if !kinds.contains(&kind) {
                            kinds.push(kind);
                        }
                    }
                }
            }
        }
    }
    if kinds.is_empty() {
        if let Some(value) = schema.get("const") {
            kinds.extend(xml_json_value_kind(value));
        } else if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            for value in values {
                if let Some(kind) = xml_json_value_kind(value) {
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
            }
        }
    }
    kinds
}

fn xml_json_value_kind(value: &Value) -> Option<XmlSchemaKind> {
    match value {
        Value::Null => Some(XmlSchemaKind::Null),
        Value::Bool(_) => Some(XmlSchemaKind::Boolean),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(XmlSchemaKind::Integer),
        Value::Number(_) => Some(XmlSchemaKind::Number),
        Value::String(_) => Some(XmlSchemaKind::String),
        Value::Array(_) => Some(XmlSchemaKind::Array),
        Value::Object(_) => Some(XmlSchemaKind::Object),
    }
}

fn xml_schema_kind_name(raw: &str) -> Option<XmlSchemaKind> {
    if raw.contains("integer") {
        Some(XmlSchemaKind::Integer)
    } else if raw.contains("number") {
        Some(XmlSchemaKind::Number)
    } else if raw.contains("boolean") {
        Some(XmlSchemaKind::Boolean)
    } else if raw.contains("array") {
        Some(XmlSchemaKind::Array)
    } else if raw.contains("object") {
        Some(XmlSchemaKind::Object)
    } else if raw.contains("null") {
        Some(XmlSchemaKind::Null)
    } else if raw.contains("string") {
        Some(XmlSchemaKind::String)
    } else {
        None
    }
}

fn one_effective_xml_kind(kinds: impl Iterator<Item = XmlSchemaKind>) -> Option<XmlSchemaKind> {
    let mut selected = None;
    let mut saw_null = false;
    for kind in kinds {
        if kind == XmlSchemaKind::Null {
            saw_null = true;
            continue;
        }
        match selected {
            None => selected = Some(kind),
            Some(XmlSchemaKind::Integer) if kind == XmlSchemaKind::Number => {
                selected = Some(XmlSchemaKind::Number)
            }
            Some(XmlSchemaKind::Number) if kind == XmlSchemaKind::Integer => {}
            Some(current) if current == kind => {}
            Some(_) => return None,
        }
    }
    selected.or_else(|| saw_null.then_some(XmlSchemaKind::Null))
}

fn parse_xml_action_fragment(body: &str) -> Result<Vec<XmlActionElement>, String> {
    let mut cursor = 0usize;
    let mut element_count = 0usize;
    let mut elements = Vec::new();
    skip_xml_space(body, &mut cursor);
    while cursor < body.len() {
        elements.push(parse_xml_action_element(
            body,
            &mut cursor,
            0,
            &mut element_count,
        )?);
        skip_xml_space(body, &mut cursor);
    }
    Ok(elements)
}

fn parse_xml_action_element(
    body: &str,
    cursor: &mut usize,
    depth: usize,
    element_count: &mut usize,
) -> Result<XmlActionElement, String> {
    if depth > MAX_XML_ACTION_DEPTH {
        return Err("xml_depth_limit_exceeded".to_string());
    }
    *element_count += 1;
    if *element_count > MAX_XML_ACTION_ELEMENTS {
        return Err("xml_element_limit_exceeded".to_string());
    }
    expect_xml(body, cursor, "<", "element_open_required")?;
    if body[*cursor..].starts_with('/') || body[*cursor..].starts_with('!') {
        return Err("unexpected_element_boundary".to_string());
    }
    let name = parse_xml_name(body, cursor).ok_or_else(|| "element_name_required".to_string())?;
    let mut attributes = BTreeMap::new();
    loop {
        skip_xml_space(body, cursor);
        if body[*cursor..].starts_with("/>") {
            *cursor += 2;
            return Ok(XmlActionElement {
                name,
                attributes,
                children: Vec::new(),
                text: String::new(),
                text_has_cdata: false,
                self_closing: true,
            });
        }
        if body[*cursor..].starts_with('>') {
            *cursor += 1;
            break;
        }
        let attr =
            parse_xml_name(body, cursor).ok_or_else(|| "attribute_name_required".to_string())?;
        skip_xml_space(body, cursor);
        expect_xml(body, cursor, "=", "attribute_equals_required")?;
        skip_xml_space(body, cursor);
        let quote = body
            .as_bytes()
            .get(*cursor)
            .copied()
            .ok_or_else(|| "attribute_value_required".to_string())?;
        if !matches!(quote, b'\'' | b'"') {
            return Err("attribute_quote_required".to_string());
        }
        *cursor += 1;
        let start = *cursor;
        let rel = body[start..]
            .find(quote as char)
            .ok_or_else(|| "attribute_unclosed".to_string())?;
        *cursor = start + rel + 1;
        let raw = &body[start..start + rel];
        if raw.contains('<') {
            return Err("attribute_xml_escape_required".to_string());
        }
        let value = decode_xml_action_text(raw)?;
        if attributes.insert(attr, value).is_some() {
            return Err("attribute_duplicate".to_string());
        }
    }

    let mut children = Vec::new();
    let mut text = String::new();
    let mut text_has_cdata = false;
    loop {
        if *cursor >= body.len() {
            return Err(format!("unclosed_tag:{name}"));
        }
        if body[*cursor..].starts_with("<![CDATA[") {
            let start = *cursor + "<![CDATA[".len();
            let rel = body[start..]
                .find("]]>")
                .ok_or_else(|| format!("unclosed_cdata:{name}"))?;
            text.push_str(&body[start..start + rel]);
            text_has_cdata = true;
            *cursor = start + rel + "]]>".len();
            continue;
        }
        if body[*cursor..].starts_with("</") {
            let close_start = *cursor;
            *cursor += 2;
            let close_name =
                parse_xml_name(body, cursor).ok_or_else(|| "close_name_required".to_string())?;
            skip_xml_space(body, cursor);
            expect_xml(body, cursor, ">", "close_boundary_required")?;
            if close_name != name {
                // A common model slip is to omit only the final tool close tag in a
                // parallel group. At depth 1 the current element is the tool itself,
                // so </parallel> is an unambiguous implicit end for that one tool.
                // Leave the parent close tag for the parent parser to consume. Never
                // apply this to argument children or arbitrary mismatched tags.
                if depth == 1 && close_name == "parallel" {
                    *cursor = close_start;
                    return Ok(XmlActionElement {
                        name,
                        attributes,
                        children,
                        text,
                        text_has_cdata,
                        self_closing: false,
                    });
                }
                return Err(format!("mismatched_close:{name}:{close_name}"));
            }
            return Ok(XmlActionElement {
                name,
                attributes,
                children,
                text,
                text_has_cdata,
                self_closing: false,
            });
        }
        if body[*cursor..].starts_with('<') {
            children.push(parse_xml_action_element(
                body,
                cursor,
                depth + 1,
                element_count,
            )?);
            continue;
        }
        let rel = body[*cursor..].find('<').unwrap_or(body.len() - *cursor);
        text.push_str(&decode_xml_action_text(&body[*cursor..*cursor + rel])?);
        *cursor += rel;
    }
}

fn decode_xml_action_text(text: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(text.len());
    let mut cursor = 0usize;
    while let Some(rel) = text[cursor..].find('&') {
        let start = cursor + rel;
        decoded.push_str(&text[cursor..start]);
        let Some(end_rel) = text[start + 1..].find(';') else {
            return Err("xml_entity_unclosed".to_string());
        };
        let end = start + 1 + end_rel;
        let entity = &text[start + 1..end];
        let value = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ if entity.starts_with("#x") => u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(valid_xml_char)
                .ok_or_else(|| "xml_entity_invalid".to_string())?,
            _ if entity.starts_with('#') => entity[1..]
                .parse::<u32>()
                .ok()
                .and_then(valid_xml_char)
                .ok_or_else(|| "xml_entity_invalid".to_string())?,
            _ => return Err("xml_entity_unsupported".to_string()),
        };
        decoded.push(value);
        cursor = end + 1;
    }
    decoded.push_str(&text[cursor..]);
    Ok(decoded)
}

fn valid_xml_char(value: u32) -> Option<char> {
    matches!(
        value,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
    .then(|| char::from_u32(value))
    .flatten()
}

fn parse_xml_name(body: &str, cursor: &mut usize) -> Option<String> {
    let start = *cursor;
    while let Some(byte) = body.as_bytes().get(*cursor).copied() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':') {
            *cursor += 1;
        } else {
            break;
        }
    }
    (*cursor > start).then(|| body[start..*cursor].to_string())
}

fn skip_xml_space(body: &str, cursor: &mut usize) {
    while body
        .as_bytes()
        .get(*cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        *cursor += 1;
    }
}

fn expect_xml(body: &str, cursor: &mut usize, expected: &str, issue: &str) -> Result<(), String> {
    if body[*cursor..].starts_with(expected) {
        *cursor += expected.len();
        Ok(())
    } else {
        Err(issue.to_string())
    }
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
            "检查到模型没有生成可解析的内容。请重新输出一个完整的 <ASSISTANT>...</ASSISTANT>；需要工具时提供 XML-native <actions>，已经完成时提供 <final_answer>。"
        }
        "truncated_model_output" => {
            "检查到刚刚的输出被 max output token 截断。请继续使用 XML response protocol，输出更短的 <free_talk> 或 <final_answer>；长报告可用 run_bash 写入文件后在回答中给出路径。"
        }
        "external_tool_call_protocol" => {
            "检查到刚刚的输出用了外部 tool_call/function_call 格式。请使用 XML-native actions：<ASSISTANT><actions><tool_id><argument>value</argument></tool_id></actions></ASSISTANT>；并行工具放入 <parallel>。"
        }
        "status_tag_not_supported" => {
            "检查到刚刚的输出格式有点问题：当前 XML response protocol 不使用 <status>。已完成时提供 <final_answer>；仍需 runtime 工具时提供 <actions>。"
        }
        "finish_confirm_required_before_final_answer" => {
            "检查到最终回答前缺少 <finish_confirm>。请在 <final_answer> 前加入 <finish_confirm>，并让其内容以协议中给定的 CONFIRM_PREFIX 完整开头。"
        }
        "finish_confirm_prefix_invalid" => {
            "检查到 <finish_confirm> 的内容没有以协议规定的 CONFIRM_PREFIX 完整开头。请保留标签位置并原样写出该前缀，然后再补充二次确认结论。"
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：<final_answer> 表示当前请求已完成，不能同时包含 <actions>。仍需工具时只选择 <actions>，拿到结果后再提供 <final_answer>。"
        }
        "xml_recovered_final_answer_requires_retry" => {
            "The runtime had to recover the outer XML boundary of a response containing <final_answer>. A recovered final answer cannot finish the task. Return the same answer again as one complete, unmodified <ASSISTANT><finish_confirm>CONFIRM_PREFIX followed by the confirmation</finish_confirm><final_answer>...</final_answer></ASSISTANT>, with nothing outside the root."
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：仍需 runtime 工具时必须提供非空 <actions>；如果当前请求已经完成，请改用 <final_answer>。"
        }
        "invalid_xml_response_root" => {
            "The response must be exactly one <ASSISTANT>...</ASSISTANT> root element, with no text or tags before <ASSISTANT> or after </ASSISTANT>. Put <free_talk> and the selected state branch inside that root."
        }
        "xml_response_root_missing" => {
            "The required <ASSISTANT> root is missing. Return XML only, beginning with <ASSISTANT> and ending with </ASSISTANT>."
        }
        "xml_response_root_unclosed" => {
            "The <ASSISTANT> root is not completely closed. Return one complete <ASSISTANT>...</ASSISTANT> document."
        }
        "xml_response_root_self_closing" => {
            "A self-closing <ASSISTANT/> cannot contain the required response branch. Use <ASSISTANT>...</ASSISTANT> with exactly one state branch inside."
        }
        "xml_content_before_response" => {
            "The response contains text or tags before <ASSISTANT>. Move all response fields inside the single <ASSISTANT> root."
        }
        "xml_content_after_response" => {
            "The response contains text or tags after </ASSISTANT>. Return exactly one XML root and remove all trailing content."
        }
        "xml_unexpected_content_inside_response" => {
            "The <ASSISTANT> body contains text or an unknown top-level tag outside a supported field. Put text inside <free_talk> or <final_answer>, and use only one supported state branch."
        }
        "xml_duplicate_free_talk" => {
            "The response contains more than one <free_talk> field. Merge them into one optional <free_talk> before the state branch."
        }
        "xml_duplicate_finish_confirm" => {
            "The response contains more than one <finish_confirm> field. Keep exactly one, after optional <free_talk> and before the selected state branch."
        }
        issue if issue.starts_with("xml_unclosed_tag:") => {
            "A response field tag is not closed. Close the named tag before writing the next field or </ASSISTANT>."
        }
        issue if issue.starts_with("xml_malformed_tag:") => {
            "A response field opening tag is malformed. Rewrite that field with a complete opening tag, matching closing tag, and no broken attributes."
        }
        "xml_tags_out_of_order" => {
            "The XML tags are out of order. Inside <ASSISTANT>, put optional <free_talk> first, optional <finish_confirm> next, then exactly one of <actions>, <context_compact>, or <final_answer>. A final answer requires <finish_confirm>."
        }
        "state_branch_must_choose_one" => {
            "The response selected more than one state branch. Inside <ASSISTANT>, use exactly one of <actions>, <context_compact>, or <final_answer>."
        }
        issue if issue.ends_with(".actions_required") => {
            "The <actions> or <parallel> element is empty. Add at least one concrete tool element from the capability catalog."
        }
        issue if issue.ends_with(".parallel_nested") => {
            "Nested <parallel> elements are not supported. Put concrete tool elements directly inside one <parallel>, or move later stages after it in <actions>."
        }
        issue if issue.contains(".unclosed_tag:") || issue.contains(".mismatched_close:") => {
            "An XML-native action or argument tag is not closed correctly. Match every opening tool/argument tag with the same closing tag."
        }
        issue if issue.contains(".attribute_") => {
            "An XML-native action attribute is malformed. Use a unique name and one quoted scalar value, for example timeout_ms=\"5000\"."
        }
        issue if issue.contains(".xml_entity_") => {
            "An XML action argument contains an invalid or unsupported entity. Escape XML-special characters with &amp;, &lt;, &gt;, &quot;, or &apos;; numeric character references are also accepted. For literal command text, use a leaf CDATA section."
        }
        issue if issue.contains(".unexpected_element_boundary") => {
            "XML declarations, DTDs, custom entities, and comments are not allowed inside <actions>. Remove that construct and keep only tool and argument elements."
        }
        issue if issue.contains(".xml_depth_limit_exceeded")
            || issue.contains(".xml_element_limit_exceeded") =>
        {
            "The XML action tree exceeds the runtime safety limit. Flatten unnecessary nesting or split the work across model rounds."
        }
        issue if issue.ends_with(".name_required") => {
            "Every XML tool action needs a non-empty, short, descriptive name attribute, for example <run_bash name=\"check git status\">...</run_bash>. The name is protocol metadata and is not passed to the tool."
        }
        issue if issue.ends_with(".name_too_long") => {
            "The XML action name is too long. Shorten it to at most 128 characters while keeping it descriptive; the name is protocol metadata and is not passed to the tool."
        }
        issue if issue.contains(".tool_text_not_allowed") => {
            "A tool element cannot contain bare text. Put each tool argument in an attribute or named child element."
        }
        issue if issue.contains(".parallel_content_invalid") => {
            "A <parallel> element may contain only concrete tool elements; remove attributes and bare text from the group."
        }
        issue if issue.starts_with("unsupported_action:") => {
            "The response requested a tool that is not in the capability catalog. Use an available exact tool id as the XML element name."
        }
        issue if issue.contains(".input.") => {
            "The XML tool arguments do not satisfy the capability schema. Correct the named attribute or child element; arrays use <item> children and objects use field-name children."
        }
        issue if issue.starts_with("context_compact[") && issue.ends_with(".ids_required") => {
            "The <context_compact> block must contain at least one non-empty <discard> or <offload> delta-id list, followed by <summary>."
        }
        issue if issue.starts_with("context_compact[") && issue.ends_with(".summary_required") => {
            "The <context_compact> block is missing a non-empty <summary> describing the essential retained task state."
        }
        _ => {
            "Use one XML <ASSISTANT>. If tools are needed, write XML-native <actions> with exact tool-id elements; if the current request is complete, write <final_answer>."
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
        let cause = xml_repair_cause(issue).unwrap_or_else(|| {
            "The previous XML violates the structural or capability rule identified by the exact error path."
                .to_string()
        });
        return format!(
            "Exact protocol error: `{issue}`. XML action indexes identify the <actions> block, stage, and tool in order; `input.<name>` identifies the argument.\nCause: {cause}\nCorrection: {}\nPreserve the parts of the previous XML that are already valid; change the smallest failing element or argument and return one complete <ASSISTANT>.",
            xml_repair_instruction(issue)
        );
    }

    let trimmed = raw_response.trim();
    let protocol_text = trimmed;
    let response_start = find_open_tag(protocol_text, "ASSISTANT");
    let has_content_before_root = response_start
        .map(|start| !protocol_text[..start].trim().is_empty())
        .unwrap_or(false);
    let branch = if protocol_text.contains("<actions") {
        "<actions>...</actions>"
    } else if protocol_text.contains("<context_compact") {
        "<context_compact>...</context_compact>"
    } else if protocol_text.contains("<final_answer") {
        "<final_answer>...</final_answer>"
    } else {
        "<actions>...</actions>"
    };
    let free_talk = if protocol_text.contains("<free_talk") {
        "<free_talk>...</free_talk>"
    } else {
        ""
    };
    let finish_confirm = if branch.starts_with("<final_answer") {
        "<finish_confirm>CONFIRM_PREFIX followed by the confirmation</finish_confirm>"
    } else {
        ""
    };
    let expected = format!("<ASSISTANT>{free_talk}{finish_confirm}{branch}</ASSISTANT>");

    if issue == "xml_content_before_response" || has_content_before_root {
        return format!(
            "Exact protocol error: `{issue}`.\nCause: The previous output placed content before the <ASSISTANT> root.\nCorrection: The response must be in format '{expected}'. Move every tag, including <free_talk>, inside <ASSISTANT>; output nothing before <ASSISTANT> or after </ASSISTANT>.\nPreserve valid inner content and return one complete <ASSISTANT>."
        );
    }
    if issue == "xml_content_after_response" {
        return format!(
            "Exact protocol error: `{issue}`.\nCause: The previous output placed content after the </ASSISTANT> root, usually trailing prose or another response root.\nCorrection: The response must be in format '{expected}'. Output nothing before <ASSISTANT> or after </ASSISTANT>.\nPreserve the first valid response content, remove the trailing duplicate or prose, and return one complete <ASSISTANT>."
        );
    }
    if issue == "xml_response_root_unclosed" || response_start.is_some() {
        return format!(
            "Exact protocol error: `{issue}`.\nCause: The previous output did not form one complete <ASSISTANT>...</ASSISTANT> root.\nCorrection: The response must be in format '{expected}'. Close the root and every inner tag; output nothing before <ASSISTANT> or after </ASSISTANT>.\nPreserve valid inner content and return one complete <ASSISTANT>."
        );
    }
    format!(
        "Exact protocol error: `{issue}`.\nCause: The previous output did not contain the required <ASSISTANT> root.\nCorrection: The response must be in format '{expected}'.\nPreserve valid content by moving it into the appropriate branch and return one complete <ASSISTANT>."
    )
}

fn xml_repair_cause(issue: &str) -> Option<String> {
    if issue == "xml_recovered_final_answer_requires_retry" {
        return Some(
            "The previous output contained a final answer inside a response root extracted from surrounding content. Terminal completion requires one unmodified protocol response with nothing outside the root."
                .to_string(),
        );
    }
    if issue.contains(".xml_entity_unclosed") {
        return Some(
            "An ampersand began an XML entity without a terminating semicolon.".to_string(),
        );
    }
    if issue.contains(".xml_entity_unsupported") {
        return Some(
            "The action used a named XML entity outside the five built-in XML entities."
                .to_string(),
        );
    }
    if issue.contains(".xml_entity_invalid") {
        return Some(
            "A numeric XML character reference is malformed or not a valid XML character."
                .to_string(),
        );
    }
    if issue.contains(".unexpected_element_boundary") {
        return Some("The action tree contains a declaration, DTD, comment, or other non-element XML construct.".to_string());
    }
    if issue.contains(".xml_depth_limit_exceeded") {
        return Some(format!(
            "The action tree is nested more than {MAX_XML_ACTION_DEPTH} levels deep."
        ));
    }
    if issue.contains(".xml_element_limit_exceeded") {
        return Some(format!(
            "The action tree contains more than {MAX_XML_ACTION_ELEMENTS} elements."
        ));
    }
    let input_issue = issue.rsplit_once(".input.")?.1;
    for (suffix, expected) in [
        ("_must_be_integer", "an integer"),
        ("_must_be_number", "a number"),
        ("_must_be_boolean", "true or false"),
        (
            "_must_be_null_or_empty",
            "null, represented by an empty element",
        ),
    ] {
        if let Some(name) = input_issue.strip_suffix(suffix) {
            return Some(format!(
                "Argument `{name}` must be {expected}; the previous XML supplied an incompatible scalar value."
            ));
        }
    }
    if let Some(name) = input_issue.strip_suffix("_duplicate") {
        return Some(format!(
            "Argument `{name}` was supplied more than once, such as both an attribute and a child element."
        ));
    }
    if let Some(name) = input_issue.strip_suffix("_required") {
        return Some(format!("Required argument `{name}` is missing or empty."));
    }
    if let Some(required) = input_issue.strip_prefix("any_required:") {
        return Some(format!(
            "At least one of these arguments is required: {}.",
            required.replace('|', ", ")
        ));
    }
    None
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

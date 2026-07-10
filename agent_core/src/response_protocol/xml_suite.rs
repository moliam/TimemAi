use super::{
    markdown_suite, ParsedAction, ParsedActionGroup, ParsedContextCompact, ParsedEnvelope,
    ResponseProtocolSuite,
};
use crate::capability::CapabilityRegistry;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
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
    if protocol_text.starts_with('{') || protocol_text.starts_with('[') {
        return super::json_suite::parse_envelope(protocol_text, capabilities);
    }
    if starts_with_markdown_protocol(protocol_text) {
        return markdown_suite::parse_markdown_envelope(protocol_text, capabilities);
    }
    if looks_like_external_tool_call_protocol(protocol_text) {
        return malformed_xml_response("external_tool_call_protocol");
    }

    let protected_protocol_text = protect_raw_text_fields(protocol_text);
    let parse_text = protected_protocol_text.as_deref().unwrap_or(protocol_text);

    let Some(response) = parse_xml_response_node(parse_text) else {
        if protocol_text.starts_with('<') {
            return malformed_xml_response("invalid_xml_response_root");
        }
        if protocol_text.is_empty() {
            return malformed_xml_response("empty_response");
        }
        return ParsedEnvelope {
            final_answer: protocol_text.to_string(),
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
    };

    let flow_issue = validate_response_branch_flow(&response);
    let mut repair_issue = None;
    let status_raw = response.first_child_text(&["status"]).to_ascii_lowercase();
    let final_answer = response.first_child_inner_xml(&["final_answer"]);
    let thought = response.first_child_inner_xml(&["free_talk", "free-talk", "freetalk"]);
    let thought_keep_in_context = !thought.trim().is_empty();

    let continue_work = match status_raw.trim() {
        "all_finished" => false,
        "working" | "in_progress" | "in progress" => true,
        "" => {
            if response.has_child("working_still_action") {
                true
            } else if !final_answer.trim().is_empty() {
                false
            } else {
                true
            }
        }
        _ => {
            repair_issue = Some("status_must_be_working_or_all_finished".to_string());
            true
        }
    };

    let context_compacts = parse_context_compacts_from_node(&response, &mut repair_issue);
    let (next_actions, action_groups) =
        parse_actions_from_node(&response, capabilities, &mut repair_issue);

    if repair_issue.is_none() && !continue_work && final_answer.trim().is_empty() {
        repair_issue = Some("final_answer_required_when_status_finished".to_string());
    }
    if repair_issue.is_none() && !final_answer.trim().is_empty() {
        if !matches!(status_raw.trim(), "all_finished") {
            repair_issue = Some("final_answer_requires_status_finished".to_string());
        }
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
    if repair_issue.is_none() {
        repair_issue = flow_issue;
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

fn validate_response_branch_flow(response: &XmlNode) -> Option<String> {
    let mut state_branch_count = 0usize;
    let mut last_order = 0usize;

    for child in response.children() {
        let order = if child.name.eq_ignore_ascii_case("free_talk")
            || child.name.eq_ignore_ascii_case("free-talk")
            || child.name.eq_ignore_ascii_case("freetalk")
        {
            1
        } else if child.name.eq_ignore_ascii_case("working_still_action")
            || child.name.eq_ignore_ascii_case("status")
            || child.name.eq_ignore_ascii_case("context_compact")
        {
            state_branch_count += 1;
            3
        } else if child.name.eq_ignore_ascii_case("final_answer") {
            4
        } else {
            continue;
        };

        if order < last_order {
            return Some("xml_tags_out_of_order".to_string());
        }
        last_order = order;
    }

    if state_branch_count > 1 {
        return Some("state_branch_must_choose_one".to_string());
    }
    None
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

fn protect_raw_text_fields(text: &str) -> Option<String> {
    const RAW_TEXT_TAGS: &[&str] = &[
        "free_talk",
        "free-talk",
        "freetalk",
        "final_answer",
        "summary",
    ];
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut changed = false;

    while let Some((open_start, tag)) = find_next_open_raw_tag(text, cursor, RAW_TEXT_TAGS) {
        let Some(open_end) = find_tag_end(text, open_start) else {
            break;
        };
        out.push_str(&text[cursor..=open_end]);
        cursor = open_end + 1;

        if is_self_closing_start_tag(&text[open_start..=open_end]) {
            continue;
        }

        let Some(close_start) = find_close_tag_for_raw_field(text, cursor, tag) else {
            continue;
        };
        let raw = &text[cursor..close_start];
        out.push_str(&raw_to_cdata(raw));
        let close_tag_len = format!("</{tag}>").len();
        out.push_str(&text[close_start..close_start + close_tag_len]);
        cursor = close_start + close_tag_len;
        changed = true;
    }

    out.push_str(&text[cursor..]);
    changed.then_some(out)
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
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
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

fn find_close_tag_for_raw_field(haystack: &str, from: usize, tag: &str) -> Option<usize> {
    let lower = haystack.to_ascii_lowercase();
    let needle = format!("</{}>", tag.to_ascii_lowercase());
    if tag.eq_ignore_ascii_case("final_answer") || tag.eq_ignore_ascii_case("summary") {
        lower[from..].rfind(&needle).map(|pos| from + pos)
    } else {
        lower[from..].find(&needle).map(|pos| from + pos)
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

fn raw_to_cdata(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("<![CDATA[") && trimmed.ends_with("]]>") {
        return raw.to_string();
    }
    let decoded = decode_xml_text(raw);
    format!("<![CDATA[{}]]>", decoded.replace("]]>", "]]]]><![CDATA[>"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum XmlFragment {
    Text(String),
    CData(String),
    Node(XmlNode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlNode {
    name: String,
    attributes: Vec<(String, String)>,
    self_closing: bool,
    fragments: Vec<XmlFragment>,
}

impl XmlNode {
    fn new(name: String, attributes: Vec<(String, String)>, self_closing: bool) -> Self {
        Self {
            name,
            attributes,
            self_closing,
            fragments: Vec::new(),
        }
    }

    fn has_child(&self, name: &str) -> bool {
        self.children()
            .any(|child| child.name.eq_ignore_ascii_case(name))
    }

    fn first_child_text(&self, names: &[&str]) -> String {
        self.first_child(names)
            .map(XmlNode::text_content)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn first_child_inner_xml(&self, names: &[&str]) -> String {
        self.first_child(names)
            .map(XmlNode::inner_xml)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn first_child(&self, names: &[&str]) -> Option<&XmlNode> {
        self.children().find(|child| {
            names
                .iter()
                .any(|name| child.name.eq_ignore_ascii_case(name))
        })
    }

    fn children(&self) -> impl Iterator<Item = &XmlNode> {
        self.fragments.iter().filter_map(|fragment| match fragment {
            XmlFragment::Node(node) => Some(node),
            XmlFragment::Text(_) => None,
            XmlFragment::CData(_) => None,
        })
    }

    fn text_content(&self) -> String {
        let mut out = String::new();
        for fragment in &self.fragments {
            match fragment {
                XmlFragment::Text(text) => out.push_str(&decode_xml_text(text)),
                XmlFragment::CData(text) => out.push_str(text),
                XmlFragment::Node(node) => out.push_str(&node.text_content()),
            }
        }
        out
    }

    fn inner_xml(&self) -> String {
        let mut out = String::new();
        for fragment in &self.fragments {
            match fragment {
                XmlFragment::Text(text) => out.push_str(&decode_xml_text(text)),
                XmlFragment::CData(text) => out.push_str(text),
                XmlFragment::Node(node) => out.push_str(&node.to_xml()),
            }
        }
        out
    }

    fn to_xml(&self) -> String {
        let attrs = self
            .attributes
            .iter()
            .map(|(key, value)| format!(r#" {key}="{value}""#))
            .collect::<String>();
        if self.self_closing && self.fragments.is_empty() {
            return format!("<{}{} />", self.name, attrs);
        }
        format!(
            "<{}{}>{}</{}>",
            self.name,
            attrs,
            self.inner_xml(),
            self.name
        )
    }
}

fn xml_node_from_start(start: &BytesStart<'_>, self_closing: bool) -> Option<XmlNode> {
    let name = String::from_utf8_lossy(start.name().as_ref()).to_string();
    let mut attributes = Vec::new();
    for attr in start.attributes().with_checks(false) {
        let attr = attr.ok()?;
        attributes.push((
            String::from_utf8_lossy(attr.key.as_ref()).to_string(),
            String::from_utf8_lossy(attr.value.as_ref()).to_string(),
        ));
    }
    Some(XmlNode::new(name, attributes, self_closing))
}

fn parse_xml_response_node(text: &str) -> Option<XmlNode> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                stack.push(xml_node_from_start(&start, false)?);
            }
            Ok(Event::Empty(start)) => {
                let node = xml_node_from_start(&start, true)?;
                if let Some(parent) = stack.last_mut() {
                    parent.fragments.push(XmlFragment::Node(node));
                } else if root.is_none() {
                    root = Some(node);
                }
            }
            Ok(Event::Text(text)) => {
                if let Some(node) = stack.last_mut() {
                    node.fragments.push(XmlFragment::Text(
                        String::from_utf8_lossy(text.as_ref()).to_string(),
                    ));
                }
            }
            Ok(Event::CData(text)) => {
                if let Some(node) = stack.last_mut() {
                    node.fragments.push(XmlFragment::CData(
                        String::from_utf8_lossy(text.as_ref()).to_string(),
                    ));
                }
            }
            Ok(Event::End(_)) => {
                let node = stack.pop()?;
                if let Some(parent) = stack.last_mut() {
                    parent.fragments.push(XmlFragment::Node(node));
                } else if root.is_none() {
                    root = Some(node);
                } else {
                    return None;
                }
            }
            Ok(Event::Eof) => break,
            Ok(Event::Comment(_))
            | Ok(Event::Decl(_))
            | Ok(Event::PI(_))
            | Ok(Event::DocType(_)) => {}
            Ok(Event::GeneralRef(reference)) => {
                if let Some(node) = stack.last_mut() {
                    node.fragments.push(XmlFragment::Text(format!(
                        "&{};",
                        String::from_utf8_lossy(reference.as_ref())
                    )));
                }
            }
            Err(_) => return None,
        }
    }

    if !stack.is_empty() {
        return None;
    }
    root.filter(|node| node.name.eq_ignore_ascii_case("response"))
}

fn decode_xml_text(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_actions_from_node(
    response: &XmlNode,
    capabilities: &CapabilityRegistry,
    repair_issue: &mut Option<String>,
) -> (Vec<ParsedAction>, Vec<ParsedActionGroup>) {
    let mut action_blocks = Vec::new();
    for action_section in response
        .children()
        .filter(|child| child.name.eq_ignore_ascii_case("working_still_action"))
    {
        let nested = action_section
            .children()
            .filter(|child| child.name.eq_ignore_ascii_case("action_json"))
            .collect::<Vec<_>>();
        if nested.is_empty() {
            let direct = action_section.text_content();
            if !direct.trim().is_empty() {
                action_blocks.push(direct);
            }
        } else {
            for item in nested {
                let decoded = item.text_content();
                if !decoded.trim().is_empty() {
                    action_blocks.push(decoded);
                }
            }
        }
    }
    parse_action_blocks(action_blocks, capabilities, repair_issue)
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
            Ok(value) => match super::parse_action_workflow_value(
                &value,
                &format!("actions[{block_idx}]"),
                capabilities,
            ) {
                Ok(groups) => action_groups.extend(groups),
                Err(issue) => {
                    *repair_issue = Some(issue);
                    return (Vec::new(), Vec::new());
                }
            },
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

fn parse_context_compacts_from_node(
    response: &XmlNode,
    repair_issue: &mut Option<String>,
) -> Vec<ParsedContextCompact> {
    let mut compacts = Vec::new();
    for (idx, node) in response
        .children()
        .filter(|child| child.name.eq_ignore_ascii_case("context_compact"))
        .enumerate()
    {
        let delta_ids = split_id_list(&node.first_child_text(&["delta_ids"]));
        let summary = node.first_child_inner_xml(&["summary"]).trim().to_string();
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
        "truncated_model_output" => {
            "检查到刚刚的输出被 max output token 截断。请继续使用 XML response protocol，输出更短的 <free_talk> 或 <final_answer>；长报告可用 run_bash 写入文件后在回答中给出路径。"
        }
        "external_tool_call_protocol" => {
            "检查到刚刚的输出用了外部 tool_call/function_call 格式。Timem 不能执行这种格式。请继续使用 XML response protocol：需要动作时写 <free_talk> 和 <working_still_action><action_json><![CDATA[{...}]]></action_json></working_still_action>；完成时写 <status>ALL_FINISHED</status> 和 <final_answer>...</final_answer>。"
        }
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你给了 <final_answer>，但没有明确 <status>ALL_FINISHED</status>。如果当前用户请求已经完成，请同时提供 <status>ALL_FINISHED</status> 和 <final_answer>；如果仍需 runtime 继续工作，请不要写 <final_answer>，改写 <free_talk> 和 <working_still_action>。"
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你写了 <status>ALL_FINISHED</status>，但缺少 <final_answer>。如果当前用户请求已经完成，请同时提供二者；如果 final_answer 里需要展示 XML 标签或 XML 示例，请把整个 final_answer 文本包进 <![CDATA[ ... ]]>，避免示例标签被当作协议标签解析。如果仍需 runtime 继续工作，请不要写 ALL_FINISHED，并提供 <free_talk> 和需要的 <working_still_action>。"
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：<status>ALL_FINISHED</status> 表示当前用户请求已完成，因此不能同时包含 <working_still_action>。如果还需要 runtime 执行动作，请保持 working，用 <free_talk> 和 <working_still_action> 继续；拿到 action result 后再写 ALL_FINISHED 和 <final_answer>。"
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：working 表示还需要 runtime 继续执行动作，因此必须提供 <free_talk> 和 <working_still_action>。如果当前用户请求已经完成，请改用 <status>ALL_FINISHED</status> 和 <final_answer>。"
        }
        _ => {
            "Use the XML response protocol. If work still needs runtime action, write <free_talk> and concrete <working_still_action>. If the current user request is complete, write <status>ALL_FINISHED</status> with <final_answer>; this does not close the Timem session."
        }
    }
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
mod tests {
    use super::*;

    fn caps() -> CapabilityRegistry {
        CapabilityRegistry::builtin()
    }

    #[test]
    fn parses_final_answer() {
        let env = parse_xml_envelope(
            "<response><status>ALL_FINISHED</status><final_answer>done</final_answer></response>",
            &caps(),
        );
        assert!(env.repair_issue.is_none());
        assert!(!env.continue_work);
        assert_eq!(env.final_answer, "done");
    }

    #[test]
    fn parses_final_answer_cdata_with_xml_examples() {
        let env = parse_xml_envelope(
            r#"<response>
  <status>ALL_FINISHED</status>
  <final_answer><![CDATA[
Example response delta:

<response>
  <status>ALL_FINISHED</status>
  <final_answer>done</final_answer>
</response>

[END DELTA]
  ]]></final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env.final_answer.contains("<response>"));
        assert!(env.final_answer.contains("</final_answer>"));
        assert!(env.final_answer.contains("[END DELTA]"));
    }

    #[test]
    fn final_answer_xml_action_examples_are_not_parsed_as_real_actions() {
        let env = parse_xml_envelope(
            r#"<response>
  <status>ALL_FINISHED</status>
  <final_answer><![CDATA[
This is only a user-facing example:

<working_still_action>
  <action_json>{"run_bash": {} // missing cmd in the example on purpose
  }</action_json>
</working_still_action>
  ]]></final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
        assert!(env.final_answer.contains("<working_still_action>"));
        assert!(env.final_answer.contains("\"run_bash\": {}"));
    }

    #[test]
    fn final_answer_raw_xml_code_block_is_opaque_text() {
        let env = parse_xml_envelope(
            r#"<response>
  <status>ALL_FINISHED</status>
  <final_answer>
Found the original malformed response:

```xml
<response>
  <free_talk>并行启动 3 个 sleep 15 的后台任务。</free_talk>
  <working_still_action>
    <action_json>
{
  "order": "parallel",
  "actions": [
    {"run_bash": { "cmd": "sleep 15", "background": true } },
    {"run_bash": { "cmd": "sleep 15", "background": true } },
    {"run_bash": { "cmd": "sleep 15", "background": true } }
  ]
}
    </action_json>
  </working_still_action>
</response>
```

The issue was the bare group object inside action_json.
  </final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
        assert!(env
            .final_answer
            .contains("Found the original malformed response"));
        assert!(env.final_answer.contains("<working_still_action>"));
        assert!(env.final_answer.contains(r#""order": "parallel""#));
    }

    #[test]
    fn final_answer_raw_unbalanced_xml_is_opaque_text() {
        let env = parse_xml_envelope(
            r#"<response>
  <status>ALL_FINISHED</status>
  <final_answer>
The previous bad output started like this:
<response>
  <free_talk>explaining an example without closing the root

Literal same-tag example:
<final_answer>inner sample</final_answer>

That was text, not a runtime action.
  </final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env.final_answer.contains("<response>"));
        assert!(env
            .final_answer
            .contains("<free_talk>explaining an example without closing the root"));
        assert!(env
            .final_answer
            .contains("<final_answer>inner sample</final_answer>"));
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
    }

    #[test]
    fn final_answer_raw_text_can_contain_other_string_tags_without_rescanning() {
        let env = parse_xml_envelope(
            r#"<response>
  <status>ALL_FINISHED</status>
  <final_answer>
This answer explains multiple protocol snippets:
<legacy_note>fake legacy note inside final answer</legacy_note>
<summary>fake compact summary inside final answer</summary>
<free_talk>fake free talk inside final answer</free_talk>
None of these are real control fields.
  </final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env.thought.is_empty());
        assert!(env.context_compacts.is_empty());
        assert!(env
            .final_answer
            .contains("<legacy_note>fake legacy note inside final answer</legacy_note>"));
        assert!(env
            .final_answer
            .contains("<summary>fake compact summary inside final answer</summary>"));
        assert!(env
            .final_answer
            .contains("<free_talk>fake free talk inside final answer</free_talk>"));
    }

    #[test]
    fn final_answer_raw_action_protocol_example_is_not_a_real_action() {
        let env = parse_xml_envelope(
            r#"<response>
<status>ALL_FINISHED</status>
<final_answer>
Here is the malformed response example the user asked for:
<response>
  <free_talk>not closed
<legacy_note>fake note</legacy_note>
<working_still_action><action_json>{"run_bash":{}}</action_json></working_still_action>
<summary>fake summary</summary>
This is all answer text.
</final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
        assert!(env.final_answer.contains("<working_still_action>"));
    }

    #[test]
    fn final_answer_nested_xml_preserves_attributes_and_escaped_text() {
        let env = parse_xml_envelope(
            r#"<response>
  <status>ALL_FINISHED</status>
  <final_answer>
Report:
<diagnostic level="warn" source="unit-test"><message>ok</message><empty marker="1" /></diagnostic>
Escaped literal: &lt;response&gt;not protocol&lt;/response&gt;
  </final_answer>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert!(env
            .final_answer
            .contains(r#"<diagnostic level="warn" source="unit-test">"#));
        assert!(env.final_answer.contains("<message>ok</message>"));
        assert!(env.final_answer.contains(r#"<empty marker="1" />"#));
        assert!(env
            .final_answer
            .contains("<response>not protocol</response>"));
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
    }

    #[test]
    fn free_talk_xml_action_examples_do_not_hide_real_actions() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk><![CDATA[
Example text only:
<working_still_action>
  <action_json>{"run_bash":{}}</action_json>
</working_still_action>
]]></free_talk>
<working_still_action>
<action_json><![CDATA[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(env.continue_work);
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
        assert!(env.thought.contains("<working_still_action>"));
    }

    #[test]
    fn free_talk_nested_xml_is_opaque_and_real_action_still_parses() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>
This is only a note:
<note priority="high"><working_still_action><action_json>{"run_bash":{}}</action_json></working_still_action></note>
</free_talk>
<working_still_action>
<action_json><![CDATA[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
        assert!(env.thought.contains(r#"<note priority="high">"#));
        assert!(env.thought.contains("<working_still_action>"));
    }

    #[test]
    fn free_talk_raw_xml_text_does_not_break_real_action() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>
I am explaining a malformed example:
<response><working_still_action><action_json>{ bad
</free_talk>
<working_still_action>
<action_json><![CDATA[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(env.thought.contains("<response><working_still_action>"));
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
    }

    #[test]
    fn string_field_protection_does_not_hide_malformed_action_json() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>text field can mention {"action":"run_bash"}</free_talk>
<working_still_action>
<action_json><![CDATA[
{"run_bash":{"cmd":"pwd",}}
]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert_eq!(env.repair_issue.as_deref(), Some("actions[0].invalid_json"));
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
    }

    #[test]
    fn old_finished_status_requests_repair() {
        let env = parse_xml_envelope(
            "<response><status>finished</status><final_answer>done</final_answer></response>",
            &caps(),
        );

        assert_eq!(
            env.repair_issue.as_deref(),
            Some("status_must_be_working_or_all_finished")
        );
        assert!(env.continue_work);
    }

    #[test]
    fn parses_actions_from_cdata_json() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>state</free_talk>
<working_still_action>
<action_json><![CDATA[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(env.continue_work);
        assert_eq!(env.thought, "state");
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].action, "run_bash");
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
    }

    #[test]
    fn rejects_old_group_object_from_action_json() {
        let env = parse_xml_envelope(
            r#"<response>
  <free_talk>并行启动 3 个 sleep 15 的后台任务。</free_talk>
  <working_still_action>
    <action_json><![CDATA[
{
  "order": "parallel",
  "actions": [
    {"run_bash": { "cmd": "sleep 15", "background": true } },
    {"run_bash": { "cmd": "sleep 15", "background": true } },
    {"run_bash": { "cmd": "sleep 15", "background": true } }
  ]
}
    ]]></action_json>
  </working_still_action>
</response>"#,
            &caps(),
        );

        assert_eq!(
            env.repair_issue.as_deref(),
            Some("actions[0].old_group_object_not_supported")
        );
        assert!(env.next_actions.is_empty());
        assert!(env.action_groups.is_empty());
    }

    #[test]
    fn parses_bare_action_array_as_parallel_group() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>parallel checks</free_talk>
<working_still_action>
<action_json><![CDATA[
[
  {"run_bash": { "cmd": "printf a", "timeout_ms": 5000 } },
  {"run_bash": { "cmd": "printf b", "timeout_ms": 5000 } }
]
]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 1);
        assert_eq!(
            env.action_groups[0].order,
            crate::ActionGroupOrder::Parallel
        );
        assert_eq!(env.action_groups[0].actions.len(), 2);
        assert_eq!(env.next_actions[0].input_str("cmd"), "printf a");
        assert_eq!(env.next_actions[1].input_str("cmd"), "printf b");
    }

    #[test]
    fn parses_nested_action_arrays_as_ordered_parallel_groups() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>stage then stage</free_talk>
<working_still_action>
<action_json><![CDATA[
[
  [
    {"run_bash": { "cmd": "printf a1", "timeout_ms": 5000 } },
    {"run_bash": { "cmd": "printf a2", "timeout_ms": 5000 } }
  ],
  [
    {"run_bash": { "cmd": "printf b1", "timeout_ms": 5000 } }
  ]
]
]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 2);
        assert_eq!(
            env.action_groups[0].order,
            crate::ActionGroupOrder::Parallel
        );
        assert_eq!(
            env.action_groups[1].order,
            crate::ActionGroupOrder::Parallel
        );
        assert_eq!(env.action_groups[0].actions.len(), 2);
        assert_eq!(env.action_groups[1].actions.len(), 1);
        assert_eq!(env.next_actions[0].input_str("cmd"), "printf a1");
        assert_eq!(env.next_actions[1].input_str("cmd"), "printf a2");
        assert_eq!(env.next_actions[2].input_str("cmd"), "printf b1");
    }

    #[test]
    fn action_args_can_contain_xml_like_text() {
        let env = parse_xml_envelope(
            r#"<response>
<working_still_action>
<action_json><![CDATA[
[
  {"run_bash": {
      "cmd": "printf group",
      "timeout_ms": 5000
    }
  },
  {"run_bash": {
      "cmd": "printf '%s\n' '<working_still_action><action_json>{\"action\":\"run_bash\"}</action_json></working_still_action>'",
      "timeout_ms": 5000
    }
  }
]
]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 1);
        assert_eq!(env.next_actions[0].input_str("cmd"), "printf group");
        assert!(env.next_actions[1]
            .input_str("cmd")
            .contains("<working_still_action>"));
    }

    #[test]
    fn action_args_strings_can_contain_protocol_isomorphic_text() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>query protocol-like text</free_talk>
<working_still_action>
<action_json><![CDATA[
[
  {"run_bash": {
      "cmd": "printf '%s\n' '<response><status>ALL_FINISHED</status><final_answer>not real</final_answer></response>' && printf '%s\n' '{\"working_still_action\":[{\"action\":\"run_bash\"}]}'",
      "timeout_ms": 5000
    }
  },
  {"memmgr": {
      "type": "raw_chat",
      "op": "sql",
      "sql": "SELECT content FROM chat_messages WHERE content LIKE ? LIMIT 5",
      "params": ["%</action_json><status>ALL_FINISHED</status><action_json>%"],
      "limit": 5
    }
  }
]
]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 1);
        assert_eq!(
            env.action_groups[0].order,
            crate::ActionGroupOrder::Parallel
        );
        assert_eq!(env.next_actions.len(), 2);
        assert!(env.next_actions[0]
            .input_str("cmd")
            .contains("<response><status>ALL_FINISHED</status>"));
        assert_eq!(
            env.next_actions[1].input_params(),
            vec!["%</action_json><status>ALL_FINISHED</status><action_json>%".to_string()]
        );
    }

    #[test]
    fn parses_context_compact() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>need compact</free_talk>
<context_compact>
<delta_ids>pd_a, pd_b</delta_ids>
<summary><![CDATA[keep state]]></summary>
</context_compact>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none());
        assert_eq!(env.context_compacts.len(), 1);
        assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a", "pd_b"]);
        assert_eq!(env.context_compacts[0].summary, "keep state");
    }

    #[test]
    fn context_compact_summary_raw_xml_is_opaque_text() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>need compact</free_talk>
<context_compact>
<delta_ids>pd_a</delta_ids>
<summary>
Keep this protocol example:
<response><status>ALL_FINISHED</status>
</summary>
</context_compact>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.context_compacts.len(), 1);
        assert!(env.context_compacts[0]
            .summary
            .contains("<response><status>ALL_FINISHED</status>"));
    }

    #[test]
    fn parses_response_wrapped_in_xml_markdown_fence() {
        let env = parse_xml_envelope(
            r#"```xml
<response>
  <free_talk>finished</free_talk>
  <status>ALL_FINISHED</status>
  <final_answer>done</final_answer>
</response>
```"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(!env.continue_work);
        assert_eq!(env.final_answer, "done");
        assert_eq!(env.thought, "finished");
    }

    #[test]
    fn xml_state_branch_must_choose_one() {
        let env = parse_xml_envelope(
            r#"<response>
<free_talk>compact and act</free_talk>
<context_compact>
<delta_ids>pd_a</delta_ids>
<summary>keep state</summary>
</context_compact>
<working_still_action>
<action_json><![CDATA[{"run_bash":{"cmd":"pwd"}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert_eq!(
            env.repair_issue.as_deref(),
            Some("state_branch_must_choose_one")
        );
    }

    #[test]
    fn repairs_external_tool_call_protocol() {
        let env = parse_xml_envelope(
            r#"<tool_call>{"name":"run_bash","arguments":{"cmd":"pwd"}}</tool_call>"#,
            &caps(),
        );
        assert_eq!(
            env.repair_issue.as_deref(),
            Some("external_tool_call_protocol")
        );
    }
}

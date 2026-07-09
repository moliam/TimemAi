use super::{
    markdown_suite, ParsedAction, ParsedActionGroup, ParsedContextCompact, ParsedEnvelope,
    ResponseProtocolSuite,
};
use crate::capability::CapabilityRegistry;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

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
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return super::json_suite::parse_envelope(content, capabilities);
    }
    if starts_with_markdown_protocol(trimmed) {
        return markdown_suite::parse_markdown_envelope(content, capabilities);
    }
    if looks_like_external_tool_call_protocol(trimmed) {
        return malformed_xml_response("external_tool_call_protocol");
    }

    let Some(response) = parse_xml_response_node(trimmed) else {
        if trimmed.starts_with('<') {
            return malformed_xml_response("invalid_xml_response_root");
        }
        if trimmed.is_empty() {
            return malformed_xml_response("empty_response");
        }
        return ParsedEnvelope {
            report_job_progress: String::new(),
            final_answer: trimmed.to_string(),
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

    let status_raw = response.first_child_text(&["status"]).to_ascii_lowercase();
    let report_job_progress =
        response.first_child_inner_xml(&["progress", "report", "report_job_progress"]);
    let final_answer = response.first_child_inner_xml(&["final_answer"]);
    let thought = response.first_child_inner_xml(&["free_talk", "free-talk", "freetalk"]);
    let thought_keep_in_context = !thought.trim().is_empty();

    let mut repair_issue = None;
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
        runtime_note: None,
        repair_issue,
    }
}

fn malformed_xml_response(issue: &str) -> ParsedEnvelope {
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

    let mut markdown = String::from("## Progress\nchecking\n\n## Working_Still_Action\n");
    for block in action_blocks {
        markdown.push_str("```action\n");
        markdown.push_str(block.trim());
        markdown.push_str("\n```\n");
    }
    let parsed = markdown_suite::parse_markdown_envelope(&markdown, capabilities);
    if let Some(issue) = parsed.repair_issue {
        *repair_issue = Some(issue);
        return (Vec::new(), Vec::new());
    }
    (parsed.next_actions, parsed.action_groups)
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
            "检查到刚刚的输出被 max output token 截断。请继续使用 XML response protocol，输出更短的 <progress> 或 <final_answer>；长报告可用 run_bash 写入文件后在回答中给出路径。"
        }
        "external_tool_call_protocol" => {
            "检查到刚刚的输出用了外部 tool_call/function_call 格式。Timem 不能执行这种格式。请继续使用 XML response protocol：需要动作时写 <progress> 和 <working_still_action><action_json><![CDATA[{...}]]></action_json></working_still_action>；完成时写 <status>ALL_FINISHED</status> 和 <final_answer>...</final_answer>。"
        }
        "final_answer_requires_status_finished" => {
            "检查到刚刚的输出格式有点问题：你给了 <final_answer>，但没有明确 <status>ALL_FINISHED</status>。如果当前用户请求已经完成，请同时提供 <status>ALL_FINISHED</status> 和 <final_answer>；如果仍需 runtime 继续工作，请不要写 <final_answer>，改写 <progress> 和 <working_still_action>。"
        }
        "final_answer_required_when_status_finished" => {
            "检查到刚刚的输出格式有点问题：你写了 <status>ALL_FINISHED</status>，但缺少 <final_answer>。如果当前用户请求已经完成，请同时提供二者；如果 final_answer 里需要展示 XML 标签或 XML 示例，请把整个 final_answer 文本包进 <![CDATA[ ... ]]>，避免示例标签被当作协议标签解析。如果仍需 runtime 继续工作，请不要写 ALL_FINISHED，并提供 <progress> 和需要的 <working_still_action>。"
        }
        "status_finished_must_not_include_next_actions" => {
            "检查到刚刚的输出格式有点问题：<status>ALL_FINISHED</status> 表示当前用户请求已完成，因此不能同时包含 <working_still_action>。如果还需要 runtime 执行动作，请保持 working，用 <progress> 和 <working_still_action> 继续；拿到 action result 后再写 ALL_FINISHED 和 <final_answer>。"
        }
        "next_actions_required_when_status_working" => {
            "检查到刚刚的输出格式有点问题：working 表示还需要 runtime 继续执行动作，因此必须提供 <progress> 和 <working_still_action>。如果当前用户请求已经完成，请改用 <status>ALL_FINISHED</status> 和 <final_answer>。"
        }
        _ => {
            "Use the XML response protocol. If work still needs runtime action, write <progress> and concrete <working_still_action>. If the current user request is complete, write <status>ALL_FINISHED</status> with <final_answer>; this does not close the Timem session."
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
  <action_json>{
    "action": "run_bash",
    "args": {} // missing cmd in the example on purpose
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
        assert!(env.final_answer.contains("\"args\": {}"));
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
  "intent": "并行拉起 3 个 sleep 15 后台任务",
  "actions": [
    { "action": "run_bash", "args": { "cmd": "sleep 15", "background": true } },
    { "action": "run_bash", "args": { "cmd": "sleep 15", "background": true } },
    { "action": "run_bash", "args": { "cmd": "sleep 15", "background": true } }
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
<progress>checking</progress>
<free_talk><![CDATA[
Example text only:
<working_still_action>
  <action_json>{"action":"run_bash","args":{}}</action_json>
</working_still_action>
]]></free_talk>
<working_still_action>
<action_json><![CDATA[{"action":"run_bash","intent":"Check cwd.","args":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(env.continue_work);
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].intent, "Check cwd.");
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
        assert!(env.thought.contains("<working_still_action>"));
    }

    #[test]
    fn free_talk_nested_xml_is_opaque_and_real_action_still_parses() {
        let env = parse_xml_envelope(
            r#"<response>
<progress>checking</progress>
<free_talk>
This is only a note:
<note priority="high"><working_still_action><action_json>{"action":"run_bash","args":{}}</action_json></working_still_action></note>
</free_talk>
<working_still_action>
<action_json><![CDATA[{"action":"run_bash","intent":"Check real action.","args":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].intent, "Check real action.");
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
        assert!(env.thought.contains(r#"<note priority="high">"#));
        assert!(env.thought.contains("<working_still_action>"));
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
<progress>checking</progress>
<free_talk>state</free_talk>
<working_still_action>
<action_json><![CDATA[{"action":"run_bash","intent":"Check files.","args":{"cmd":"pwd","timeout_ms":5000}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert!(env.continue_work);
        assert_eq!(env.report_job_progress, "checking");
        assert_eq!(env.thought, "state");
        assert_eq!(env.next_actions.len(), 1);
        assert_eq!(env.next_actions[0].action, "run_bash");
        assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
    }

    #[test]
    fn parses_single_group_object_from_action_json() {
        let env = parse_xml_envelope(
            r#"<response>
  <free_talk>并行启动 3 个 sleep 15 的后台任务。</free_talk>
  <working_still_action>
    <action_json><![CDATA[
{
  "order": "parallel",
  "intent": "并行拉起 3 个 sleep 15 后台任务",
  "actions": [
    { "action": "run_bash", "args": { "cmd": "sleep 15", "background": true } },
    { "action": "run_bash", "args": { "cmd": "sleep 15", "background": true } },
    { "action": "run_bash", "args": { "cmd": "sleep 15", "background": true } }
  ]
}
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
        assert_eq!(env.action_groups[0].actions.len(), 3);
        assert_eq!(env.next_actions.len(), 3);
        assert_eq!(env.next_actions[0].input_str("cmd"), "sleep 15");
        assert!(env.next_actions[0].input_bool("background"));
    }

    #[test]
    fn action_intent_and_args_can_contain_xml_like_text() {
        let env = parse_xml_envelope(
            r#"<response>
<progress>checking</progress>
<working_still_action>
<action_json><![CDATA[
{
  "order": "parallel",
  "intent": "Group intent mentions <response><status>ALL_FINISHED</status></response> only as text.",
  "actions": [
    {
      "action": "run_bash",
      "args": {
        "cmd": "printf group",
        "timeout_ms": 5000
      }
    },
    {
      "action": "run_bash",
      "intent": "Print XML-like payload <payload id=\"42\">ok</payload> as data.",
      "args": {
        "cmd": "printf '%s\n' '<working_still_action><action_json>{\"action\":\"run_bash\"}</action_json></working_still_action>'",
        "timeout_ms": 5000
      }
    }
  ]
}
]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
        assert_eq!(env.action_groups.len(), 1);
        assert_eq!(
            env.action_groups[0].actions[0].parent_intent.as_deref(),
            Some("Group intent mentions <response><status>ALL_FINISHED</status></response> only as text.")
        );
        assert_eq!(env.next_actions[0].input_str("cmd"), "printf group");
        assert!(env.next_actions[1]
            .intent
            .contains(r#"<payload id="42">ok</payload>"#));
        assert!(env.next_actions[1]
            .input_str("cmd")
            .contains("<working_still_action>"));
    }

    #[test]
    fn parses_context_compact() {
        let env = parse_xml_envelope(
            r#"<response>
<progress>compact</progress>
<context_compact>
<delta_ids>pd_a, pd_b</delta_ids>
<summary><![CDATA[keep state]]></summary>
</context_compact>
<working_still_action>
<action_json><![CDATA[{"action":"run_bash","intent":"Check files.","args":{"cmd":"pwd"}}]]></action_json>
</working_still_action>
</response>"#,
            &caps(),
        );

        assert!(env.repair_issue.is_none());
        assert_eq!(env.context_compacts.len(), 1);
        assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a", "pd_b"]);
        assert_eq!(env.context_compacts[0].summary, "keep state");
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

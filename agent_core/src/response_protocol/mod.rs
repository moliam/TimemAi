pub mod json_suite;
pub mod markdown_suite;
pub mod xml_suite;

use serde_json::Value;

use crate::capability::CapabilityRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseProtocolKind {
    Markdown,
    Json,
    Xml,
}

impl ResponseProtocolKind {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "" => Self::default(),
            "markdown" | "md" | "markdown_v1" => Self::Markdown,
            "json" | "json_v1" | "response_v1" => Self::Json,
            "xml" | "xml_v1" => Self::Xml,
            _ => Self::default(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Xml => "xml",
        }
    }

    pub fn lang_format(&self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Xml => "XML",
        }
    }

    pub fn suite(&self) -> &'static dyn ResponseProtocolSuite {
        match self {
            Self::Markdown => &markdown_suite::MarkdownSuiteV1,
            Self::Json => &json_suite::JsonSuiteV1,
            Self::Xml => &xml_suite::XmlSuiteV1,
        }
    }
}

impl Default for ResponseProtocolKind {
    fn default() -> Self {
        Self::Xml
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAction {
    pub action: String,
    pub raw_input: Value,
}
impl ParsedAction {
    pub fn audit_input(&self) -> Value {
        let mut input = self.raw_input.clone();
        if self.action == "self_tool" {
            if let Some(object) = input.as_object_mut() {
                if let Some(key) = object.get("key").and_then(Value::as_str) {
                    if crate::self_tool::is_sensitive_env_key(key)
                        || crate::self_tool::is_memory_path_env_key(key)
                    {
                        object.insert("value".to_string(), serde_json::json!("<redacted>"));
                    }
                }
            }
        }
        input
    }

    pub fn input_str(&self, key: &str) -> String {
        self.raw_input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    }

    pub fn input_raw_str(&self, key: &str) -> String {
        self.raw_input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    pub fn input_lower(&self, key: &str) -> String {
        self.input_str(key).to_lowercase()
    }

    pub fn input_u64(&self, key: &str) -> Option<u64> {
        self.raw_input.get(key).and_then(json_u64)
    }

    pub fn input_i64(&self, key: &str) -> Option<i64> {
        self.raw_input.get(key).and_then(json_i64)
    }

    pub fn input_bool(&self, key: &str) -> bool {
        self.raw_input
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn input_list(&self, key: &str) -> Vec<String> {
        self.raw_input
            .get(key)
            .map(json_string_list)
            .unwrap_or_default()
    }

    pub fn input_params(&self) -> Vec<String> {
        self.raw_input
            .get("params")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(json_sql_param_to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub fn timeout_ms(&self, default_ms: u64) -> u64 {
        self.input_u64("timeout_ms")
            .or_else(|| {
                self.input_u64("timeout_sec")
                    .map(|seconds| seconds.saturating_mul(1000))
            })
            .unwrap_or(default_ms)
    }

    pub fn timeout_ms_i64(&self, default_ms: i64) -> i64 {
        self.input_i64("timeout_ms")
            .or_else(|| {
                self.input_i64("timeout_sec")
                    .map(|seconds| seconds.saturating_mul(1000))
            })
            .unwrap_or(default_ms)
    }

    pub fn shell_timeout_ms(&self) -> u64 {
        self.timeout_ms(5000).max(1)
    }

    pub fn status_timeout_ms(&self) -> u64 {
        self.timeout_ms(0).min(15000)
    }

    pub fn background(&self) -> bool {
        self.input_bool("background")
            || self
                .raw_input
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("background"))
    }
}

pub(crate) fn parse_action_object(
    value: &Value,
    label: &str,
    capabilities: &CapabilityRegistry,
) -> Result<ParsedAction, String> {
    let Some(object) = value.as_object() else {
        return Err(format!("{label}.action_missing"));
    };
    if object.contains_key("order") || object.contains_key("actions") {
        return Err(format!("{label}.old_group_object_not_supported"));
    }
    if object.len() != 1 {
        return Err(format!("{label}.action_missing"));
    }
    let (name, input) = object.iter().next().expect("checked len");
    if !input.is_object() {
        return Err(format!("{label}.args_must_be_object"));
    }
    validate_parsed_action(name.to_string(), input.clone(), label, capabilities)
}

pub(crate) fn is_tool_action_object(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 1 {
        return false;
    }
    let (name, input) = object.iter().next().expect("checked len");
    !matches!(
        name.as_str(),
        "order"
            | "actions"
            | "status"
            | "final_answer"
            | "free_talk"
            | "working_still_action"
            | "next_actions"
            | "context_compact"
            | "context_compacts"
            | "memory_candidates"
    ) && input.is_object()
}

pub(crate) fn parse_action_workflow_value(
    value: &Value,
    label: &str,
    capabilities: &CapabilityRegistry,
) -> Result<Vec<ParsedActionGroup>, String> {
    if value.is_object() {
        return Ok(vec![ParsedActionGroup {
            order: ActionGroupOrder::Sequential,
            actions: vec![parse_action_object(value, label, capabilities)?],
        }]);
    }

    let Some(items) = value.as_array() else {
        return Err("actions_section_must_be_action_or_array".to_string());
    };
    if items.is_empty() {
        return Err(format!("{label}.actions_required"));
    }

    if items.iter().all(is_tool_action_object) {
        return Ok(vec![ParsedActionGroup {
            order: ActionGroupOrder::Parallel,
            actions: parse_action_array(items, label, capabilities)?,
        }]);
    }

    let mut groups = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let item_label = format!("{label}[{idx}]");
        if item.is_object() {
            groups.push(ParsedActionGroup {
                order: ActionGroupOrder::Sequential,
                actions: vec![parse_action_object(item, &item_label, capabilities)?],
            });
        } else if let Some(inner) = item.as_array() {
            if inner.is_empty() {
                return Err(format!("{item_label}.actions_required"));
            }
            groups.push(ParsedActionGroup {
                order: ActionGroupOrder::Parallel,
                actions: parse_action_array(inner, &item_label, capabilities)?,
            });
        } else {
            return Err(format!("{item_label}.action_missing"));
        }
    }
    Ok(groups)
}

fn parse_action_array(
    items: &[Value],
    label: &str,
    capabilities: &CapabilityRegistry,
) -> Result<Vec<ParsedAction>, String> {
    let mut actions = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        actions.push(parse_action_object(
            item,
            &format!("{label}[{idx}]"),
            capabilities,
        )?);
    }
    Ok(actions)
}

fn validate_parsed_action(
    name: String,
    input: Value,
    label: &str,
    capabilities: &CapabilityRegistry,
) -> Result<ParsedAction, String> {
    if !capabilities.contains_tool(&name) {
        return Err(format!("unsupported_action:{name}"));
    }
    if let Err(issue) = capabilities.validate_action_input(&name, &input) {
        return Err(format!("{label}.{issue}"));
    }
    Ok(ParsedAction {
        action: name,
        raw_input: input,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedActionGroup {
    pub order: ActionGroupOrder,
    pub actions: Vec<ParsedAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionGroupOrder {
    Sequential,
    Parallel,
}

impl ActionGroupOrder {
    pub fn from_name(name: &str) -> Self {
        if name.trim().eq_ignore_ascii_case("parallel") {
            Self::Parallel
        } else {
            Self::Sequential
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Parallel => "parallel",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]

pub struct ParsedEnvelope {
    pub final_answer: String,
    pub continue_work: bool,
    pub thought: String,
    pub thought_keep_in_context: bool,
    pub next_actions: Vec<ParsedAction>,
    pub action_groups: Vec<ParsedActionGroup>,
    pub context_compacts: Vec<ParsedContextCompact>,
    pub memory_candidates: Vec<String>,
    pub runtime_note: Option<String>,
    pub repair_issue: Option<String>,
}

impl ParsedEnvelope {
    pub fn final_text(&self) -> String {
        self.final_answer.trim().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedContextCompact {
    pub discard_delta_ids: Vec<String>,
    pub offload_delta_ids: Vec<String>,
    pub delta_ids: Vec<String>,
    pub slice_ids: Vec<String>,
    pub summary: String,
}

/// Trait for response protocol implementations.
pub trait ResponseProtocolSuite {
    fn name(&self) -> &str;
    fn lang_format(&self) -> &str;
    fn protocol_schema(&self) -> &str;
    fn protocol_examples(&self) -> &str;
    fn response_schema_summary(&self) -> &str;
    fn protocol_prompt_section(&self) -> String {
        format!("{}\n\n{}", self.protocol_schema(), self.protocol_examples())
    }
    fn parse(&self, raw: &str, capabilities: &CapabilityRegistry) -> ParsedEnvelope;
    fn repair_instruction(&self, issue: &str) -> &str;
    fn repair_instruction_for_response(&self, issue: &str, _raw_response: &str) -> String {
        self.repair_instruction(issue).to_string()
    }
    fn repair_reason(&self, issue: &str) -> &str;
    fn focused_repair_text(&self, issue: &str, text: &str) -> String;
    fn can_show_plain_text_after_repair_failure(&self, content: &str) -> bool;
}

pub fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
}

pub fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|raw| u64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|raw| raw.trim().parse().ok()))
}

pub fn json_string_array(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .or_else(|| value.as_i64().map(|raw| raw.to_string()))
                .or_else(|| value.as_u64().map(|raw| raw.to_string()))
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

pub fn json_string_list(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        return json_string_array(items);
    }
    value
        .as_str()
        .map(|text| {
            text.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| item.trim_matches(['"', '\'']).to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub fn json_sql_param_to_string(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(num) = value.as_i64() {
        return Some(num.to_string());
    }
    if let Some(num) = value.as_u64() {
        return Some(num.to_string());
    }
    if let Some(num) = value.as_f64() {
        return Some(num.to_string());
    }
    value.as_bool().map(|flag| flag.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn caps() -> CapabilityRegistry {
        CapabilityRegistry::builtin()
    }

    fn parse_json(raw: &str) -> ParsedEnvelope {
        json_suite::parse_envelope(raw, &caps())
    }

    fn parse_markdown(raw: &str) -> ParsedEnvelope {
        markdown_suite::parse_markdown_envelope(raw, &caps())
    }

    fn parse_xml(raw: &str) -> ParsedEnvelope {
        xml_suite::parse_xml_envelope(raw, &caps())
    }

    fn assert_protocols_equivalent(json_raw: &str, markdown_raw: &str, xml_raw: &str) {
        let json_env = parse_json(json_raw);
        let markdown_env = parse_markdown(markdown_raw);
        let xml_env = parse_xml(xml_raw);
        assert_eq!(
            markdown_env.repair_issue, None,
            "markdown env: {markdown_env:?}"
        );
        assert_eq!(json_env.repair_issue, None, "json env: {json_env:?}");
        assert_eq!(xml_env.repair_issue, None, "xml env: {xml_env:?}");
        assert_eq!(markdown_env.continue_work, json_env.continue_work);
        assert_eq!(xml_env.continue_work, json_env.continue_work);
        assert_eq!(markdown_env.final_answer, json_env.final_answer);
        assert_eq!(xml_env.final_answer, json_env.final_answer);
        assert_eq!(markdown_env.thought, json_env.thought);
        assert_eq!(xml_env.thought, json_env.thought);
        assert_eq!(
            markdown_env.thought_keep_in_context,
            json_env.thought_keep_in_context
        );
        assert_eq!(
            xml_env.thought_keep_in_context,
            json_env.thought_keep_in_context
        );
        assert_eq!(markdown_env.next_actions, json_env.next_actions);
        assert_eq!(xml_env.next_actions, json_env.next_actions);
        assert_eq!(markdown_env.action_groups, json_env.action_groups);
        assert_eq!(xml_env.action_groups, json_env.action_groups);
        assert_eq!(markdown_env.context_compacts, json_env.context_compacts);
        assert_eq!(xml_env.context_compacts, json_env.context_compacts);
    }

    #[test]
    fn json_markdown_xml_protocols_parse_same_final_answer() {
        assert_protocols_equivalent(
            r#"{"status":"ALL_FINISHED","final_answer":"done"}"#,
            "## Status\nfinished\n\n## Final_Answer\ndone",
            "<response><final_answer>done</final_answer></response>",
        );
    }

    #[test]
    fn json_markdown_xml_protocols_treat_protocol_language_inside_text_as_text() {
        assert_protocols_equivalent(
            r#"{"status":"ALL_FINISHED","final_answer":"Example only:\n<working_still_action><action_json>{\"run_bash\":{}}</action_json></working_still_action>\n{\"working_still_action\":{\"run_bash\":{}}}\n## Working_Still_Action\n```action\n{\"run_bash\":{}}\n```"}"#,
            "## Status\nfinished\n\n## Final_Answer\nExample only:\n<working_still_action><action_json>{\"run_bash\":{}}</action_json></working_still_action>\n{\"working_still_action\":{\"run_bash\":{}}}\n## Working_Still_Action\n```action\n{\"run_bash\":{}}\n```",
            r#"<response><final_answer><![CDATA[Example only:
<working_still_action><action_json>{"run_bash":{}}</action_json></working_still_action>
{"working_still_action":{"run_bash":{}}}
## Working_Still_Action
```action
{"run_bash":{}}
```]]></final_answer></response>"#,
        );
    }

    #[test]
    fn json_markdown_xml_protocols_parse_same_working_actions() {
        assert_protocols_equivalent(
            r#"{"free_talk":"state","working_still_action":[{"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]}"#,
            "## Free_talk\nstate\n\n## Working_Still_Action\n```action\n[{\"memmgr\":{\"type\":\"durable\",\"op\":\"sql\",\"sql\":\"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5\",\"params\":[\"%project%\"],\"limit\":5}},{\"run_bash\":{\"cmd\":\"pwd\",\"timeout_ms\":5000}}]\n```",
            r#"<response><free_talk>state</free_talk><working_still_action><action_json><![CDATA[[{"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json></working_still_action></response>"#,
        );
    }

    #[test]
    fn json_markdown_xml_protocols_parse_same_bare_action_array() {
        assert_protocols_equivalent(
            r#"{"free_talk":"checking","working_still_action":[{"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]}"#,
            "## Free_talk\nchecking\n\n## Working_Still_Action\n[{\"memmgr\":{\"type\":\"durable\",\"op\":\"sql\",\"sql\":\"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5\",\"params\":[\"%project%\"],\"limit\":5}},{\"run_bash\":{\"cmd\":\"pwd\",\"timeout_ms\":5000}}]",
            r#"<response><free_talk>checking</free_talk><working_still_action><action_json><![CDATA[[{"memmgr":{"type":"durable","op":"sql","sql":"SELECT id, version, content FROM memories WHERE content LIKE ? LIMIT 5","params":["%project%"],"limit":5}},{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json></working_still_action></response>"#,
        );
    }

    #[test]
    fn json_markdown_xml_protocols_parse_same_mixed_action_group_array() {
        assert_protocols_equivalent(
            r#"{"free_talk":"checking","working_still_action":[[{"run_bash":{"cmd":"printf a","timeout_ms":5000}},{"run_bash":{"cmd":"printf b","timeout_ms":5000}}],{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]}"#,
            "## Free_talk\nchecking\n\n## Working_Still_Action\n[[{\"run_bash\":{\"cmd\":\"printf a\",\"timeout_ms\":5000}},{\"run_bash\":{\"cmd\":\"printf b\",\"timeout_ms\":5000}}],{\"run_bash\":{\"cmd\":\"pwd\",\"timeout_ms\":5000}}]",
            r#"<response><free_talk>checking</free_talk><working_still_action><action_json><![CDATA[[[{"run_bash":{"cmd":"printf a","timeout_ms":5000}},{"run_bash":{"cmd":"printf b","timeout_ms":5000}}],{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json></working_still_action></response>"#,
        );
    }

    #[test]
    fn json_markdown_xml_protocols_parse_complex_actions_with_protocol_like_string_args() {
        let action_payload = r#"[[{"run_bash":{"cmd":"printf '%s\n' '<working_still_action>{\"action\":\"run_bash\"}</working_still_action>' && printf '%s\n' '## Final_Answer not a section'","timeout_ms":5000}},{"memmgr":{"type":"raw_chat","op":"sql","sql":"SELECT content FROM chat_messages WHERE content LIKE ? LIMIT 5","params":["%<response><status>ALL_FINISHED</status></response> {\"working_still_action\":[]} ## Working_Still_Action%"],"limit":5}}],{"run_bash":{"cmd":"printf done","timeout_ms":5000}}]"#;
        let json_raw = format!(
            r#"{{"free_talk":"Plan text includes {{\"action\":\"run_bash\"}} only as text. Note text includes <working_still_action>fake</working_still_action>.","working_still_action":{action_payload}}}"#
        );
        let markdown_raw = format!(
            "## Free_talk\nPlan text includes {{\"action\":\"run_bash\"}} only as text. Note text includes <working_still_action>fake</working_still_action>.\n\n## Working_Still_Action\n{action_payload}"
        );
        let xml_raw = format!(
            r#"<response><free_talk><![CDATA[Plan text includes {{"action":"run_bash"}} only as text. Note text includes <working_still_action>fake</working_still_action>.]]></free_talk><working_still_action><action_json><![CDATA[{action_payload}]]></action_json></working_still_action></response>"#
        );

        assert_protocols_equivalent(&json_raw, &markdown_raw, &xml_raw);

        let env = parse_json(&json_raw);
        assert_eq!(env.next_actions.len(), 3);
        assert_eq!(env.action_groups.len(), 2);
        assert_eq!(env.action_groups[0].order, ActionGroupOrder::Parallel);
        assert_eq!(
            env.next_actions[0].input_str("cmd"),
            "printf '%s\n' '<working_still_action>{\"action\":\"run_bash\"}</working_still_action>' && printf '%s\n' '## Final_Answer not a section'"
        );
        assert_eq!(
            env.next_actions[1].input_params(),
            vec![
                "%<response><status>ALL_FINISHED</status></response> {\"working_still_action\":[]} ## Working_Still_Action%".to_string()
            ]
        );
        assert!(env.context_compacts.is_empty());
    }

    #[test]
    fn json_markdown_xml_protocols_parse_same_context_compact() {
        assert_protocols_equivalent(
            r#"{"free_talk":"compact","context_compact":{"discard":["pd_a"],"offload":["pd_b"],"summary":"keep state"}}"#,
            "## Free_talk\ncompact\n\n## Context Compact\ndiscard: pd_a\noffload: pd_b\nsummary:\nkeep state",
            r#"<response><free_talk>compact</free_talk><context_compact><discard>pd_a</discard><offload>pd_b</offload><summary>keep state</summary></context_compact></response>"#,
        );
    }

    #[test]
    fn json_markdown_xml_protocols_repair_same_finished_with_actions() {
        let json_env = parse_json(
            r#"{"status":"ALL_FINISHED","final_answer":"done","working_still_action":{"run_bash":{"cmd":"test -s output.txt","timeout_ms":5000}}}"#,
        );
        let markdown_env = parse_markdown(
            "## Status\nfinished\n\n## Working_Still_Action\n```action\n{\"run_bash\":{\"cmd\":\"test -s output.txt\",\"timeout_ms\":5000}}\n```\n\n## Final_Answer\ndone",
        );
        let xml_env = parse_xml(
            r#"<response><final_answer>done</final_answer><working_still_action><action_json><![CDATA[[{"run_bash":{"cmd":"test -s output.txt","timeout_ms":5000}}]]]></action_json></working_still_action></response>"#,
        );
        assert_eq!(
            json_env.repair_issue.as_deref(),
            Some("status_finished_must_not_include_next_actions")
        );
        assert_eq!(markdown_env.repair_issue, json_env.repair_issue);
        assert_eq!(xml_env.repair_issue, json_env.repair_issue);
    }

    #[test]
    fn json_markdown_xml_protocols_repair_same_final_answer_without_finished_status() {
        let json_env = parse_json(r#"{"final_answer":"done"}"#);
        let markdown_env = parse_markdown("## Final_Answer\ndone");
        let xml_env = parse_xml("<response><final_answer>done</final_answer></response>");
        assert_eq!(
            json_env.repair_issue.as_deref(),
            Some("final_answer_requires_status_finished")
        );
        assert_eq!(markdown_env.repair_issue, json_env.repair_issue);
        assert_eq!(xml_env.repair_issue, None);
        assert!(!xml_env.continue_work);
        assert_eq!(xml_env.final_answer, "done");
    }

    #[test]
    fn json_markdown_xml_protocols_repair_same_working_without_actions() {
        let json_env = parse_json(r#"{"status":"working"}"#);
        let markdown_env = parse_markdown("## Status\nworking\n\n## Free_talk\nchecking");
        let xml_env = parse_xml("<response><free_talk>checking</free_talk></response>");
        assert_eq!(
            json_env.repair_issue.as_deref(),
            Some("next_actions_required_when_status_working")
        );
        assert_eq!(markdown_env.repair_issue, json_env.repair_issue);
        assert_eq!(xml_env.repair_issue, json_env.repair_issue);
    }

    #[test]
    fn json_markdown_xml_protocols_share_action_input_shape() {
        let action = json!({"run_bash": {"cmd": "pwd", "timeout_ms": 5000}
        });
        let json_env = parse_json(&json!({"working_still_action":[action.clone()]}).to_string());
        let markdown_env = parse_markdown(&format!(
            "## Working_Still_Action\n```action\n{}\n```",
            action
        ));
        let xml_env = parse_xml(&format!(
            "<response><working_still_action><action_json><![CDATA[[{}]]]></action_json></working_still_action></response>",
            action
        ));
        assert_eq!(json_env.repair_issue, None);
        assert_eq!(markdown_env.repair_issue, None);
        assert_eq!(xml_env.repair_issue, None);
        assert_eq!(json_env.next_actions, markdown_env.next_actions);
        assert_eq!(json_env.next_actions, xml_env.next_actions);
    }
}

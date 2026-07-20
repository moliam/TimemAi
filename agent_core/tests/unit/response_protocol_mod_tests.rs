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
fn toolgen_retrospect_has_equivalent_final_response_semantics_in_all_protocols() {
    let json = parse_json(
        r#"{"status":"ALL_FINISHED","toolgen_retrospect":"Created semantic-tool after runtime returned ready.","final_answer":"review done"}"#,
    );
    let markdown = parse_markdown(
        "## Status\nfinished\n\n## ToolGen_Retrospect\nCreated semantic-tool after runtime returned ready.\n\n## Final_Answer\nreview done",
    );
    let xml = parse_xml(
        "<response><toolgen_retrospect>Created semantic-tool after runtime returned ready.</toolgen_retrospect><final_answer>review done</final_answer></response>",
    );
    for envelope in [&json, &markdown, &xml] {
        assert!(envelope.repair_issue.is_none());
        assert!(!envelope.continue_work);
        assert_eq!(envelope.final_answer, "review done");
        assert_eq!(
            envelope.toolgen_retrospect,
            "Created semantic-tool after runtime returned ready."
        );
    }
}

#[test]
fn toolgen_retrospect_is_rejected_from_working_responses_in_all_protocols() {
    let json = parse_json(
        r#"{"toolgen_retrospect":"premature","working_still_action":{"run_bash":{"cmd":"pwd"}}}"#,
    );
    let markdown = parse_markdown(
        "## ToolGen_Retrospect\npremature\n\n## Working_Still_Action\n```action\n{\"run_bash\":{\"cmd\":\"pwd\"}}\n```",
    );
    let xml = parse_xml(
        "<response><toolgen_retrospect>premature</toolgen_retrospect><working_still_action><action_json><![CDATA[{\"run_bash\":{\"cmd\":\"pwd\"}}]]></action_json></working_still_action></response>",
    );
    for (protocol, envelope) in [("json", &json), ("markdown", &markdown), ("xml", &xml)] {
        assert_eq!(
            envelope.repair_issue.as_deref(),
            Some("toolgen_retrospect_requires_final_answer"),
            "{protocol}: {envelope:?}"
        );
    }
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

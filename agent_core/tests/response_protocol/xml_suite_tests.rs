
use super::*;

fn caps() -> CapabilityRegistry {
    CapabilityRegistry::builtin()
}

fn extract_response_examples(text: &str) -> Vec<String> {
    let mut examples = Vec::new();
    let mut cursor = 0usize;
    while let Some(start_rel) = text[cursor..].find("\n<response>\n") {
        let start = cursor + start_rel + 1;
        let search_from = start + "<response>".len();
        let Some(end_rel) = text[search_from..].find("</response>") else {
            break;
        };
        let end = search_from + end_rel + "</response>".len();
        examples.push(text[start..end].to_string());
        cursor = end;
    }
    examples
}

#[test]
fn documented_xml_response_examples_parse_with_runtime_parser() {
    let examples = extract_response_examples(XML_RESPONSE_PROTOCOL_SECTION);
    assert!(
        examples.len() >= 4,
        "expected protocol doc to contain concrete XML response examples"
    );

    for (idx, example) in examples.iter().enumerate() {
        let env = parse_xml_envelope(example, &caps());
        assert!(
            env.repair_issue.is_none(),
            "documented XML example #{idx} did not parse: {:?}\n{}",
            env.repair_issue,
            example
        );
        assert!(
            !env.final_answer.trim().is_empty()
                || !env.next_actions.is_empty()
                || !env.context_compacts.is_empty(),
            "documented XML example #{idx} produced no runtime-visible result:\n{}",
            example
        );
    }
}

#[test]
fn parses_final_answer() {
    let env = parse_xml_envelope(
        "<response><final_answer>done</final_answer></response>",
        &caps(),
    );
    assert!(env.repair_issue.is_none());
    assert!(!env.continue_work);
    assert_eq!(env.final_answer, "done");
}

#[test]
fn root_repair_moves_free_talk_inside_response_with_matching_action_branch() {
    let malformed = r#"<free_talk>searching</free_talk>
<response><working_still_action>...</working_still_action></response>"#;
    let instruction = xml_repair_instruction_for_response("xml_content_before_response", malformed);

    assert!(instruction.contains("placed content before the <response> root"));
    assert!(instruction.contains(
            "The response must be in format '<response><free_talk>...</free_talk><working_still_action>...</working_still_action></response>'"
        ));
    assert!(instruction.contains("output nothing before <response> or after </response>"));
}

#[test]
fn root_repair_selects_the_branch_present_in_the_malformed_response() {
    let final_instruction = xml_repair_instruction_for_response(
        "xml_content_before_response",
        "preface<response><final_answer>done</final_answer></response>",
    );
    assert!(final_instruction.contains("<response><final_answer>...</final_answer></response>"));

    let compact_instruction = xml_repair_instruction_for_response(
        "xml_content_after_response",
        "<response><context_compact><summary>x</summary></context_compact></response>tail",
    );
    assert!(
        compact_instruction.contains("<response><context_compact>...</context_compact></response>")
    );
    assert!(compact_instruction.contains("placed content after the </response> root"));
}

#[test]
fn malformed_raw_responses_map_to_distinct_issue_and_guidance() {
    let cases = [
            (
                "<free_talk>x</free_talk><response><final_answer>done</final_answer></response>",
                "xml_content_before_response",
                "placed content before the <response> root",
            ),
            (
                "<response><final_answer>done</final_answer></response>tail",
                "xml_content_after_response",
                "placed content after the </response> root",
            ),
            (
                "<free_talk>missing root</free_talk>",
                "xml_response_root_missing",
                "did not contain the required <response> root",
            ),
            (
                "<response><final_answer>done</final_answer>",
                "xml_response_root_unclosed",
                "did not form one complete <response>...</response> root",
            ),
            (
                "<response/>",
                "xml_response_root_self_closing",
                "did not form one complete <response>...</response> root",
            ),
            (
                "<response>stray<final_answer>done</final_answer></response>",
                "xml_unexpected_content_inside_response",
                "unknown top-level tag outside a supported field",
            ),
            (
                "<response><free_talk>a</free_talk><free_talk>b</free_talk><final_answer>done</final_answer></response>",
                "xml_duplicate_free_talk",
                "more than one <free_talk>",
            ),
            (
                "<response><free_talk>broken<final_answer>done</final_answer></response>",
                "xml_unclosed_tag:free_talk",
                "field tag is not closed",
            ),
        ];

    for (raw, expected_issue, expected_guidance) in cases {
        let parsed = parse_xml_envelope(raw, &caps());
        assert_eq!(
            parsed.repair_issue.as_deref(),
            Some(expected_issue),
            "raw={raw}"
        );
        let instruction = xml_repair_instruction_for_response(expected_issue, raw);
        assert!(
            instruction.contains(expected_guidance),
            "issue={expected_issue}, instruction={instruction}"
        );
    }
}

#[test]
fn malformed_response_corpus_maps_raw_output_to_precise_repair_reason() {
    struct Case {
        name: &'static str,
        raw: &'static str,
        issue: &'static str,
        guidance: &'static str,
    }

    let cases = [
            Case {
                name: "empty output",
                raw: "   ",
                issue: "empty_response",
                guidance: "没有生成可解析的内容",
            },
            Case {
                name: "missing response root",
                raw: "<final_answer>done</final_answer>",
                issue: "xml_response_root_missing",
                guidance: "did not contain the required <response> root",
            },
            Case {
                name: "xml declaration before root",
                raw: "<?xml version=\"1.0\"?><response><final_answer>done</final_answer></response>",
                issue: "xml_content_before_response",
                guidance: "content before the <response> root",
            },
            Case {
                name: "free talk before root",
                raw: "<free_talk>thinking</free_talk><response><final_answer>done</final_answer></response>",
                issue: "xml_content_before_response",
                guidance: "Move every tag, including <free_talk>, inside <response>",
            },
            Case {
                name: "trailing prose after root",
                raw: "<response><final_answer>done</final_answer></response>extra",
                issue: "xml_content_after_response",
                guidance: "content after the </response> root",
            },
            Case {
                name: "second response root",
                raw: "<response><final_answer>one</final_answer></response><response><final_answer>two</final_answer></response>",
                issue: "xml_content_after_response",
                guidance: "Output nothing before <response> or after </response>",
            },
            Case {
                name: "unclosed response root",
                raw: "<response><final_answer>done</final_answer>",
                issue: "xml_response_root_unclosed",
                guidance: "one complete <response>...</response> root",
            },
            Case {
                name: "self closing response root",
                raw: "<response/>",
                issue: "xml_response_root_self_closing",
                guidance: "one complete <response>...</response> root",
            },
            Case {
                name: "empty response body",
                raw: "<response></response>",
                issue: "next_actions_required_when_status_working",
                guidance: "必须提供 <working_still_action>",
            },
            Case {
                name: "unknown top level tag",
                raw: "<response><progress>working</progress><final_answer>done</final_answer></response>",
                issue: "xml_unexpected_content_inside_response",
                guidance: "unknown top-level tag",
            },
            Case {
                name: "raw text inside response",
                raw: "<response>thinking<final_answer>done</final_answer></response>",
                issue: "xml_unexpected_content_inside_response",
                guidance: "Put text inside <free_talk> or <final_answer>",
            },
            Case {
                name: "duplicate free talk",
                raw: "<response><free_talk>a</free_talk><free_talk>b</free_talk><final_answer>done</final_answer></response>",
                issue: "xml_duplicate_free_talk",
                guidance: "Merge them into one optional <free_talk>",
            },
            Case {
                name: "free talk after state branch",
                raw: "<response><final_answer>done</final_answer><free_talk>late</free_talk></response>",
                issue: "xml_tags_out_of_order",
                guidance: "tags are out of order",
            },
            Case {
                name: "unclosed free talk",
                raw: "<response><free_talk>broken<final_answer>done</final_answer></response>",
                issue: "xml_unclosed_tag:free_talk",
                guidance: "field tag is not closed",
            },
            Case {
                name: "working and final branches together",
                raw: "<response><working_still_action><action_json><![CDATA[[{\"run_bash\":{\"cmd\":\"pwd\"}}]]]></action_json></working_still_action><final_answer>done</final_answer></response>",
                issue: "status_finished_must_not_include_next_actions",
                guidance: "不能同时包含 <working_still_action>",
            },
            Case {
                name: "compact and final branches together",
                raw: "<response><context_compact><discard>pd_1</discard><summary>state</summary></context_compact><final_answer>done</final_answer></response>",
                issue: "state_branch_must_choose_one",
                guidance: "selected more than one state branch",
            },
            Case {
                name: "unsupported status tag",
                raw: "<response><status>ALL_FINISHED</status></response>",
                issue: "status_tag_not_supported",
                guidance: "不使用 <status>",
            },
            Case {
                name: "action payload is not array",
                raw: "<response><working_still_action><action_json><![CDATA[{\"run_bash\":{\"cmd\":\"pwd\"}}]]></action_json></working_still_action></response>",
                issue: "actions[0].array_required",
                guidance: "必须是 JSON array",
            },
            Case {
                name: "invalid action json",
                raw: "<response><working_still_action><action_json><![CDATA[[{\"run_bash\":{\"cmd\":\"pwd\",}}]]]></action_json></working_still_action></response>",
                issue: "actions[0].invalid_json",
                guidance: "not valid JSON",
            },
            Case {
                name: "removed action group shape",
                raw: "<response><working_still_action><action_json><![CDATA[{\"order\":\"parallel\",\"actions\":[]}]]></action_json></working_still_action></response>",
                issue: "actions[0].old_group_object_not_supported",
                guidance: "removed {\"order\":...,\"actions\":[...]} group shape",
            },
            Case {
                name: "empty workflow",
                raw: "<response><working_still_action><action_json><![CDATA[[]]]></action_json></working_still_action></response>",
                issue: "actions[0].actions_required",
                guidance: "empty or incomplete stage",
            },
            Case {
                name: "empty parallel stage",
                raw: "<response><working_still_action><action_json><![CDATA[[[]]]]></action_json></working_still_action></response>",
                issue: "actions[0][0].actions_required",
                guidance: "empty or incomplete stage",
            },
            Case {
                name: "action has no tool key",
                raw: "<response><working_still_action><action_json><![CDATA[[{}]]]></action_json></working_still_action></response>",
                issue: "actions[0][0].action_missing",
                guidance: "missing its tool-name key",
            },
            Case {
                name: "action has multiple tool keys",
                raw: "<response><working_still_action><action_json><![CDATA[[{\"run_bash\":{},\"memmgr\":{}}]]]></action_json></working_still_action></response>",
                issue: "actions[0][0].action_missing",
                guidance: "missing its tool-name key",
            },
            Case {
                name: "tool arguments are scalar",
                raw: "<response><working_still_action><action_json><![CDATA[[{\"run_bash\":\"pwd\"}]]]></action_json></working_still_action></response>",
                issue: "actions[0][0].args_must_be_object",
                guidance: "not a JSON object",
            },
            Case {
                name: "workflow entry is scalar",
                raw: "<response><working_still_action><action_json><![CDATA[[42]]]></action_json></working_still_action></response>",
                issue: "actions[0][0].action_missing",
                guidance: "missing its tool-name key",
            },
            Case {
                name: "unknown tool",
                raw: "<response><working_still_action><action_json><![CDATA[[{\"not_a_tool\":{}}]]]></action_json></working_still_action></response>",
                issue: "unsupported_action:not_a_tool",
                guidance: "not in the available capability catalog",
            },
            Case {
                name: "missing required run bash command",
                raw: "<response><working_still_action><action_json><![CDATA[[{\"run_bash\":{}}]]]></action_json></working_still_action></response>",
                issue: "actions[0][0].input.any_required:cmd|loop_cmd",
                guidance: "do not satisfy the capability specification",
            },
            Case {
                name: "compact missing ids",
                raw: "<response><context_compact><summary>state</summary></context_compact></response>",
                issue: "context_compact[0].ids_required",
                guidance: "at least one non-empty <discard> or <offload>",
            },
            Case {
                name: "compact missing summary",
                raw: "<response><context_compact><discard>pd_1</discard></context_compact></response>",
                issue: "context_compact[0].summary_required",
                guidance: "missing a non-empty <summary>",
            },
        ];

    assert!(cases.len() >= 30, "keep the malformed corpus substantial");
    for case in cases {
        let parsed = parse_xml_envelope(case.raw, &caps());
        assert_eq!(
            parsed.repair_issue.as_deref(),
            Some(case.issue),
            "case={} raw={}",
            case.name,
            case.raw
        );
        let instruction = xml_repair_instruction_for_response(case.issue, case.raw);
        assert!(
            instruction.contains(case.guidance),
            "case={} issue={} guidance={} instruction={}",
            case.name,
            case.issue,
            case.guidance,
            instruction
        );
    }
}

#[test]
fn non_root_repair_keeps_issue_specific_static_instruction() {
    assert_eq!(
        xml_repair_instruction_for_response(
            "state_branch_must_choose_one",
            "<response></response>"
        ),
        xml_repair_instruction("state_branch_must_choose_one")
    );
}

#[test]
fn common_action_repair_issues_have_specific_correction_guidance() {
    let cases = [
        ("actions[0].invalid_json", "not valid JSON"),
        ("actions[0].action_missing", "missing its tool-name key"),
        ("actions[0].args_must_be_object", "not a JSON object"),
        (
            "actions[0].old_group_object_not_supported",
            "removed {\"order\":...,\"actions\":[...]} group shape",
        ),
        ("actions[0].actions_required", "empty or incomplete stage"),
        (
            "unsupported_action:ghost",
            "not in the available capability catalog",
        ),
        (
            "actions[0].input.cmd_required",
            "do not satisfy the capability specification",
        ),
        ("context_compact[0].ids_required", "at least one non-empty"),
        (
            "context_compact[0].summary_required",
            "missing a non-empty <summary>",
        ),
    ];

    for (issue, expected) in cases {
        let instruction = xml_repair_instruction_for_response(issue, "<response/>");
        assert!(
            instruction.contains(expected),
            "issue={issue}, instruction={instruction}"
        );
    }
}

#[test]
fn parses_final_answer_cdata_with_xml_examples() {
    let env = parse_xml_envelope(
        r#"<response>
  <final_answer><![CDATA[
Example response delta:

<response>
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
fn final_answer_can_contain_multiple_adjacent_response_examples_as_text() {
    let env = parse_xml_envelope(
        r#"<response>
  <final_answer><![CDATA[
First example:
<response><final_answer>one</final_answer></response>
<response><final_answer>two</final_answer></response>
  ]]></final_answer>
</response>"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert!(!env.continue_work);
    assert!(env
        .final_answer
        .contains("<response><final_answer>one</final_answer></response>"));
    assert!(env
        .final_answer
        .contains("<response><final_answer>two</final_answer></response>"));
}

#[test]
fn final_answer_raw_unbalanced_xml_is_opaque_text() {
    let env = parse_xml_envelope(
        r#"<response>
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
<action_json><![CDATA[[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json>
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
<action_json><![CDATA[[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json>
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
<action_json><![CDATA[[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json>
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
[{"run_bash":{"cmd":"pwd",}}]
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
fn adjacent_top_level_action_arrays_request_repair_instead_of_execution() {
    let env = parse_xml_envelope(
        r#"<response>
  <free_talk>two stage command plan</free_talk>
  <working_still_action>
    <action_json><![CDATA[[{"run_bash":{"cmd":"sleep 10","timeout_ms":1000}}],[{"run_bash":{"cmd":"sleep 10","background":true}}]]]></action_json>
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
    let env = parse_xml_envelope("<response><status>finished</status></response>", &caps());

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("status_tag_not_supported")
    );
    assert!(env.continue_work);
}

#[test]
fn parses_actions_from_cdata_json() {
    let env = parse_xml_envelope(
        r#"<response>
<free_talk>state</free_talk>
<working_still_action>
<action_json><![CDATA[[{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]]]></action_json>
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
      "cmd": "printf '%s\n' '<response><final_answer>not real</final_answer></response>' && printf '%s\n' '{\"working_still_action\":[{\"action\":\"run_bash\"}]}'",
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
        .contains("<response><final_answer>not real</final_answer>"));
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
<discard>pd_a</discard>
<offload>pd_b</offload>
<summary><![CDATA[keep state]]></summary>
</context_compact>
</response>"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none());
    assert_eq!(env.context_compacts.len(), 1);
    assert_eq!(env.context_compacts[0].delta_ids, vec!["pd_a", "pd_b"]);
    assert_eq!(env.context_compacts[0].discard_delta_ids, vec!["pd_a"]);
    assert_eq!(env.context_compacts[0].offload_delta_ids, vec!["pd_b"]);
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
<response><final_answer>not real</final_answer>
</summary>
</context_compact>
</response>"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.context_compacts.len(), 1);
    assert!(env.context_compacts[0]
        .summary
        .contains("<response><final_answer>not real</final_answer>"));
}

#[test]
fn parses_response_wrapped_in_xml_markdown_fence() {
    let env = parse_xml_envelope(
        r#"```xml
<response>
  <free_talk>finished</free_talk>
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
<action_json><![CDATA[[{"run_bash":{"cmd":"pwd"}}]]]></action_json>
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

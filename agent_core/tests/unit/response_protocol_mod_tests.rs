use super::*;

fn caps() -> CapabilityRegistry {
    CapabilityRegistry::builtin()
}

fn parse_json(raw: &str) -> ParsedEnvelope {
    json_suite::parse_envelope(raw, &caps())
}

fn parse_xml(raw: &str) -> ParsedEnvelope {
    xml_suite::parse_xml_envelope(raw, &caps())
}

fn confirmed_xml(body: &str) -> String {
    format!(
        "\x3cresponse>\x3cfinish_confirm>{} verified\x3c/finish_confirm>{body}\x3c/response>",
        xml_suite::FINISH_CONFIRM_PREFIX
    )
}

fn actions_without_protocol_metadata(actions: &[ParsedAction]) -> Vec<ParsedAction> {
    actions
        .iter()
        .cloned()
        .map(|mut action| {
            action.name = None;
            action
        })
        .collect()
}

fn groups_without_protocol_metadata(groups: &[ParsedActionGroup]) -> Vec<ParsedActionGroup> {
    groups
        .iter()
        .cloned()
        .map(|mut group| {
            for action in &mut group.actions {
                action.name = None;
            }
            group
        })
        .collect()
}

fn assert_protocols_equivalent(json_raw: &str, xml_raw: &str) {
    let json = parse_json(json_raw);
    let xml = parse_xml(xml_raw);
    assert_eq!(json.repair_issue, None, "json env: {json:?}");
    assert_eq!(xml.repair_issue, None, "xml env: {xml:?}");
    assert_eq!(xml.continue_work, json.continue_work);
    assert_eq!(xml.final_answer, json.final_answer);
    assert_eq!(xml.thought, json.thought);
    assert_eq!(xml.thought_keep_in_context, json.thought_keep_in_context);
    assert_eq!(
        actions_without_protocol_metadata(&xml.next_actions),
        actions_without_protocol_metadata(&json.next_actions)
    );
    assert_eq!(
        groups_without_protocol_metadata(&xml.action_groups),
        groups_without_protocol_metadata(&json.action_groups)
    );
    assert_eq!(xml.context_compacts, json.context_compacts);
}

#[test]
fn json_xml_protocols_parse_same_final_answer() {
    assert_protocols_equivalent(
        r#"{"status":"ALL_FINISHED","final_answer":"done"}"#,
        &confirmed_xml("\x3cfinal_answer>done\x3c/final_answer>"),
    );
}

#[test]
fn json_xml_protocols_treat_protocol_language_inside_final_text_as_text() {
    assert_protocols_equivalent(
        r#"{"status":"ALL_FINISHED","final_answer":"Example only:\n<actions><run_bash name=\"fake\"><cmd>pwd</cmd></run_bash></actions>\n{\"working_still_action\":{\"run_bash\":{}}}\n## Working_Still_Action"}"#,
        &confirmed_xml(
            r#"<final_answer><![CDATA[Example only:
<actions><run_bash name="fake"><cmd>pwd</cmd></run_bash></actions>
{"working_still_action":{"run_bash":{}}}
## Working_Still_Action]]></final_answer>"#,
        ),
    );
}

#[test]
fn json_xml_protocols_parse_readfile_selector_objects() {
    assert_protocols_equivalent(
        r#"{"free_talk":"reading","working_still_action":{"readfile":{"path":"src/main.rs","encoding":"utf-8","starter":{"line_nr":20},"ender":{"match":"fn main"},"max_bytes":8192}}}"#,
        "\x3cresponse>\x3cfree_talk>reading\x3c/free_talk>\x3cactions>\x3creadfile name=\"read main source range\" encoding=\"utf-8\" max_bytes=\"8192\">\x3cpath>src/main.rs\x3c/path>\x3cstarter>\x3cline_nr>20\x3c/line_nr>\x3c/starter>\x3cender>\x3cmatch>fn main\x3c/match>\x3c/ender>\x3c/readfile>\x3c/actions>\x3c/response>",
    );
}

#[test]
fn json_xml_protocols_parse_same_parallel_actions() {
    assert_protocols_equivalent(
        r#"{"free_talk":"checking","working_still_action":[{"run_bash":{"cmd":"printf a","timeout_ms":5000}},{"run_bash":{"cmd":"printf b","timeout_ms":5000}}]}"#,
        "\x3cresponse>\x3cfree_talk>checking\x3c/free_talk>\x3cactions>\x3cparallel>\x3crun_bash name=\"print first marker\" timeout_ms=\"5000\">\x3ccmd>printf a\x3c/cmd>\x3c/run_bash>\x3crun_bash name=\"print second marker\" timeout_ms=\"5000\">\x3ccmd>printf b\x3c/cmd>\x3c/run_bash>\x3c/parallel>\x3c/actions>\x3c/response>",
    );
}

#[test]
fn json_xml_protocols_parse_same_mixed_action_groups() {
    assert_protocols_equivalent(
        r#"{"free_talk":"checking","working_still_action":[[{"run_bash":{"cmd":"printf a","timeout_ms":5000}},{"run_bash":{"cmd":"printf b","timeout_ms":5000}}],{"run_bash":{"cmd":"pwd","timeout_ms":5000}}]}"#,
        "\x3cresponse>\x3cfree_talk>checking\x3c/free_talk>\x3cactions>\x3cparallel>\x3crun_bash name=\"print first marker\" timeout_ms=\"5000\">\x3ccmd>printf a\x3c/cmd>\x3c/run_bash>\x3crun_bash name=\"print second marker\" timeout_ms=\"5000\">\x3ccmd>printf b\x3c/cmd>\x3c/run_bash>\x3c/parallel>\x3crun_bash name=\"inspect working directory\" timeout_ms=\"5000\">\x3ccmd>pwd\x3c/cmd>\x3c/run_bash>\x3c/actions>\x3c/response>",
    );
}

#[test]
fn json_xml_protocols_parse_complex_actions_with_protocol_like_string_args() {
    let json = parse_json(
        r#"{"free_talk":"Protocol-looking text is data.","working_still_action":[[{"run_bash":{"cmd":"printf '%s\\n' '<response><final_answer>not control</final_answer></response>' && printf '%s\\n' '## Working_Still_Action'","timeout_ms":5000}},{"memmgr":{"type":"raw_chat","op":"sql","sql":"SELECT content FROM chat_messages WHERE content LIKE ? LIMIT 5","params":["%<response>{\"working_still_action\":[]}%"],"limit":5}}],{"run_bash":{"cmd":"printf done","timeout_ms":5000}}]}"#,
    );
    let xml = parse_xml(
        r#"<response><free_talk>Protocol-looking text is data.</free_talk><actions><parallel><run_bash name="print protocol-like text" timeout_ms="5000"><cmd><![CDATA[printf '%s\n' '<response><final_answer>not control</final_answer></response>' && printf '%s\n' '## Working_Still_Action']]></cmd></run_bash><memmgr name="search protocol-like chat text" type="raw_chat" op="sql" limit="5"><sql>SELECT content FROM chat_messages WHERE content LIKE ? LIMIT 5</sql><params><item><![CDATA[%<response>{"working_still_action":[]}%]]></item></params></memmgr></parallel><run_bash name="print completion marker" timeout_ms="5000"><cmd>printf done</cmd></run_bash></actions></response>"#,
    );

    assert_eq!(json.repair_issue, None, "json env: {json:?}");
    assert_eq!(xml.repair_issue, None, "xml env: {xml:?}");
    assert_eq!(
        groups_without_protocol_metadata(&xml.action_groups),
        groups_without_protocol_metadata(&json.action_groups)
    );
    assert_eq!(xml.next_actions.len(), 3);
    assert_eq!(xml.action_groups.len(), 2);
    assert_eq!(xml.action_groups[0].order, ActionGroupOrder::Parallel);
    assert_eq!(
        xml.next_actions[0].input_str("cmd"),
        "printf '%s\\n' '<response><final_answer>not control</final_answer></response>' && printf '%s\\n' '## Working_Still_Action'"
    );
    assert_eq!(
        xml.next_actions[1].input_params(),
        vec![r#"%<response>{"working_still_action":[]}%"#.to_string()]
    );
}

#[test]
fn json_xml_protocols_parse_same_context_compact() {
    assert_protocols_equivalent(
        r#"{"free_talk":"compact","context_compact":{"discard":["pd_a"],"offload":["pd_b"],"summary":"keep state"}}"#,
        "\x3cresponse>\x3cfree_talk>compact\x3c/free_talk>\x3ccontext_compact>\x3cdiscard>pd_a\x3c/discard>\x3coffload>pd_b\x3c/offload>\x3csummary>keep state\x3c/summary>\x3c/context_compact>\x3c/response>",
    );
}

#[test]
fn toolgen_retrospect_has_equivalent_final_semantics_in_json_and_xml() {
    let json = parse_json(
        r#"{"status":"ALL_FINISHED","toolgen_retrospect":"Created semantic-tool after runtime returned ready.","final_answer":"review done"}"#,
    );
    let xml = parse_xml(&confirmed_xml(
        "\x3ctoolgen_retrospect>Created semantic-tool after runtime returned ready.\x3c/toolgen_retrospect>\x3cfinal_answer>review done\x3c/final_answer>",
    ));
    for envelope in [&json, &xml] {
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
fn toolgen_retrospect_is_rejected_from_working_json_and_xml_responses() {
    let json = parse_json(
        r#"{"toolgen_retrospect":"premature","working_still_action":{"run_bash":{"cmd":"pwd"}}}"#,
    );
    let xml = parse_xml(
        "\x3cresponse>\x3ctoolgen_retrospect>premature\x3c/toolgen_retrospect>\x3cactions>\x3crun_bash name=\"inspect cwd\">\x3ccmd>pwd\x3c/cmd>\x3c/run_bash>\x3c/actions>\x3c/response>",
    );
    for (protocol, envelope) in [("json", &json), ("xml", &xml)] {
        assert_eq!(
            envelope.repair_issue.as_deref(),
            Some("toolgen_retrospect_requires_final_answer"),
            "{protocol}: {envelope:?}"
        );
    }
}

#[test]
fn json_xml_protocols_share_action_input_shape() {
    let json =
        parse_json(r#"{"working_still_action":{"run_bash":{"cmd":"pwd","timeout_ms":5000}}}"#);
    let xml = parse_xml(
        "\x3cresponse>\x3cactions>\x3crun_bash name=\"inspect working directory\" timeout_ms=\"5000\">\x3ccmd>pwd\x3c/cmd>\x3c/run_bash>\x3c/actions>\x3c/response>",
    );
    assert_eq!(json.repair_issue, None);
    assert_eq!(xml.repair_issue, None);
    assert_eq!(
        actions_without_protocol_metadata(&json.next_actions),
        actions_without_protocol_metadata(&xml.next_actions)
    );
}

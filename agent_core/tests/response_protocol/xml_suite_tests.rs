use super::*;

fn caps() -> CapabilityRegistry {
    CapabilityRegistry::builtin()
}

fn parse_confirmed_final(content: &str) -> ParsedEnvelope {
    let confirm = format!("<finish_confirm>{FINISH_CONFIRM_PREFIX}</finish_confirm>");
    let insertion_point = content
        .find("<toolgen_retrospect")
        .or_else(|| content.find("<final_answer"))
        .expect("confirmed final fixture must contain a final branch");
    let response = format!(
        "{}{}{}",
        &content[..insertion_point],
        confirm,
        &content[insertion_point..]
    );
    parse_xml_envelope(&response, &caps())
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
        examples.len() >= 3,
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
    let env = parse_confirmed_final("<response><final_answer>done</final_answer></response>");
    assert!(env.repair_issue.is_none());
    assert!(!env.continue_work);
    assert_eq!(env.final_answer, "done");
}

#[test]
fn final_answer_requires_a_valid_finish_confirmation() {
    let missing = parse_xml_envelope(
        "<response><final_answer>must not escape</final_answer></response>",
        &caps(),
    );
    assert_eq!(
        missing.repair_issue.as_deref(),
        Some("finish_confirm_required_before_final_answer")
    );
    assert!(missing.final_answer.is_empty());
    assert!(missing.continue_work);

    let wrong_prefix = parse_xml_envelope(
        "<response><finish_confirm>I think it is done.</finish_confirm><final_answer>must not escape</final_answer></response>",
        &caps(),
    );
    assert_eq!(
        wrong_prefix.repair_issue.as_deref(),
        Some("finish_confirm_prefix_invalid")
    );
    assert!(wrong_prefix.final_answer.is_empty());
    assert!(wrong_prefix.continue_work);

    let previous_prefix = parse_xml_envelope(
        "<response><finish_confirm>Now let me think seriously twice before I stop. Do I really complete all user's valid tasks or need to stop now? If not, i should continue action.</finish_confirm><final_answer>must not escape</final_answer></response>",
        &caps(),
    );
    assert_eq!(
        previous_prefix.repair_issue.as_deref(),
        Some("finish_confirm_prefix_invalid")
    );
    assert!(previous_prefix.final_answer.is_empty());
    assert!(previous_prefix.continue_work);
}

#[test]
fn runtime_finish_confirmation_prefix_matches_the_prompt_contract() {
    assert!(XML_RESPONSE_PROTOCOL_SECTION
        .contains(&format!("CONFIRM_PREFIX: \"{FINISH_CONFIRM_PREFIX}\"")));
}

#[test]
fn finish_confirmation_can_reconsider_and_continue_with_actions() {
    let raw = format!(
        "<response><finish_confirm>{FINISH_CONFIRM_PREFIX} More evidence is needed.</finish_confirm><actions><run_bash><cmd>pwd</cmd></run_bash></actions></response>"
    );
    let env = parse_xml_envelope(&raw, &caps());

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert!(env.continue_work);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
}

#[test]
fn finish_confirmation_is_unique_and_precedes_the_state_branch() {
    let duplicated = format!(
        "<response><finish_confirm>{FINISH_CONFIRM_PREFIX}</finish_confirm><finish_confirm>{FINISH_CONFIRM_PREFIX}</finish_confirm><final_answer>done</final_answer></response>"
    );
    assert_eq!(
        parse_xml_envelope(&duplicated, &caps())
            .repair_issue
            .as_deref(),
        Some("xml_duplicate_finish_confirm")
    );

    let late = format!(
        "<response><final_answer>done</final_answer><finish_confirm>{FINISH_CONFIRM_PREFIX}</finish_confirm></response>"
    );
    let env = parse_xml_envelope(&late, &caps());
    assert_eq!(env.repair_issue.as_deref(), Some("xml_tags_out_of_order"));
}

#[test]
fn largest_complete_response_root_finds_prefixed_final_root() {
    let raw = "preface<response><final_answer>done</final_answer></response>";
    let (start, end) =
        largest_complete_response_root(raw).expect("prefixed complete response root must be found");
    assert_eq!(
        &raw[start..end],
        "<response><final_answer>done</final_answer></response>"
    );
    assert!(parse_response_fields(&raw[start..end]).is_some());
}

#[test]
fn extracted_final_answer_requires_an_unmodified_retry() {
    for raw in [
        "preface<response><final_answer>done</final_answer></response>",
        "<response><final_answer>done</final_answer></response>trailing",
        "```xml\n<response><final_answer>done</final_answer></response>\n```",
        "preface<response><actions><run_bash><cmd>true</cmd></run_bash></actions><final_answer>done</final_answer></response>",
    ] {
        let env = parse_xml_envelope(raw, &caps());
        assert_eq!(
            env.repair_issue.as_deref(),
            Some("xml_recovered_final_answer_requires_retry"),
            "raw={raw}"
        );
        assert!(env.final_answer.is_empty(), "raw={raw}");
        assert!(env.continue_work, "raw={raw}");
        assert!(env.next_actions.is_empty(), "raw={raw}");
    }
}

#[test]
fn missing_response_boundaries_are_not_synthesized() {
    for raw in [
        "<final_answer>done</final_answer>",
        "<response><final_answer>done</final_answer>",
        "<final_answer>done</final_answer></response>",
        "preface<response><final_answer>done</final_answer>",
        "<response><actions><run_bash><cmd>true</cmd></run_bash></actions>",
        "<actions><run_bash><cmd>true</cmd></run_bash></actions></response>",
    ] {
        let env = parse_xml_envelope(raw, &caps());
        assert!(
            env.repair_issue.is_some(),
            "missing response boundary must be a protocol deviation: raw={raw}"
        );
        assert_ne!(
            env.repair_issue.as_deref(),
            Some("xml_recovered_final_answer_requires_retry"),
            "incomplete roots are not extracted complete roots: raw={raw}"
        );
        assert!(env.final_answer.is_empty(), "raw={raw}");
        assert!(env.next_actions.is_empty(), "raw={raw}");
        assert!(env.action_groups.is_empty(), "raw={raw}");
        assert!(env.accepted_response.is_none(), "raw={raw}");
    }
}

#[test]
fn recovery_remains_allowed_for_non_terminal_actions() {
    let env = parse_xml_envelope(
        "preface<response><actions><run_bash><cmd>printf safe</cmd></run_bash></actions></response>trailing",
        &caps(),
    );
    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].input_str("cmd"), "printf safe");
    assert!(env.final_answer.is_empty());
    assert!(env.continue_work);
}

#[test]
fn cdata_text_fields_are_literal_while_normal_xml_text_decodes_once() {
    let cdata = parse_confirmed_final(
        "<response><final_answer><![CDATA[&lt;literal&gt; &amp;]]></final_answer></response>",
    );
    assert_eq!(cdata.final_answer, "&lt;literal&gt; &amp;");

    let escaped = parse_confirmed_final(
        "<response><final_answer>&amp;lt;decoded-once&amp;gt;</final_answer></response>",
    );
    assert_eq!(escaped.final_answer, "&lt;decoded-once&gt;");
}

#[test]
fn parses_xml_native_actions_with_sequential_and_parallel_groups() {
    let env = parse_xml_envelope(
        r#"<response>
  <free_talk>Inspect, then test.</free_talk>
  <actions>
    <run_bash timeout_ms="5000"><cmd>pwd</cmd></run_bash>
    <parallel>
      <run_bash timeout_ms="6000"><cmd>git status --short</cmd></run_bash>
      <run_bash background="true"><cmd><![CDATA[printf '%s\n' '<ready>' > result.txt]]></cmd></run_bash>
    </parallel>
  </actions>
</response>"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.action_groups.len(), 2);
    assert_eq!(
        env.action_groups[0].order,
        crate::ActionGroupOrder::Sequential
    );
    assert_eq!(
        env.action_groups[1].order,
        crate::ActionGroupOrder::Parallel
    );
    assert_eq!(env.action_groups[1].actions.len(), 2);
    assert_eq!(env.next_actions[0].input_str("cmd"), "pwd");
    assert_eq!(env.next_actions[0].input_i64("timeout_ms"), Some(5000));
    assert_eq!(env.next_actions[1].input_i64("timeout_ms"), Some(6000));
    assert!(env.next_actions[2].input_bool("background"));
    assert!(env.next_actions[2].input_raw_str("cmd").contains("<ready>"));
}

#[test]
fn recovers_one_missing_final_tool_close_before_parallel_close() {
    let env = parse_xml_envelope(
        r#"<response>
  <actions>
    <parallel>
      <run_bash timeout_ms="5000"><cmd>pwd</cmd></run_bash>
      <run_bash timeout_ms="5000"><cmd><![CDATA[git status --short]]></cmd>
    </parallel>
  </actions>
</response>"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.action_groups.len(), 1);
    assert_eq!(env.action_groups[0].actions.len(), 2);
    assert_eq!(env.next_actions[1].input_str("cmd"), "git status --short");
}

#[test]
fn does_not_recover_argument_or_unrelated_mismatched_close_tags() {
    for raw in [
        r#"<response><actions><parallel><run_bash><cmd>pwd</run_bash></parallel></actions></response>"#,
        r#"<response><actions><parallel><run_bash><cmd>pwd</cmd></memmgr></parallel></actions></response>"#,
    ] {
        let env = parse_xml_envelope(raw, &caps());
        assert!(
            env.repair_issue
                .as_deref()
                .is_some_and(|issue| issue.contains("mismatched_close")),
            "{:?}",
            env.repair_issue
        );
    }
}

#[test]
fn xml_native_actions_reject_nested_parallel_and_duplicate_arguments() {
    let nested = parse_xml_envelope(
        r#"<response><actions><parallel><parallel><run_bash><cmd>pwd</cmd></run_bash></parallel></parallel></actions></response>"#,
        &caps(),
    );
    assert!(nested
        .repair_issue
        .as_deref()
        .is_some_and(|issue| issue.ends_with("parallel_nested")));

    let duplicate = parse_xml_envelope(
        r#"<response><actions><run_bash timeout_ms="1"><cmd>pwd</cmd><timeout_ms>2</timeout_ms></run_bash></actions></response>"#,
        &caps(),
    );
    assert!(duplicate
        .repair_issue
        .as_deref()
        .is_some_and(|issue| issue.ends_with("input.timeout_ms_duplicate")));
}

#[test]
fn xml_native_actions_convert_schema_typed_arrays_objects_and_mcp_tool_names() {
    let tool = crate::mcp::McpTool {
        server_id: "demo".to_string(),
        server_name: "Demo".to_string(),
        name: "batch".to_string(),
        action_name: "mcp.demo.batch".to_string(),
        description: "Batch demo".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["files", "options"],
            "properties": {
                "files": {"type": "array", "items": {"type": "string"}},
                "options": {
                    "type": "object",
                    "properties": {
                        "limit": {"type": "integer"},
                        "strict": {"type": "boolean"}
                    }
                }
            }
        }),
    };
    let capabilities = caps().with_mcp_tools(&[tool]).expect("MCP capability");
    let env = parse_xml_envelope(
        r#"<response><actions><mcp.demo.batch>
          <files><item>README.md</item><item>package.json</item></files>
          <options strict="true"><limit>20</limit></options>
        </mcp.demo.batch></actions></response>"#,
        &capabilities,
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.next_actions[0].action, "mcp.demo.batch");
    assert_eq!(
        env.next_actions[0].raw_input["files"],
        serde_json::json!(["README.md", "package.json"])
    );
    assert_eq!(
        env.next_actions[0].raw_input["options"],
        serde_json::json!({"strict": true, "limit": 20})
    );
}

#[test]
fn xml_native_values_cover_nullable_large_integer_additional_properties_and_literal_cdata() {
    let tool = crate::mcp::McpTool {
        server_id: "types".to_string(),
        server_name: "Types".to_string(),
        name: "probe".to_string(),
        action_name: "mcp.types.probe".to_string(),
        description: "Exercise XML value conversion".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["count", "ratio", "choice", "nothing", "tuple", "metadata", "rows", "payload"],
            "properties": {
                "count": {"type": ["integer", "null"]},
                "ratio": {"anyOf": [{"type": "number"}, {"type": "null"}]},
                "choice": {"oneOf": [{"type": "integer"}, {"type": "string"}]},
                "nothing": {"type": ["string", "null"]},
                "tuple": {
                    "type": "array",
                    "prefixItems": [{"type": "integer"}, {"type": "boolean"}]
                },
                "metadata": {
                    "type": "object",
                    "additionalProperties": {"type": "integer"}
                },
                "rows": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "ok": {"type": "boolean"},
                            "name": {"type": "string"}
                        }
                    }
                },
                "payload": {"type": "string"}
            }
        }),
    };
    let capabilities = caps().with_mcp_tools(&[tool]).expect("MCP capability");
    let env = parse_xml_envelope(
        r#"<response><actions><mcp.types.probe count="18446744073709551615" ratio="1.25" choice="7">
          <nothing/>
          <tuple><item>9</item><item>false</item></tuple>
          <metadata><alpha>1</alpha><beta>2</beta></metadata>
          <rows><item ok="true"><name>A&#38;B</name></item></rows>
          <payload><![CDATA[  &lt;raw>&value  ]]></payload>
        </mcp.types.probe></actions></response>"#,
        &capabilities,
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    let input = &env.next_actions[0].raw_input;
    assert_eq!(input["count"], serde_json::json!(u64::MAX));
    assert_eq!(input["ratio"], serde_json::json!(1.25));
    assert_eq!(input["choice"], serde_json::json!(7));
    assert_eq!(input["nothing"], serde_json::Value::Null);
    assert_eq!(input["tuple"], serde_json::json!([9, false]));
    assert_eq!(
        input["metadata"],
        serde_json::json!({"alpha": 1, "beta": 2})
    );
    assert_eq!(
        input["rows"],
        serde_json::json!([{"ok": true, "name": "A&B"}])
    );
    assert_eq!(input["payload"], "  &lt;raw>&value  ");
}

#[test]
fn capability_prompt_exposes_the_same_nested_types_that_xml_runtime_accepts() {
    let tool = crate::mcp::McpTool {
        server_id: "typed".to_string(),
        server_name: "Typed".to_string(),
        name: "submit".to_string(),
        action_name: "mcp.typed.submit".to_string(),
        description: "Submit typed values".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["rows"],
            "properties": {
                "rows": {
                    "type": "array",
                    "description": "Rows to submit.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "count": {"type": "integer"},
                            "enabled": {"type": "boolean"}
                        }
                    }
                }
            }
        }),
    };
    let capabilities = caps().with_mcp_tools(&[tool]).expect("MCP capability");
    let prompt = capabilities.render_tool_catalog_markdown();

    assert!(prompt.contains(
        "Type: array<object {count: integer, enabled: boolean, name: string}>. Rows to submit."
    ));
    assert!(prompt.contains("Provide arguments matching this MCP tool's input options."));
    assert!(prompt.contains(
        "the MCP server validates advanced schema constraints, and any rejection returns as tool evidence"
    ));
    assert!(!prompt.contains("Pass a JSON object matching this MCP tool"));
}

#[test]
fn xml_native_actions_reject_unsafe_xml_constructs_and_resource_exhaustion() {
    for raw in [
        r#"<response><actions><run_bash><cmd>&unknown;</cmd></run_bash></actions></response>"#,
        r#"<response><actions><run_bash><cmd>&unterminated</cmd></run_bash></actions></response>"#,
        r#"<response><actions><run_bash><cmd><!-- hidden --></cmd></run_bash></actions></response>"#,
        r#"<response><actions><!DOCTYPE run_bash><run_bash><cmd>pwd</cmd></run_bash></actions></response>"#,
    ] {
        let env = parse_xml_envelope(raw, &caps());
        assert!(env.repair_issue.is_some(), "unsafe XML accepted: {raw}");
        assert!(env.next_actions.is_empty(), "unsafe XML executed: {raw}");
        let issue = env.repair_issue.as_deref().expect("repair issue");
        let guidance = xml_repair_instruction_for_response(issue, raw);
        assert!(guidance.contains("Exact protocol error:"));
        assert!(guidance.contains("Cause:"));
        assert!(guidance.contains("Correction:"));
    }

    let nested = format!(
        "<response><actions><run_bash><cmd>{}x{}</cmd></run_bash></actions></response>",
        "<level>".repeat(MAX_XML_ACTION_DEPTH + 1),
        "</level>".repeat(MAX_XML_ACTION_DEPTH + 1)
    );
    let env = parse_xml_envelope(&nested, &caps());
    assert!(env
        .repair_issue
        .as_deref()
        .is_some_and(|issue| issue.contains("xml_depth_limit_exceeded")));
    assert!(env.next_actions.is_empty());

    let oversized = format!(
        "<response><actions>{}</actions></response>",
        "<run_bash/>".repeat(MAX_XML_ACTION_ELEMENTS + 1)
    );
    let env = parse_xml_envelope(&oversized, &caps());
    assert!(env
        .repair_issue
        .as_deref()
        .is_some_and(|issue| issue.contains("xml_element_limit_exceeded")));
    assert!(env.next_actions.is_empty());
}

#[test]
fn xml_native_action_batch_is_atomic_when_a_later_action_is_invalid() {
    let env = parse_xml_envelope(
        r#"<response><actions>
          <run_bash><cmd>printf should-not-run</cmd></run_bash>
          <run_bash timeout_ms="not-an-integer"><cmd>pwd</cmd></run_bash>
        </actions></response>"#,
        &caps(),
    );
    assert!(env.repair_issue.is_some());
    assert!(env.next_actions.is_empty());
    assert!(env.action_groups.is_empty());
}

#[test]
fn xml_protocol_rejects_json_markdown_and_plain_text_roots() {
    let cases = [
        r#"{"final_answer":"done"}"#,
        r#"[{"run_bash":{"cmd":"pwd"}}]"#,
        "## Final_Answer\n\ndone",
        "plain final prose",
    ];

    for raw in cases {
        let env = parse_xml_envelope(raw, &caps());
        assert_eq!(
            env.repair_issue.as_deref(),
            Some("xml_response_root_missing"),
            "raw={raw}"
        );
        assert!(env.final_answer.is_empty(), "raw={raw}");
        assert!(env.next_actions.is_empty(), "raw={raw}");
        assert!(env.action_groups.is_empty(), "raw={raw}");
    }
}

#[test]
fn outer_text_is_discarded_when_a_non_final_response_root_is_extracted() {
    let env = parse_xml_envelope(
        r#"prefix prose <actions><run_bash><cmd>must not execute</cmd></run_bash></actions>
<response>
  <free_talk>inside thought</free_talk>
  <actions><run_bash><cmd>printf safe</cmd></run_bash></actions>
</response>
trailing prose <run_bash><cmd>also must not execute</cmd></run_bash>"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].input_str("cmd"), "printf safe");
    assert_eq!(env.thought, "inside thought");
    assert_eq!(
        env.accepted_response.as_deref(),
        Some(
            "<response>\n  <free_talk>inside thought</free_talk>\n  <actions><run_bash><cmd>printf safe</cmd></run_bash></actions>\n</response>"
        )
    );
    assert!(!env.thought.contains("must not execute"));
    assert!(env.runtime_note.is_none());
}

#[test]
fn multiple_response_roots_select_the_largest_complete_root() {
    let env = parse_xml_envelope(
        r#"before
<response><actions><run_bash><cmd>must not execute</cmd></run_bash></actions></response>
noise
<response>
  <free_talk>use the larger complete response</free_talk>
  <actions><run_bash><cmd>printf selected</cmd></run_bash></actions>
</response>
after"#,
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].input_str("cmd"), "printf selected");
    assert_eq!(env.thought, "use the larger complete response");
    let accepted = env.accepted_response.as_deref().unwrap();
    assert!(accepted.contains("printf selected"));
    assert!(!accepted.contains("must not execute"));
}

#[test]
fn equal_sized_response_roots_keep_the_first_complete_root() {
    let env = parse_xml_envelope(
        "<response><actions><run_bash><cmd>printf first</cmd></run_bash></actions></response>\
         <response><actions><run_bash><cmd>printf later</cmd></run_bash></actions></response>",
        &caps(),
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(env.next_actions.len(), 1);
    assert_eq!(env.next_actions[0].input_str("cmd"), "printf first");
}

#[test]
fn root_repair_moves_free_talk_inside_response_with_matching_action_branch() {
    let malformed = r#"<free_talk>searching</free_talk>
<response><working_still_action>...</working_still_action></response>"#;
    let instruction = xml_repair_instruction_for_response("xml_content_before_response", malformed);

    assert!(instruction.contains("placed content before the <response> root"));
    assert!(instruction.contains(
            "The response must be in format '<response><free_talk>...</free_talk><actions>...</actions></response>'"
        ));
    assert!(instruction.contains("output nothing before <response> or after </response>"));
}

#[test]
fn root_repair_selects_the_branch_present_in_the_malformed_response() {
    let final_instruction = xml_repair_instruction_for_response(
        "xml_content_before_response",
        "preface<response><final_answer>done</final_answer></response>",
    );
    assert!(final_instruction.contains(
        "<response><finish_confirm>CONFIRM_PREFIX followed by the confirmation</finish_confirm><final_answer>...</final_answer></response>"
    ));

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
                name: "self closing response root",
                raw: "<response/>",
                issue: "xml_response_root_self_closing",
                guidance: "one complete <response>...</response> root",
            },
            Case {
                name: "empty response body",
                raw: "<response></response>",
                issue: "next_actions_required_when_status_working",
                guidance: "必须提供非空 <actions>",
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
                raw: "<response><actions><run_bash><cmd>pwd</cmd></run_bash></actions><final_answer>done</final_answer></response>",
                issue: "status_finished_must_not_include_next_actions",
                guidance: "不能同时包含 <actions>",
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
                name: "unknown tool",
                raw: "<response><actions><not_a_tool/></actions></response>",
                issue: "unsupported_action:not_a_tool",
                guidance: "not in the capability catalog",
            },
            Case {
                name: "missing required run bash command",
                raw: "<response><actions><run_bash/></actions></response>",
                issue: "actions[0][0].input.any_required:cmd|loop_cmd",
                guidance: "do not satisfy the capability schema",
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

    assert!(cases.len() >= 14, "keep the malformed corpus substantial");
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
        assert!(instruction.contains(&format!("Exact protocol error: `{}`", case.issue)));
        assert!(instruction.contains("Cause:"));
        assert!(instruction.contains("Correction:"));
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
    let instruction = xml_repair_instruction_for_response(
        "actions[0][1].input.timeout_ms_must_be_integer",
        "<response></response>",
    );
    assert!(instruction
        .contains("Exact protocol error: `actions[0][1].input.timeout_ms_must_be_integer`"));
    assert!(instruction.contains("block, stage, and tool"));
    assert!(instruction.contains("input.<name>"));
    assert!(instruction.contains("Cause: Argument `timeout_ms` must be an integer"));
    assert!(instruction.contains("change the smallest failing element or argument"));
}

#[test]
fn common_action_repair_issues_have_specific_correction_guidance() {
    let cases = [
        (
            "actions[0].actions_required",
            "<actions> or <parallel> element is empty",
        ),
        ("unsupported_action:ghost", "not in the capability catalog"),
        (
            "actions[0].input.cmd_required",
            "do not satisfy the capability schema",
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
    let env = parse_confirmed_final(
        r#"<response>
  <final_answer><![CDATA[
Example response delta:

<response>
  <final_answer>done</final_answer>
</response>

[END DELTA]
  ]]></final_answer>
</response>"#,
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert!(!env.continue_work);
    assert!(env.final_answer.contains("<response>"));
    assert!(env.final_answer.contains("</final_answer>"));
    assert!(env.final_answer.contains("[END DELTA]"));
}

#[test]
fn final_answer_xml_action_examples_are_not_parsed_as_real_actions() {
    let env = parse_confirmed_final(
        r#"<response>
  <final_answer><![CDATA[
This is only a user-facing example:

<working_still_action>
  <action_json>{"run_bash": {} // missing cmd in the example on purpose
  }</action_json>
</working_still_action>
  ]]></final_answer>
</response>"#,
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
    let env = parse_confirmed_final(
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
    let env = parse_confirmed_final(
        r#"<response>
  <final_answer><![CDATA[
First example:
<response><final_answer>one</final_answer></response>
<response><final_answer>two</final_answer></response>
  ]]></final_answer>
</response>"#,
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
    let env = parse_confirmed_final(
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
    let env = parse_confirmed_final(
        r#"<response>
  <final_answer>
This answer explains multiple protocol snippets:
<legacy_note>fake legacy note inside final answer</legacy_note>
<summary>fake compact summary inside final answer</summary>
<free_talk>fake free talk inside final answer</free_talk>
None of these are real control fields.
  </final_answer>
</response>"#,
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
    let env = parse_confirmed_final(
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
    );

    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert!(!env.continue_work);
    assert!(env.next_actions.is_empty());
    assert!(env.action_groups.is_empty());
    assert!(env.final_answer.contains("<working_still_action>"));
}

#[test]
fn final_answer_nested_xml_preserves_attributes_and_escaped_text() {
    let env = parse_confirmed_final(
        r#"<response>
  <final_answer>
Report:
<diagnostic level="warn" source="unit-test"><message>ok</message><empty marker="1" /></diagnostic>
Escaped literal: &lt;response&gt;not protocol&lt;/response&gt;
  </final_answer>
</response>"#,
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
fn old_finished_status_requests_repair() {
    let env = parse_xml_envelope("<response><status>finished</status></response>", &caps());

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("status_tag_not_supported")
    );
    assert!(env.continue_work);
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
<discard>pd_a</discard>
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
fn rejects_response_wrapped_in_xml_markdown_fence() {
    let env = parse_xml_envelope(
        r#"```xml
<response>
  <free_talk>finished</free_talk>
  <final_answer>done</final_answer>
</response>
```"#,
        &caps(),
    );

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("xml_recovered_final_answer_requires_retry")
    );
    assert!(env.final_answer.is_empty());
}

#[test]
fn xml_state_branch_must_choose_one() {
    let env = parse_xml_envelope(
        r#"<response>
<free_talk>compact and act</free_talk>
<context_compact>
<discard>pd_a</discard>
<summary>keep state</summary>
</context_compact>
<actions><run_bash><cmd>pwd</cmd></run_bash></actions>
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

#[test]
fn removed_action_container_is_rejected_as_unknown_top_level_content() {
    let env = parse_xml_envelope(
        r#"<response>
  <free_talk>writing files</free_talk>
  <working_still_action><run_bash><cmd>pwd</cmd></run_bash></working_still_action>
</response>"#,
        &caps(),
    );

    assert_eq!(
        env.repair_issue.as_deref(),
        Some("xml_unexpected_content_inside_response")
    );
}

#[test]
fn parses_toolgen_retrospect_immediately_before_final_answer() {
    let env = parse_confirmed_final(
        r#"<response><free_talk>reviewed</free_talk><toolgen_retrospect>Created log-inspector and runtime returned status: ready.</toolgen_retrospect><final_answer>internal completion</final_answer></response>"#,
    );
    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert_eq!(
        env.toolgen_retrospect,
        "Created log-inspector and runtime returned status: ready."
    );
    assert_eq!(env.final_answer, "internal completion");
}

#[test]
fn toolgen_retrospect_is_opaque_when_it_contains_protocol_shaped_text() {
    let env = parse_confirmed_final(
        r#"<response><toolgen_retrospect><![CDATA[README demonstrates <response><working_still_action><action_json>{\"fake\":{}}</action_json></working_still_action></response> literally.]]></toolgen_retrospect><final_answer>done</final_answer></response>"#,
    );
    assert!(env.repair_issue.is_none(), "{:?}", env.repair_issue);
    assert!(env.toolgen_retrospect.contains("<working_still_action>"));
    assert_eq!(env.final_answer, "done");
}

#[test]
fn toolgen_retrospect_without_final_answer_requests_repair() {
    let env = parse_xml_envelope(
        r#"<response><toolgen_retrospect>created a tool</toolgen_retrospect></response>"#,
        &caps(),
    );
    assert_eq!(
        env.repair_issue.as_deref(),
        Some("toolgen_retrospect_requires_final_answer")
    );
}

#[test]
fn toolgen_retrospect_with_working_actions_requests_repair() {
    let env = parse_xml_envelope(
        r#"<response><toolgen_retrospect>not finished</toolgen_retrospect><actions><run_bash timeout_ms="5000"><cmd>pwd</cmd></run_bash></actions></response>"#,
        &caps(),
    );
    assert_eq!(
        env.repair_issue.as_deref(),
        Some("toolgen_retrospect_requires_final_answer")
    );
}

#[test]
fn toolgen_retrospect_after_final_answer_requests_order_repair() {
    let env = parse_xml_envelope(
        r#"<response><final_answer>done</final_answer><toolgen_retrospect>late</toolgen_retrospect></response>"#,
        &caps(),
    );
    assert_eq!(env.repair_issue.as_deref(), Some("xml_tags_out_of_order"));
}

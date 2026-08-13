use super::*;
use crate::response_protocol::json_suite::JsonSuiteV1;
use crate::response_protocol::markdown_suite::MarkdownSuiteV1;
use crate::response_protocol::xml_suite::XmlSuiteV1;

#[test]
fn prompt_renderer_injects_protocol_and_visible_delta_roles() {
    let delta = PromptDelta {
        delta_id: "pd_test_1".to_string(),
        time_ms: 1,
        hidden_slice_ids: vec!["ps_test_1_s002".to_string()],
        slices: vec![
            PromptSlice {
                delta_id: "pd_test_1".to_string(),
                slice_id: "ps_test_1_s001".to_string(),
                prompt_type: "user_question".to_string(),
                time_ms: 2,
                text: "hello".to_string(),
                slice_index: 1,
                slice_count: 2,
            },
            PromptSlice {
                delta_id: "pd_test_1".to_string(),
                slice_id: "ps_test_1_s002".to_string(),
                prompt_type: "llm_response".to_string(),
                time_ms: 3,
                text: "HIDDEN".to_string(),
                slice_index: 2,
                slice_count: 2,
            },
            PromptSlice {
                delta_id: "pd_test_1".to_string(),
                slice_id: "ps_test_1_s003".to_string(),
                prompt_type: "result_of_llm_action".to_string(),
                time_ms: 4,
                text: "Action result: run_bash\nok".to_string(),
                slice_index: 3,
                slice_count: 3,
            },
        ],
    };
    let rendered_static = render_static_prompt(
        "{{RESPONSE_PROTOCOL_SECTION}}
{{TOOL_CATALOG}}
{{SKILL_HEADERS}}",
        &CapabilityRegistry::builtin(),
        &MarkdownSuiteV1,
        "TIMEM_ASSISTANT",
    );
    let rendered = render_prompt_with_rendered_static(
        &rendered_static,
        &[delta],
        "TIMEM_ASSISTANT",
        "one Markdown response with one state branch",
    );
    assert!(rendered.contains("Response Protocol"));
    assert!(rendered.contains("memmgr"));
    assert!(rendered.contains("hello"));
    assert!(rendered.contains("[BEGIN DELTA]"));
    assert!(rendered.contains("## USER"));
    assert!(rendered.contains("## SYSTEM"));
    assert!(!rendered.contains("## ACTIONS"));
    assert!(
        rendered.contains("The following are results of TIMEM_ASSISTANT newly initiated actions:")
    );
    assert!(rendered.contains("Action result: run_bash"));
    assert!(!rendered.contains("slice_id:"));
    assert!(!rendered.contains("prompt_type:"));
    assert!(!rendered.contains("HIDDEN"));
    assert!(rendered.ends_with(
        "Now please fulfill your response part like one Markdown response with one state branch:"
    ));
}

#[test]
fn formatted_response_trailer_parser_extracts_heading_free_trailer() {
    let prompt = format!(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n\n{}",
        formatted_response_trailer("one-root label <response>...</response>", "Ai7")
    );
    let (prefix, trailer) = split_formatted_response_trailer(&prompt);
    assert_eq!(prefix, "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]");
    assert_eq!(
        trailer.as_deref(),
        Some("Now please fulfill your response part like one-root label <response>...</response>:")
    );
}

#[test]
fn formatted_response_trailer_uses_each_protocol_shape_hint() {
    assert_eq!(
        formatted_response_trailer("one-root label <response>...</response>", "Ai7"),
        "Now please fulfill your response part like one-root label <response>...</response>:"
    );
    assert_eq!(
        formatted_response_trailer("one JSON object {...}", "Ai7"),
        "Now please fulfill your response part like one JSON object {...}:"
    );
    assert_eq!(
        formatted_response_trailer("one Markdown response with one state branch", "Ai7"),
        "Now please fulfill your response part like one Markdown response with one state branch:"
    );
}

#[test]
fn formatted_response_trailer_parser_rejects_empty_legacy_assistant_heading() {
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n\nNow please fulfill your response part in one-JSON format as required in protocol:\n## ";
    let (prefix, trailer) = split_formatted_response_trailer(prompt);
    assert_eq!(prefix, prompt);
    assert_eq!(trailer, None);
}

#[test]
fn formatted_response_trailer_parser_accepts_legacy_assistant_heading() {
    let prompt = "STATIC\n\nNow please fulfill your response part in one-XML format as required in protocol:\n## Ai7";
    let (prefix, trailer) = split_formatted_response_trailer(prompt);
    assert_eq!(prefix, "STATIC");
    assert_eq!(
        trailer.as_deref(),
        Some("Now please fulfill your response part in one-XML format as required in protocol:\n## Ai7")
    );
}

#[test]
fn prompt_renderer_replaces_current_protocol_language() {
    let template = "Return {{CURRENT_PROTOCOL_LANG}}\n{{RESPONSE_PROTOCOL_SECTION}}";
    let markdown = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &MarkdownSuiteV1,
        "Ai7",
    );
    let json = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &JsonSuiteV1,
        "Ai7",
    );
    let xml = render_static_prompt(template, &CapabilityRegistry::builtin(), &XmlSuiteV1, "Ai7");

    assert!(markdown.contains("Return Markdown"));
    assert!(json.contains("Return JSON"));
    assert!(xml.contains("Return XML"));
    assert!(!markdown.contains("{{CURRENT_PROTOCOL_LANG}}"));
    assert!(!json.contains("{{CURRENT_PROTOCOL_LANG}}"));
    assert!(!xml.contains("{{CURRENT_PROTOCOL_LANG}}"));
}

#[test]
fn prompt_renderer_replaces_assistant_id() {
    let rendered = render_static_prompt(
        "YOUR ID is: {{ASSSISTANT_ID}}\n## ASSSISTANT_ID",
        &CapabilityRegistry::builtin(),
        &MarkdownSuiteV1,
        "Ai7",
    );
    assert!(rendered.contains("YOUR ID is: Ai7"));
    assert!(rendered.contains("## Ai7"));
    assert!(!rendered.contains("{{ASSSISTANT_ID}}"));
    assert!(!rendered.contains("ASSSISTANT_ID"));
}

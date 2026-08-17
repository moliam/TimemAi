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
                component_id: String::new(),
                prompt_type: "user_question".to_string(),
                time_ms: 2,
                text: "hello".to_string(),
                slice_index: 1,
                slice_count: 2,
            },
            PromptSlice {
                delta_id: "pd_test_1".to_string(),
                slice_id: "ps_test_1_s002".to_string(),
                component_id: String::new(),
                prompt_type: "llm_response".to_string(),
                time_ms: 3,
                text: "HIDDEN".to_string(),
                slice_index: 2,
                slice_count: 2,
            },
            PromptSlice {
                delta_id: "pd_test_1".to_string(),
                slice_id: "ps_test_1_s003".to_string(),
                component_id: String::new(),
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
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
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
        "Please continue the work and respond as protocol requires in user's language:"
    ));
    assert!(!rendered.contains("one Markdown response with one state branch"));
}

#[test]
fn action_result_truncation_is_byte_safe_and_reports_omitted_words() {
    assert_eq!(MAX_ACTION_RESULT_PROMPT_BYTES, 32 * 1024);
    let input = format!(
        "{} alpha beta gamma",
        "x".repeat(MAX_ACTION_RESULT_PROMPT_BYTES - 1)
    );
    let truncated = truncate_action_result_for_prompt(&input);
    assert!(truncated.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
    assert!(truncated.ends_with("[Too long, 4 words truncated.  Issue new actions  if necessary]"));
    assert!(!truncated.ends_with('…'));

    let unicode_boundary = format!("{}界 alpha", "x".repeat(MAX_ACTION_RESULT_PROMPT_BYTES - 2));
    let unicode_truncated = truncate_action_result_for_prompt(&unicode_boundary);
    assert!(unicode_truncated.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
    assert!(unicode_truncated.is_char_boundary(unicode_truncated.len()));
    assert!(unicode_truncated
        .ends_with("[Too long, 2 words truncated.  Issue new actions  if necessary]"));
}

#[test]
fn prompt_renderer_defensively_truncates_legacy_action_result_slices() {
    let oversized = format!(
        "{} alpha beta",
        "x".repeat(MAX_ACTION_RESULT_PROMPT_BYTES - 1)
    );
    let delta = PromptDelta {
        delta_id: "pd_legacy_action".to_string(),
        time_ms: 1,
        hidden_slice_ids: Vec::new(),
        slices: vec![PromptSlice {
            delta_id: "pd_legacy_action".to_string(),
            slice_id: "ps_legacy_action_s001".to_string(),
            component_id: String::new(),
            prompt_type: "result_of_llm_action".to_string(),
            time_ms: 1,
            text: oversized,
            slice_index: 1,
            slice_count: 1,
        }],
    };
    let rendered = render_prompt_with_rendered_static(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]",
        &[delta],
        "Ai7",
        "",
    );
    assert!(rendered.contains("words truncated.  Issue new actions  if necessary]"));
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
        Some("Please continue the work and respond as protocol requires in user's language:")
    );
}

#[test]
fn formatted_response_trailer_is_protocol_neutral_and_does_not_repeat_the_shape() {
    assert_eq!(
        formatted_response_trailer("one-root label <response>...</response>", "Ai7"),
        "Please continue the work and respond as protocol requires in user's language:"
    );
    assert_eq!(
        formatted_response_trailer("one JSON object {...}", "Ai7"),
        "Please continue the work and respond as protocol requires in user's language:"
    );
    assert_eq!(
        formatted_response_trailer("one Markdown response with one state branch", "Ai7"),
        "Please continue the work and respond as protocol requires in user's language:"
    );
}

#[test]
fn formatted_response_trailer_parser_ignores_unrecognized_trailing_text() {
    let prompt = "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n\nunrecognized trailing text";
    let (prefix, trailer) = split_formatted_response_trailer(prompt);
    assert_eq!(prefix, prompt);
    assert_eq!(trailer, None);
}

#[test]
fn prompt_renderer_replaces_current_protocol_language() {
    let template = "Return {{CURRENT_PROTOCOL_LANG}}\n{{RESPONSE_PROTOCOL_SECTION}}";
    let markdown = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &MarkdownSuiteV1,
        "Ai7",
        "startup",
    );
    let json = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &JsonSuiteV1,
        "Ai7",
        "startup",
    );
    let xml = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &XmlSuiteV1,
        "Ai7",
        "startup",
    );

    assert!(markdown.contains("Return Markdown"));
    assert!(json.contains("Return JSON"));
    assert!(xml.contains("Return XML"));
    assert!(!markdown.contains("{{CURRENT_PROTOCOL_LANG}}"));
    assert!(!json.contains("{{CURRENT_PROTOCOL_LANG}}"));
    assert!(!xml.contains("{{CURRENT_PROTOCOL_LANG}}"));
}

#[test]
fn prompt_renderer_uses_protocol_native_tool_synopses() {
    let template =
        "# Tools\n\n{{TOOL_CATALOG}}\n\n{{SKILL_HEADERS}}\n\n{{RESPONSE_PROTOCOL_SECTION}}";
    let xml = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &XmlSuiteV1,
        "Ai7",
        "startup",
    );
    let json = render_static_prompt(
        template,
        &CapabilityRegistry::builtin(),
        &JsonSuiteV1,
        "Ai7",
        "startup",
    );

    assert!(xml.contains("`<readfile><path>src/main.rs</path>"), "{xml}");
    assert!(
        json.contains("`{\"readfile\":{\"path\":\"src/main.rs\""),
        "{json}"
    );
    assert!(!xml.contains("`{\"readfile\":"), "{xml}");
}

#[test]
fn prompt_renderer_replaces_assistant_id() {
    let rendered = render_static_prompt(
        "YOUR ID is: {{ASSSISTANT_ID}}\n## ASSSISTANT_ID",
        &CapabilityRegistry::builtin(),
        &MarkdownSuiteV1,
        "Ai7",
        "startup",
    );
    assert!(rendered.contains("YOUR ID is: Ai7"));
    assert!(rendered.contains("## Ai7"));
    assert!(!rendered.contains("{{ASSSISTANT_ID}}"));
    assert!(!rendered.contains("ASSSISTANT_ID"));
}

#[test]
fn prompt_renderer_replaces_startup_stamp() {
    let rendered = render_static_prompt(
        "## TIMESTAMP\n{{STARTUP_STAMP}}",
        &CapabilityRegistry::builtin(),
        &MarkdownSuiteV1,
        "Ai7",
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
    );

    assert!(rendered.contains("## TIMESTAMP\n2026-08-17 12:34:56 local_time, weekday=周一/Monday"));
    assert!(!rendered.contains("{{STARTUP_STAMP}}"));
}

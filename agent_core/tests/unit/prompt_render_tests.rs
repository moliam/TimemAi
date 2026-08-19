use super::*;
use crate::response_protocol::json_suite::JsonSuiteV1;
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
{{TOOL_CATALOG}}",
        &CapabilityRegistry::builtin(),
        &JsonSuiteV1,
        "TIMEM_ASSISTANT",
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
    );
    let rendered = render_prompt_with_rendered_static(
        &rendered_static,
        &[delta],
        "TIMEM_ASSISTANT",
        &JsonSuiteV1,
    );
    assert!(rendered.contains("Response Protocol"));
    assert!(rendered.contains("memmgr"));
    assert!(rendered.contains("hello"));
    assert!(rendered.contains("[BEGIN DELTA]"));
    assert!(rendered.contains("## USER"));
    assert!(rendered.contains("## RUNTIME"));
    assert!(!rendered.contains("## ACTIONS"));
    assert!(rendered.contains("The following are results of the actions generated in response:"));
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
fn xml_protocol_wraps_static_prompt_with_timem_system_prompt_boundary() {
    let rendered = render_static_prompt(
        "STATIC",
        &CapabilityRegistry::builtin(),
        &XmlSuiteV1,
        "TIMEM_ASSISTANT",
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
    );

    assert!(rendered.starts_with("<Timem System Prompt>\n"));
    assert!(rendered.ends_with("\n</Timem System Prompt>"));
    assert!(!rendered.contains("[BEGIN SYSTEM PROMPT]"));
    assert!(!rendered.contains("[END SYSTEM PROMPT]"));

    let json = render_static_prompt(
        "STATIC",
        &CapabilityRegistry::builtin(),
        &JsonSuiteV1,
        "TIMEM_ASSISTANT",
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
    );
    assert!(json.starts_with("[BEGIN SYSTEM PROMPT]\n"));
    assert!(json.ends_with("\n[END SYSTEM PROMPT]"));
    assert!(!json.contains("<Timem System Prompt>"));
}

#[test]
fn xml_protocol_uses_xml_style_prompt_delta_boundaries() {
    let delta = PromptDelta {
        delta_id: "pd_xml_14".to_string(),
        time_ms: 123,
        hidden_slice_ids: Vec::new(),
        slices: vec![PromptSlice {
            delta_id: "pd_xml_14".to_string(),
            slice_id: "ps_xml_14_s001".to_string(),
            component_id: String::new(),
            prompt_type: "user_question".to_string(),
            time_ms: 123,
            text: "hello".to_string(),
            slice_index: 1,
            slice_count: 1,
        }],
    };
    let rendered = render_prompt_with_rendered_static(
        "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]",
        &[delta],
        "TIMEM_ASSISTANT",
        &XmlSuiteV1,
    );

    assert!(rendered.contains("<prompt_delta id=\"pd_xml_14\" time_ms=\"123\">"));
    assert!(rendered.contains("</prompt_delta>"));
    assert!(!rendered.contains("[BEGIN DELTA]"));
    assert!(!rendered.contains("delta_id: pd_xml_14"));
}

#[test]
fn xml_action_result_preserves_name_escapes_xml_and_wraps_output_with_stable_id() {
    let rendered = render_xml_action_result(
        "readfile",
        Some(r#" check <diff> & "status" "#),
        "output <ready> & done",
        123,
    );
    let expected_id = action_output_id("output <ready> & done", 123);
    assert_eq!(expected_id.len(), 6);
    assert!(
        expected_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "output ID must contain exactly six lowercase hexadecimal digits"
    );
    assert_eq!(
        rendered,
        format!(
            "<action_result><readfile name=\"check &lt;diff&gt; &amp; &quot;status&quot;\"><output_id_{expected_id}>output &lt;ready&gt; &amp; done</output_id_{expected_id}></readfile></action_result>"
        )
    );

    let repeated = render_xml_action_result(
        "readfile",
        Some(r#" check <diff> & "status" "#),
        "output <ready> & done",
        123,
    );
    assert_eq!(repeated, rendered);

    let changed_time = render_xml_action_result("readfile", None, "output <ready> & done", 124);
    assert!(!changed_time.contains(&format!("output_id_{expected_id}")));

    let changed_output = render_xml_action_result("readfile", None, "different", 123);
    assert!(!changed_output.contains(&format!("output_id_{expected_id}")));

    let whitespace_variant =
        render_xml_action_result("readfile", None, " output <ready> & done ", 123);
    assert!(
        !whitespace_variant.contains(&format!("output_id_{expected_id}")),
        "the hash must use the original output bytes, even though display whitespace is trimmed"
    );

    let fallback = render_xml_action_result("readfile", None, "ok", 456);
    let fallback_id = action_output_id("ok", 456);
    assert_eq!(
        fallback,
        format!(
            "<action_result><readfile name=\"readfile\"><output_id_{fallback_id}>ok</output_id_{fallback_id}></readfile></action_result>"
        )
    );
}

#[test]
fn xml_bash_result_uses_single_dynamic_output_block_for_one_stream() {
    let evidence = BashResultEvidence {
        stdout: "On branch main\nmodified: src/<App>.tsx\n```bash\necho \"</bash_result>\"\n```"
            .to_string(),
        stderr: String::new(),
        exit_code: Some(0),
        signal: None,
        pid: None,
        error_type: None,
    };
    let rendered = render_xml_bash_result(
        Some(r#" check <tree> & "status" "#),
        ActionStatus::Completed,
        &evidence,
        123,
    );
    let id = bash_boundary_id(
        r#"check <tree> & "status""#,
        &evidence.stdout,
        &evidence.stderr,
        123,
    );

    assert_eq!(id.len(), 4);
    assert!(id
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert!(rendered.starts_with(
        r#"<bash_result task="check &lt;tree&gt; &amp; &quot;status&quot;" status="finished" exit_code="0">"#
    ));
    assert!(rendered.contains(&format!("<<<OUTPUT_{id}\n")));
    assert!(rendered.contains("src/<App>.tsx"));
    assert!(rendered.contains("echo \"</bash_result>\""));
    assert!(rendered.ends_with(&format!("OUTPUT_{id}\n</bash_result>")));
    assert_eq!(
        render_xml_bash_result(
            Some(r#" check <tree> & "status" "#),
            ActionStatus::Completed,
            &evidence,
            123,
        ),
        rendered
    );
}

#[test]
fn xml_bash_result_handles_empty_and_stderr_only_streams() {
    let empty_evidence = BashResultEvidence {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: Some(0),
        signal: None,
        pid: None,
        error_type: None,
    };
    let empty = render_xml_bash_result(
        Some("empty command"),
        ActionStatus::Completed,
        &empty_evidence,
        10,
    );
    let empty_id = bash_boundary_id("empty command", "", "", 10);
    assert_eq!(
        empty,
        format!(
            "<bash_result task=\"empty command\" status=\"finished\" exit_code=\"0\">\n<<<OUTPUT_{empty_id}\n\nOUTPUT_{empty_id}\n</bash_result>"
        )
    );

    let stderr_evidence = BashResultEvidence {
        stdout: String::new(),
        stderr: "fatal <message>\n".to_string(),
        exit_code: Some(2),
        signal: None,
        pid: None,
        error_type: None,
    };
    let stderr_only = render_xml_bash_result(
        Some("stderr only"),
        ActionStatus::Failed,
        &stderr_evidence,
        11,
    );
    let stderr_id = bash_boundary_id("stderr only", "", &stderr_evidence.stderr, 11);
    assert!(stderr_only
        .starts_with(r#"<bash_result task="stderr only" status="finished" exit_code="2">"#));
    assert!(stderr_only.contains(&format!(
        "<<<OUTPUT_{stderr_id}\nfatal <message>\nOUTPUT_{stderr_id}"
    )));
    assert!(!stderr_only.contains("<stderr>"));
}

#[test]
fn xml_bash_result_preserves_unicode_and_trims_only_trailing_stream_whitespace() {
    let evidence = BashResultEvidence {
        stdout: "开始\n界🙂\n\n".to_string(),
        stderr: String::new(),
        exit_code: Some(0),
        signal: None,
        pid: None,
        error_type: None,
    };
    let rendered = render_xml_bash_result(Some("unicode"), ActionStatus::Completed, &evidence, 12);
    let id = bash_boundary_id("unicode", &evidence.stdout, "", 12);

    assert!(rendered.contains(&format!("<<<OUTPUT_{id}\n开始\n界🙂\nOUTPUT_{id}")));
    assert!(rendered.is_char_boundary(rendered.len()));
    assert!(rendered.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
}

#[test]
fn xml_bash_result_uses_shared_dynamic_id_for_stdout_and_stderr() {
    let evidence = BashResultEvidence {
        stdout: "compiled".to_string(),
        stderr: "test failed".to_string(),
        exit_code: Some(1),
        signal: None,
        pid: None,
        error_type: None,
    };
    let rendered =
        render_xml_bash_result(Some("build and test"), ActionStatus::Failed, &evidence, 456);
    let id = bash_boundary_id("build and test", &evidence.stdout, &evidence.stderr, 456);

    assert!(rendered
        .starts_with(r#"<bash_result task="build and test" status="finished" exit_code="1">"#));
    assert!(rendered.contains(&format!(
        "<stdout>\n<<<OUT_{id}\ncompiled\nOUT_{id}\n</stdout>"
    )));
    assert!(rendered.contains(&format!(
        "<stderr>\n<<<ERR_{id}\ntest failed\nERR_{id}\n</stderr>"
    )));
}

#[test]
fn xml_bash_result_boundary_changes_for_time_content_task_and_marker_collision() {
    let first = bash_boundary_id("task one", "same", "", 100);
    assert_ne!(first, bash_boundary_id("task two", "same", "", 100));
    assert_ne!(first, bash_boundary_id("task one", "different", "", 100));
    assert_ne!(first, bash_boundary_id("task one", "same", "", 101));

    let colliding = format!("payload contains OUTPUT_{first}");
    let collision_safe = bash_boundary_id("task one", &colliding, "", 100);
    assert_ne!(collision_safe, first);
    let rendered = render_xml_bash_result(
        Some("task one"),
        ActionStatus::Completed,
        &BashResultEvidence {
            stdout: colliding,
            stderr: String::new(),
            exit_code: Some(0),
            signal: None,
            pid: None,
            error_type: None,
        },
        100,
    );
    assert!(rendered.contains(&format!("<<<OUTPUT_{collision_safe}")));
    assert!(rendered.ends_with(&format!("OUTPUT_{collision_safe}\n</bash_result>")));
}

#[test]
fn xml_bash_result_avoids_output_out_and_err_marker_collisions_in_either_stream() {
    let first = bash_boundary_id("collision task", "seed", "error", 77);
    for payload in [
        format!("OUTPUT_{first}"),
        format!("OUT_{first}"),
        format!("ERR_{first}"),
    ] {
        let next = bash_boundary_id("collision task", &payload, "error", 77);
        assert_ne!(next, first);
        let rendered = render_xml_bash_result(
            Some("collision task"),
            ActionStatus::Failed,
            &BashResultEvidence {
                stdout: payload,
                stderr: "error".to_string(),
                exit_code: Some(1),
                signal: None,
                pid: None,
                error_type: None,
            },
            77,
        );
        assert!(rendered.contains(&format!("<<<OUT_{next}")));
        assert!(rendered.contains(&format!("<<<ERR_{next}")));
        assert!(rendered.ends_with("</bash_result>"));
    }
}

#[test]
fn xml_bash_result_single_stream_budget_boundary_stays_complete() {
    let evidence = BashResultEvidence {
        stdout: "界".repeat(MAX_ACTION_RESULT_PROMPT_BYTES),
        stderr: String::new(),
        exit_code: Some(0),
        signal: None,
        pid: None,
        error_type: None,
    };
    let rendered = render_xml_bash_result(
        Some("large unicode"),
        ActionStatus::Completed,
        &evidence,
        88,
    );
    let id = bash_boundary_id("large unicode", &evidence.stdout, "", 88);

    assert!(rendered.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
    assert!(rendered.is_char_boundary(rendered.len()));
    assert!(rendered
        .starts_with(r#"<bash_result task="large unicode" status="finished" exit_code="0">"#));
    assert!(rendered.contains(&format!("<<<OUTPUT_{id}\n")));
    assert!(rendered.ends_with(&format!("OUTPUT_{id}\n</bash_result>")));
    assert!(rendered.contains("words truncated. Generate more actions if necessary !!!"));
}

#[test]
fn oversized_xml_bash_result_keeps_stream_tags_and_markers_complete() {
    let evidence = BashResultEvidence {
        stdout: format!(
            "{} stdout tail words",
            "x".repeat(MAX_ACTION_RESULT_PROMPT_BYTES)
        ),
        stderr: format!(
            "{} stderr tail words",
            "y".repeat(MAX_ACTION_RESULT_PROMPT_BYTES)
        ),
        exit_code: Some(1),
        signal: None,
        pid: None,
        error_type: None,
    };
    let rendered =
        render_xml_bash_result(Some("large build"), ActionStatus::Failed, &evidence, 789);
    let id = bash_boundary_id("large build", &evidence.stdout, &evidence.stderr, 789);

    assert!(rendered.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
    assert!(
        rendered.starts_with(r#"<bash_result task="large build" status="finished" exit_code="1">"#)
    );
    assert!(rendered.contains(&format!("<stdout>\n<<<OUT_{id}\n")));
    assert!(rendered.contains(&format!("\nOUT_{id}\n</stdout>")));
    assert!(rendered.contains(&format!("<stderr>\n<<<ERR_{id}\n")));
    assert!(rendered.contains(&format!("\nERR_{id}\n</stderr>")));
    assert!(rendered.ends_with("</bash_result>"));
    assert!(rendered.contains("words truncated. Generate more actions if necessary !!!"));
}

#[test]
fn xml_bash_result_renders_running_timeout_cancelled_and_signal_metadata() {
    let running = render_xml_bash_result(
        Some("background server"),
        ActionStatus::BackgroundRunning,
        &BashResultEvidence {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            signal: None,
            pid: Some(4321),
            error_type: None,
        },
        1,
    );
    assert!(running
        .starts_with(r#"<bash_result task="background server" status="running" pid="4321">"#));

    let timeout = render_xml_bash_result(
        Some("slow command"),
        ActionStatus::Timeout,
        &BashResultEvidence {
            stdout: "partial".to_string(),
            stderr: String::new(),
            exit_code: None,
            signal: None,
            pid: Some(9876),
            error_type: None,
        },
        2,
    );
    assert!(timeout.starts_with(r#"<bash_result task="slow command" status="timeout" pid="9876">"#));

    let cancelled = render_xml_bash_result(
        Some("cancel command"),
        ActionStatus::Cancelled,
        &BashResultEvidence {
            stdout: String::new(),
            stderr: "cancelled".to_string(),
            exit_code: None,
            signal: None,
            pid: None,
            error_type: Some("Cancelled".to_string()),
        },
        3,
    );
    assert!(cancelled.starts_with(
        r#"<bash_result task="cancel command" status="finished" error_type="Cancelled">"#
    ));

    let signal = render_xml_bash_result(
        Some("crash command"),
        ActionStatus::Failed,
        &BashResultEvidence {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            signal: Some(11),
            pid: None,
            error_type: None,
        },
        4,
    );
    assert!(
        signal.starts_with(r#"<bash_result task="crash command" status="finished" signal="11">"#)
    );
}

#[test]
fn xml_bash_result_escapes_error_type_without_changing_finished_lifecycle() {
    let rendered = render_xml_bash_result(
        Some("invalid input"),
        ActionStatus::Failed,
        &BashResultEvidence {
            stdout: String::new(),
            stderr: "not executed".to_string(),
            exit_code: None,
            signal: None,
            pid: None,
            error_type: Some(r#"Invalid<&"'"#.to_string()),
        },
        5,
    );

    assert!(rendered.starts_with(
        r#"<bash_result task="invalid input" status="finished" error_type="Invalid&lt;&amp;&quot;&apos;">"#
    ));
    assert!(!rendered.contains(r#"status="error""#));
    assert!(!rendered.contains(r#"status="cancelled""#));
}

#[test]
fn oversized_xml_action_result_is_truncated_inside_output_id_envelope() {
    let result = format!(
        "{}界 <tag> & tail words",
        "&".repeat(MAX_ACTION_RESULT_PROMPT_BYTES)
    );
    let rendered = render_xml_action_result(
        "readfile",
        Some(r#"inspect <large> & "escaped" output"#),
        &result,
        789,
    );
    let output_id = action_output_id(&result, 789);

    assert!(rendered.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
    assert!(rendered.starts_with(&format!(
        r#"<action_result><readfile name="inspect &lt;large&gt; &amp; &quot;escaped&quot; output"><output_id_{output_id}>"#
    )));
    assert!(rendered.ends_with(&format!(
        "</output_id_{output_id}></readfile></action_result>"
    )));
    assert!(rendered.contains("words truncated. Generate more actions if necessary !!!"));
    assert!(!rendered.contains("<tag>"));
    assert!(!rendered.contains(" & tail"));
    assert_eq!(
        truncate_action_result_for_prompt(&rendered),
        rendered,
        "generic action-result truncation must not cut the XML/output-ID wrapper"
    );
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
    assert!(truncated
        .ends_with("!!!Too long, 4 words truncated. Generate more actions if necessary !!!"));
    assert!(!truncated.ends_with('…'));

    let unicode_boundary = format!("{}界 alpha", "x".repeat(MAX_ACTION_RESULT_PROMPT_BYTES - 2));
    let unicode_truncated = truncate_action_result_for_prompt(&unicode_boundary);
    assert!(unicode_truncated.len() <= MAX_ACTION_RESULT_PROMPT_BYTES);
    assert!(unicode_truncated.is_char_boundary(unicode_truncated.len()));
    assert!(unicode_truncated
        .ends_with("!!!Too long, 2 words truncated. Generate more actions if necessary !!!"));
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
        &JsonSuiteV1,
    );
    assert!(rendered.contains("words truncated. Generate more actions if necessary !!!"));
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
        formatted_response_trailer("one JSON object {...}", "Ai7"),
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

    assert!(json.contains("Return JSON"));
    assert!(xml.contains("Return XML"));
    assert!(!json.contains("{{CURRENT_PROTOCOL_LANG}}"));
    assert!(!xml.contains("{{CURRENT_PROTOCOL_LANG}}"));
}

#[test]
fn prompt_renderer_injects_only_the_active_protocol_delta_example() {
    let template = "{{PROMPT_DELTA_EXAMPLE}}";
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

    assert!(json.contains("[BEGIN DELTA]\ndelta_id: pd_1\ntime: 123"));
    assert!(json.contains("[END DELTA]"));
    assert!(!json.contains("<prompt_delta "));
    assert!(!json.contains("</prompt_delta>"));
    assert!(json.contains("## USER"));
    assert!(json.contains("## Ai7"));
    assert!(json.contains("## RUNTIME"));
    assert!(json.contains("RUNTIME's 'TIPS'"));
    assert!(!json.contains("## SYSTEM"));
    assert!(!json.contains("SYSTEM's 'TIPS'"));

    assert!(xml.contains(r#"<prompt_delta id="pd_1" time_ms="123">"#));
    assert!(xml.contains("</prompt_delta>"));
    assert!(!xml.contains("[BEGIN DELTA]"));
    assert!(!xml.contains("[END DELTA]"));
    assert!(!xml.contains("delta_id: pd_1"));
    assert!(xml.contains("## USER"));
    assert!(xml.contains("## Ai7"));
    assert!(xml.contains("## RUNTIME"));
    assert!(xml.contains("RUNTIME's 'TIPS'"));
    assert!(!xml.contains("## SYSTEM"));
    assert!(!xml.contains("SYSTEM's 'TIPS'"));

    assert!(!json.contains("{{PROMPT_DELTA_EXAMPLE}}"));
    assert!(!xml.contains("{{PROMPT_DELTA_EXAMPLE}}"));
}

#[test]
fn prompt_renderer_uses_protocol_native_tool_synopses() {
    let template = "# Tools\n\n{{TOOL_CATALOG}}\n\n{{RESPONSE_PROTOCOL_SECTION}}";
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
        &JsonSuiteV1,
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
        &JsonSuiteV1,
        "Ai7",
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
    );

    assert!(rendered.contains("## TIMESTAMP\n2026-08-17 12:34:56 local_time, weekday=周一/Monday"));
    assert!(!rendered.contains("{{STARTUP_STAMP}}"));
}

#[test]
fn prompt_serialization_is_byte_stable_and_append_only_before_trailer() {
    fn delta(id: &str, time_ms: i64, prompt_type: &str, text: &str) -> PromptDelta {
        PromptDelta {
            delta_id: id.to_string(),
            time_ms,
            hidden_slice_ids: Vec::new(),
            slices: vec![PromptSlice {
                delta_id: id.to_string(),
                slice_id: format!("ps_{}_s001", id.trim_start_matches("pd_")),
                component_id: format!("component_{id}"),
                prompt_type: prompt_type.to_string(),
                time_ms,
                text: text.to_string(),
                slice_index: 1,
                slice_count: 1,
            }],
        }
    }

    let rendered_static = render_static_prompt(
        "STATIC",
        &CapabilityRegistry::builtin(),
        &XmlSuiteV1,
        "TIMEM_ASSISTANT",
        "2026-08-17 12:34:56 local_time, weekday=周一/Monday",
    );
    let first_deltas = vec![
        delta("pd_1", 100, "user_question", "first question"),
        delta("pd_2", 200, "llm_response", "first answer"),
    ];

    let first = render_prompt_with_rendered_static(
        &rendered_static,
        &first_deltas,
        "TIMEM_ASSISTANT",
        &XmlSuiteV1,
    );
    let repeated = render_prompt_with_rendered_static(
        &rendered_static,
        &first_deltas,
        "TIMEM_ASSISTANT",
        &XmlSuiteV1,
    );
    assert_eq!(
        first, repeated,
        "serializing unchanged structured context must be byte stable"
    );

    let mut appended_deltas = first_deltas;
    appended_deltas.push(delta("pd_3", 300, "user_supplement", "second question"));
    let appended = render_prompt_with_rendered_static(
        &rendered_static,
        &appended_deltas,
        "TIMEM_ASSISTANT",
        &XmlSuiteV1,
    );

    let (first_prefix, first_trailer) = split_formatted_response_trailer(&first);
    let (appended_prefix, appended_trailer) = split_formatted_response_trailer(&appended);
    assert_eq!(first_trailer.as_deref(), Some(RESPONSE_TRAILER));
    assert_eq!(appended_trailer.as_deref(), Some(RESPONSE_TRAILER));
    assert!(
        appended_prefix.starts_with(first_prefix),
        "without context maintenance, protocol/identity changes, or static refresh, \
         previously rendered bytes must remain an exact prefix"
    );
    assert!(appended_prefix.contains("pd_3"));
    assert!(appended_prefix.contains("second question"));
}

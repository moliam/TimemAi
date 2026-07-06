use crate::capability::CapabilityRegistry;
use crate::prompt_spec;
use crate::response_protocol::ResponseProtocolSuite;
use crate::{PromptDelta, PromptSlice};

pub(crate) fn formatted_response_trailer(protocol_language: &str) -> String {
    format!(
        "Follow the system prompt, give your {} formatted response:",
        protocol_language.trim()
    )
}

pub(crate) fn split_formatted_response_trailer(rendered_prompt: &str) -> (&str, Option<String>) {
    let trimmed = rendered_prompt.trim_end();
    let Some(line_start) = trimmed.rfind('\n') else {
        return (rendered_prompt, None);
    };
    let candidate = trimmed[line_start + 1..].trim();
    if candidate.starts_with("Follow the system prompt, give your ")
        && candidate.ends_with(" formatted response:")
        && candidate.len() > "Follow the system prompt, give your  formatted response:".len()
    {
        let prefix = trimmed[..line_start].trim_end();
        return (prefix, Some(candidate.to_string()));
    }
    (rendered_prompt, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisiblePromptRole {
    User,
    You,
    System,
}

impl VisiblePromptRole {
    fn heading(self, assistant_heading: &str) -> String {
        match self {
            VisiblePromptRole::User => "USER".to_string(),
            VisiblePromptRole::You => assistant_heading.to_string(),
            VisiblePromptRole::System => "SYSTEM".to_string(),
        }
    }
}

fn visible_role(prompt_type: &str) -> VisiblePromptRole {
    match prompt_type {
        "user_question" | "user_supplement" => VisiblePromptRole::User,
        "llm_response" | "llm_free_talk" => VisiblePromptRole::You,
        "result_of_llm_action" | "response_repair" | "context_compacted" => {
            VisiblePromptRole::System
        }
        _ => VisiblePromptRole::System,
    }
}

pub(crate) fn render_static_prompt(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
) -> String {
    // 1. Fill {{RESPONSE_PROTOCOL_SECTION}} from protocol suite
    let with_protocol = static_prompt.replace(
        "{{RESPONSE_PROTOCOL_SECTION}}",
        &protocol_suite.protocol_prompt_section(),
    );
    let with_protocol =
        with_protocol.replace("{{CURRENT_PROTOCOL_LANG}}", protocol_suite.lang_format());
    // 2. Fill {{TOOL_CATALOG}} and {{SKILL_HEADERS}} from capabilities
    let with_caps = capabilities.enrich_static_prompt(&with_protocol);
    // 3. Fill {{RESPONSE_V1_SCHEMA}} from prompt_spec
    let static_prompt = prompt_spec::enrich_static_prompt_with_response_schema(
        &with_caps,
        protocol_suite.response_schema_summary(),
    );

    format!(
        "[BEGIN SYSTEM PROMPT]\n{}\n[END SYSTEM PROMPT]",
        static_prompt
    )
}

pub(crate) fn render_prompt_with_rendered_static(
    rendered_static_prompt: &str,
    deltas: &[PromptDelta],
    assistant_heading: &str,
    protocol_language: &str,
) -> String {
    let mut out = format!("{}", rendered_static_prompt);

    for delta in deltas {
        let slices = render_delta_slices(delta);
        if slices.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str("[BEGIN DELTA]\n");
        out.push_str(&format!(
            "delta_id: {}\ntime: {}\n",
            delta.delta_id, delta.time_ms
        ));
        let mut last_role = None;
        for slice in slices {
            let role = visible_role(&slice.prompt_type);
            if last_role != Some(role) {
                out.push('\n');
                out.push_str(&format!("## {}\n", role.heading(assistant_heading)));
                last_role = Some(role);
            }
            out.push('\n');
            out.push_str(slice.text.trim());
            out.push('\n');
        }
        out.push_str("\n[END DELTA]");
    }

    out.push_str("\n\n");
    out.push_str(&formatted_response_trailer(protocol_language));
    out
}

pub(crate) fn render_prompt_slices(deltas: &[PromptDelta]) -> Vec<PromptSlice> {
    deltas
        .iter()
        .flat_map(render_delta_slices)
        .collect::<Vec<_>>()
}

pub(crate) fn render_delta_slices(delta: &PromptDelta) -> Vec<PromptSlice> {
    delta
        .slices
        .iter()
        .filter(|slice| !delta.hidden_slice_ids.contains(&slice.slice_id))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
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
        );
        let rendered = render_prompt_with_rendered_static(
            &rendered_static,
            &[delta],
            "TIMEM_ASSISTANT",
            "Markdown",
        );
        assert!(rendered.contains("Response Protocol"));
        assert!(rendered.contains("memmgr"));
        assert!(rendered.contains("hello"));
        assert!(rendered.contains("[BEGIN DELTA]"));
        assert!(rendered.contains("## USER"));
        assert!(rendered.contains("## SYSTEM"));
        assert!(!rendered.contains("## ACTIONS"));
        assert!(rendered.contains("Action result: run_bash"));
        assert!(!rendered.contains("slice_id:"));
        assert!(!rendered.contains("prompt_type:"));
        assert!(!rendered.contains("HIDDEN"));
        assert!(
            rendered.ends_with("Follow the system prompt, give your Markdown formatted response:")
        );
    }

    #[test]
    fn formatted_response_trailer_parser_preserves_protocol_name() {
        let prompt =
            "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]\n\nFollow the system prompt, give your XML formatted response:";
        let (prefix, trailer) = split_formatted_response_trailer(prompt);
        assert_eq!(prefix, "[BEGIN SYSTEM PROMPT]\nSTATIC\n[END SYSTEM PROMPT]");
        assert_eq!(
            trailer.as_deref(),
            Some("Follow the system prompt, give your XML formatted response:")
        );
    }

    #[test]
    fn prompt_renderer_replaces_current_protocol_language() {
        let template = "Return {{CURRENT_PROTOCOL_LANG}}\n{{RESPONSE_PROTOCOL_SECTION}}";
        let markdown =
            render_static_prompt(template, &CapabilityRegistry::builtin(), &MarkdownSuiteV1);
        let json = render_static_prompt(template, &CapabilityRegistry::builtin(), &JsonSuiteV1);
        let xml = render_static_prompt(template, &CapabilityRegistry::builtin(), &XmlSuiteV1);

        assert!(markdown.contains("Return Markdown"));
        assert!(json.contains("Return JSON"));
        assert!(xml.contains("Return XML"));
        assert!(!markdown.contains("{{CURRENT_PROTOCOL_LANG}}"));
        assert!(!json.contains("{{CURRENT_PROTOCOL_LANG}}"));
        assert!(!xml.contains("{{CURRENT_PROTOCOL_LANG}}"));
    }
}

use crate::capability::CapabilityRegistry;
use crate::prompt_spec;
use crate::response_protocol::ResponseProtocolSuite;
use crate::{PromptDelta, PromptSlice};

pub(crate) fn formatted_response_trailer(
    protocol_language: &str,
    assistant_heading: &str,
) -> String {
    let instruction = if protocol_language.trim().eq_ignore_ascii_case("XML") {
        "Now please continue your ID's response part in XML as required in protocol:"
    } else {
        "Now please continue your ID's response part as required in protocol:"
    };
    format!("{instruction}\n## {}", assistant_heading.trim())
}

pub(crate) fn split_formatted_response_trailer(rendered_prompt: &str) -> (&str, Option<String>) {
    let trimmed = rendered_prompt.trim_end();
    let Some(trailer_start) = [
        "\n\nNow please continue your ID's response part as required in protocol:\n## ",
        "\n\nNow please continue your ID's response part in XML as required in protocol:\n## ",
    ]
    .into_iter()
    .filter_map(|prefix| trimmed.rfind(prefix))
    .max() else {
        return (rendered_prompt, None);
    };
    let candidate = &trimmed[trailer_start + 2..];
    let assistant_heading = candidate
        .split_once("\n## ")
        .map(|(_, heading)| heading)
        .unwrap_or_default();
    if !assistant_heading.is_empty() && !assistant_heading.contains('\n') {
        let prefix = trimmed[..trailer_start].trim_end();
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

fn is_action_result_prompt_type(prompt_type: &str) -> bool {
    prompt_type == "result_of_llm_action"
}

pub(crate) fn render_static_prompt(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    protocol_suite: &dyn ResponseProtocolSuite,
    assistant_heading: &str,
) -> String {
    // 1. Fill {{RESPONSE_PROTOCOL_SECTION}} from protocol suite
    let with_protocol = static_prompt.replace(
        "{{RESPONSE_PROTOCOL_SECTION}}",
        &protocol_suite.protocol_prompt_section(),
    );
    let with_protocol =
        with_protocol.replace("{{CURRENT_PROTOCOL_LANG}}", protocol_suite.lang_format());
    let assistant_heading = assistant_heading.trim();
    let with_protocol = with_protocol.replace("{{ASSSISTANT_ID}}", assistant_heading);
    let with_protocol = with_protocol.replace("ASSSISTANT_ID", assistant_heading);
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
    let mut out = rendered_static_prompt.to_string();

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
        let mut last_was_action_result = false;
        for slice in slices {
            let role = visible_role(&slice.prompt_type);
            if last_role != Some(role) {
                out.push('\n');
                out.push_str(&format!("## {}\n", role.heading(assistant_heading)));
                last_role = Some(role);
                last_was_action_result = false;
            }
            let is_action_result = is_action_result_prompt_type(&slice.prompt_type);
            if is_action_result && !last_was_action_result {
                out.push('\n');
                out.push_str(&format!(
                    "The following are results of {} newly initiated actions:\n",
                    assistant_heading
                ));
            }
            out.push('\n');
            out.push_str(slice.text.trim());
            out.push('\n');
            last_was_action_result = is_action_result;
        }
        out.push_str("\n[END DELTA]");
    }

    out.push_str("\n\n");
    out.push_str(&formatted_response_trailer(
        protocol_language,
        assistant_heading,
    ));
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
#[path = "../tests/unit/prompt_render_tests.rs"]
mod tests;

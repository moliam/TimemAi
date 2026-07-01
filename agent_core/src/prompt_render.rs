use crate::capability::CapabilityRegistry;
use crate::prompt_spec;
use crate::{PromptDelta, PromptSlice};

pub(crate) fn render_prompt(
    static_prompt: &str,
    capabilities: &CapabilityRegistry,
    deltas: &[PromptDelta],
) -> String {
    let static_prompt = prompt_spec::enrich_static_prompt_with_response_schema(
        &capabilities.enrich_static_prompt(static_prompt),
    );
    let mut out = format!(
        "[BEGIN SEGMENT 0: prompt_0]\n{}\n[END SEGMENT 0: prompt_0]",
        static_prompt
    );
    let slices = render_prompt_slices(deltas);
    for (idx, slice) in slices.iter().enumerate() {
        out.push('\n');
        out.push_str(&format!("[BEGIN SEGMENT {}: prompt_delta]\n", idx + 1));
        out.push_str(&format!(
            "delta_id: {}\nslice_id: {}\nslice: {}/{}\n",
            slice.delta_id, slice.slice_id, slice.slice_index, slice.slice_count
        ));
        out.push_str(&format!(
            "prompt_type: {}\n{}\ntime: {}\n",
            slice.prompt_type, slice.text, slice.time_ms
        ));
        out.push_str(&format!("[END SEGMENT {}: prompt_delta]", idx + 1));
    }
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

    #[test]
    fn prompt_renderer_injects_schema_catalog_and_visible_slices() {
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
                    text: "UNIQUE_HIDDEN_SLICE_SENTINEL".to_string(),
                    slice_index: 2,
                    slice_count: 2,
                },
            ],
        };

        let rendered = render_prompt(
            "## Response Protocol\n{{RESPONSE_V1_SCHEMA}}\n## Tools\n{{TOOL_CATALOG}}\n{{SKILL_HEADERS}}",
            &CapabilityRegistry::builtin(),
            &[delta],
        );

        assert!(rendered.contains("[BEGIN SEGMENT 0: prompt_0]"));
        assert!(!rendered.contains("\"$id\""));
        assert!(rendered.contains("\"fields\""));
        assert!(rendered.contains("\"status?\""));
        assert!(rendered.contains("#### `memmgr`"));
        assert!(rendered.contains("**Options**"));
        assert!(!rendered.contains("\"tool_catalog\""));
        assert!(rendered.contains("delta_id: pd_test_1"));
        assert!(rendered.contains("hello"));
        assert!(!rendered.contains("UNIQUE_HIDDEN_SLICE_SENTINEL"));
    }
}

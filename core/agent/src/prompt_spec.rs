use serde_json::Value;

pub fn response_v1_summary_value(response_schema_summary: &str) -> Value {
    serde_json::from_str(response_schema_summary)
        .expect("response_v1 summary resource must be valid JSON")
}

pub fn enrich_static_prompt_with_response_schema(
    static_prompt: &str,
    response_schema_summary: &str,
) -> String {
    if static_prompt.contains("{{RESPONSE_V1_SCHEMA}}") {
        return static_prompt.replace("{{RESPONSE_V1_SCHEMA}}", response_schema_summary);
    }
    static_prompt.to_string()
}

pub(crate) fn replace_markdown_placeholder_with_text(
    source: &str,
    placeholder: &str,
    replacement: &str,
) -> Option<String> {
    source
        .contains(placeholder)
        .then(|| source.replace(placeholder, replacement))
}

#[cfg(test)]
#[path = "../tests/unit/prompt_spec_tests.rs"]
mod tests;

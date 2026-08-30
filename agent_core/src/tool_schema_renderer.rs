use serde_json::{Map, Value};

/// Provider-specific JSON Schema dialect used only for model-facing tool definitions.
/// The capability registry and executor retain and validate the original schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolSchemaDialect {
    OpenAi,
    AnthropicBedrock,
}

pub(crate) fn render_tool_input_schema(schema: &Value, dialect: ToolSchemaDialect) -> Value {
    match dialect {
        ToolSchemaDialect::OpenAi => schema.clone(),
        ToolSchemaDialect::AnthropicBedrock => render_anthropic_bedrock_schema(schema),
    }
}

fn render_anthropic_bedrock_schema(schema: &Value) -> Value {
    let Some(source) = schema.as_object() else {
        return empty_object_schema();
    };

    let mut rendered = source.clone();
    let removed = ["oneOf", "allOf", "anyOf"]
        .into_iter()
        .filter_map(|keyword| rendered.remove(keyword).map(|value| (keyword, value)))
        .collect::<Vec<_>>();

    // Bedrock's Anthropic tool API rejects these composition keywords at the
    // input_schema root. The executor still validates the original schema.
    if !removed.is_empty() {
        append_validation_hint(&mut rendered, &removed);
    }
    rendered
        .entry("type".to_string())
        .or_insert_with(|| Value::String("object".to_string()));
    rendered
        .entry("properties".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    Value::Object(rendered)
}

fn empty_object_schema() -> Value {
    Value::Object(Map::from_iter([
        ("type".to_string(), Value::String("object".to_string())),
        ("properties".to_string(), Value::Object(Map::new())),
    ]))
}

fn append_validation_hint(rendered: &mut Map<String, Value>, removed: &[(&'static str, Value)]) {
    let keywords = removed
        .iter()
        .map(|(keyword, _)| *keyword)
        .collect::<Vec<_>>()
        .join(", ");
    let hint = format!(
        "Additional argument-combination constraints ({keywords}) are enforced after the tool call. Follow field descriptions and provide only arguments needed for the selected operation."
    );
    match rendered.get_mut("description") {
        Some(Value::String(description)) if !description.trim().is_empty() => {
            description.push_str("\n\n");
            description.push_str(&hint);
        }
        _ => {
            rendered.insert("description".to_string(), Value::String(hint));
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/tool_schema_renderer_tests.rs"]
mod tests;

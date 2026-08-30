use super::*;
use serde_json::json;

#[test]
fn openai_preserves_the_original_schema_exactly() {
    let schema = json!({
        "type": "object",
        "properties": {"cmd": {"type": "string"}},
        "oneOf": [{"required": ["cmd"]}, {"required": ["loop_cmd"]}],
        "allOf": [{"if": {"required": ["cmd"]}, "then": {"required": ["timeout_ms"]}}],
        "additionalProperties": false
    });
    assert_eq!(
        render_tool_input_schema(&schema, ToolSchemaDialect::OpenAi),
        schema
    );
}

#[test]
fn anthropic_bedrock_removes_only_unsupported_root_composition() {
    let schema = json!({
        "type": "object",
        "description": "Run one command mode.",
        "properties": {
            "cmd": {"type": "string"},
            "selector": {"oneOf": [{"type": "string"}, {"type": "number"}]}
        },
        "required": ["selector"],
        "oneOf": [{"required": ["cmd"]}, {"required": ["loop_cmd"]}],
        "allOf": [{"if": {"required": ["cmd"]}, "then": {"required": ["timeout_ms"]}}],
        "additionalProperties": false
    });
    let rendered = render_tool_input_schema(&schema, ToolSchemaDialect::AnthropicBedrock);

    assert!(rendered.get("oneOf").is_none());
    assert!(rendered.get("allOf").is_none());
    assert!(rendered.get("anyOf").is_none());
    assert_eq!(rendered["type"], "object");
    assert_eq!(rendered["required"], json!(["selector"]));
    assert_eq!(rendered["additionalProperties"], false);
    assert!(rendered["properties"]["selector"].get("oneOf").is_some());
    let description = rendered["description"].as_str().unwrap();
    assert!(description.starts_with("Run one command mode."));
    assert!(description.contains("oneOf, allOf"));
    assert!(description.contains("enforced after the tool call"));
}

#[test]
fn anthropic_bedrock_output_is_stable_and_object_shaped() {
    let schema = json!({"anyOf": [{"required": ["left"]}, {"required": ["right"]}]});
    let first = render_tool_input_schema(&schema, ToolSchemaDialect::AnthropicBedrock);
    let second = render_tool_input_schema(&schema, ToolSchemaDialect::AnthropicBedrock);
    assert_eq!(first, second);
    assert_eq!(first["type"], "object");
    assert_eq!(first["properties"], json!({}));
    assert!(first.get("anyOf").is_none());
}

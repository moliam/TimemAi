use super::*;

#[test]
fn optimizer_is_recursive_and_conservative() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "nested": {
                "type": "object",
                "oneOf": [
                    {"required": ["left"]},
                    {"required": ["right"]}
                ],
                "allOf": [
                    {"anyOf": [{"required": ["left"]}, {"required": ["right"]}]},
                    {"required": ["kept"]},
                    {"required": ["kept"]}
                ]
            }
        }
    });

    let optimized = optimize_provider_schema(schema);
    assert_eq!(
        optimized["properties"]["nested"]["allOf"],
        serde_json::json!([{"required": ["kept"]}])
    );

    let incomplete = serde_json::json!({
        "oneOf": [
            {"required": ["left"]},
            {"properties": {"right": {"type": "string"}}}
        ],
        "allOf": [
            {"anyOf": [{"required": ["left"]}, {"required": ["right"]}]}
        ]
    });
    assert!(optimize_provider_schema(incomplete).get("allOf").is_some());
}

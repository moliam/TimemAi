//! Payload mappings only. Capability and scheduling policy belong upstream.
use crate::reasoning::EffectiveReasoning;
use crate::{ApiProtocol, ModelServiceConfig};
use serde_json::{json, Value};

fn adapter(config: &ModelServiceConfig) -> Option<crate::model_catalog::ReasoningAdapter> {
    crate::model_requirements::capabilities(config)
        .and_then(|model| {
            model
                .protocols
                .iter()
                .find(|p| p.protocol == config.api_protocol.label())
        })
        .map(|profile| profile.reasoning_adapter)
}

pub(crate) fn apply_reasoning(
    config: &ModelServiceConfig,
    body: &mut Value,
    reasoning: &EffectiveReasoning,
) {
    if adapter(config) == Some(crate::model_catalog::ReasoningAdapter::ZhipuChatReasoning) {
        let default = crate::model_requirements::capabilities(config)
            .map(|m| m.default_effort.as_str())
            .unwrap_or("max");
        let effort = match reasoning {
            EffectiveReasoning::Disabled => "none",
            EffectiveReasoning::Enabled {
                intensity: Some(value),
            } => value,
            _ => default,
        };
        let object = body.as_object_mut().expect("request object");
        object.remove("enable_thinking");
        object.remove("stream_options");
        object.insert("reasoning_effort".into(), json!(effort));
        object.insert("thinking".into(), json!({"type": if effort == "none" { "disabled" } else { "enabled" }, "clear_thinking": true}));
    } else {
        let official = crate::model_requirements::provider(config) == Some("openai");
        apply_legacy_reasoning_wire(
            body,
            config.api_protocol,
            reasoning,
            !official && config.openai_compatible.enable_thinking.is_some(),
        );
        if official {
            body.as_object_mut().unwrap().remove("enable_thinking");
        }
    }
}

/// Protocol translation only: policy is resolved once before this boundary.
fn apply_legacy_reasoning_wire(
    body: &mut Value,
    protocol: ApiProtocol,
    reasoning: &EffectiveReasoning,
    chat_thinking_extension: bool,
) {
    let (enabled, intensity) = match reasoning {
        EffectiveReasoning::Unspecified => return,
        EffectiveReasoning::Disabled => (false, None),
        EffectiveReasoning::Enabled { intensity } => (true, intensity.as_deref()),
    };
    match protocol {
        ApiProtocol::OpenAiCompatible => {
            if chat_thinking_extension {
                body["enable_thinking"] = json!(enabled);
            }
            if !enabled {
                body["reasoning_effort"] = json!("none");
            } else if let Some(intensity) = intensity {
                body["reasoning_effort"] = json!(intensity);
            } else if !chat_thinking_extension {
                // Explicit enable without a strength uses the provider default.
                body["enable_thinking"] = json!(true);
            }
        }
        ApiProtocol::OpenAiResponses => {
            body["reasoning"] = if !enabled {
                json!({"effort": "none"})
            } else if let Some(intensity) = intensity {
                json!({"effort": intensity})
            } else {
                json!({})
            };
        }
        ApiProtocol::Anthropic => {
            body["thinking"] = json!({"type": if enabled { "adaptive" } else { "disabled" }});
            if enabled {
                if let Some(intensity) = intensity {
                    body["output_config"] = json!({"effort": intensity});
                }
            }
        }
    }
}

/// Compare the final wire values to the resolved demand, not merely an enum range.
pub(crate) fn validate_request(
    config: &ModelServiceConfig,
    body: &Value,
    critical: bool,
) -> Result<(), String> {
    crate::model_requirements::validate_config(config)?;
    let reasoning = crate::model_requirements::resolve_reasoning(config, critical);
    let mut expected = json!({});
    apply_reasoning(config, &mut expected, &reasoning);
    for key in [
        "reasoning_effort",
        "reasoning",
        "thinking",
        "enable_thinking",
        "output_config",
    ] {
        if body.get(key) != expected.get(key) {
            return Err(format!("model_demand_payload_mismatch:{key}"));
        }
    }
    if let Some(allowed) = &config.openai_compatible.requirements.allowed_reasoning {
        if let Some(level) = body
            .get("reasoning_effort")
            .or_else(|| body.pointer("/reasoning/effort"))
            .and_then(Value::as_str)
        {
            if !allowed.iter().any(|v| v == level) {
                return Err("request_reasoning_not_in_allowed_set".into());
            }
        }
    }
    Ok(())
}

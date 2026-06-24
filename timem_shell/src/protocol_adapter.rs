use crate::prompt_cache::{CacheControl, PromptBlock, PromptBlockRole};
use crate::structured_output::StructuredOutputHint;
use crate::{ApiProtocol, ProviderConfig};
use serde_json::{json, Value};

pub fn build_request_from_blocks(
    config: &ProviderConfig,
    blocks: &[PromptBlock],
    structured_output: StructuredOutputHint,
) -> Value {
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible => {
            build_openai_compatible_request(config, blocks, structured_output)
        }
        ApiProtocol::OpenAiResponses => build_openai_responses_request(config, blocks),
        ApiProtocol::Anthropic => build_anthropic_request(config, blocks),
    }
}

fn build_openai_compatible_request(
    config: &ProviderConfig,
    blocks: &[PromptBlock],
    structured_output: StructuredOutputHint,
) -> Value {
    let messages = blocks
        .iter()
        .map(|block| {
            let mut message = json!({
                "role": role_label(block.role),
                "content": block.text,
            });
            apply_cache_control(&mut message, block.cache);
            message
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": config.max_tokens
    });
    apply_structured_output(&mut body, structured_output);
    body
}

fn build_openai_responses_request(config: &ProviderConfig, blocks: &[PromptBlock]) -> Value {
    let instructions = blocks
        .iter()
        .filter(|block| block.role == PromptBlockRole::System)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let input = blocks
        .iter()
        .filter(|block| block.role == PromptBlockRole::User)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "model": config.model,
        "instructions": instructions,
        "input": input,
        "max_output_tokens": config.max_tokens
    })
}

fn build_anthropic_request(config: &ProviderConfig, blocks: &[PromptBlock]) -> Value {
    let system = blocks
        .iter()
        .filter(|block| block.role == PromptBlockRole::System)
        .map(|block| {
            let mut item = json!({"type":"text", "text": block.text});
            apply_cache_control(&mut item, block.cache);
            item
        })
        .collect::<Vec<_>>();
    let content = blocks
        .iter()
        .filter(|block| block.role == PromptBlockRole::User)
        .map(|block| {
            let mut item = json!({"type":"text", "text": block.text});
            apply_cache_control(&mut item, block.cache);
            item
        })
        .collect::<Vec<_>>();
    json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "system": system,
        "messages": [{"role":"user", "content": content}]
    })
}

fn role_label(role: PromptBlockRole) -> &'static str {
    match role {
        PromptBlockRole::System => "system",
        PromptBlockRole::User => "user",
    }
}

fn apply_cache_control(value: &mut Value, cache: CacheControl) {
    if cache == CacheControl::Ephemeral {
        if let Some(map) = value.as_object_mut() {
            map.insert("cache_control".to_string(), json!({"type":"ephemeral"}));
        }
    }
}

fn apply_structured_output(value: &mut Value, structured_output: StructuredOutputHint) {
    if structured_output == StructuredOutputHint::JsonObject {
        if let Some(map) = value.as_object_mut() {
            map.insert("response_format".to_string(), json!({"type":"json_object"}));
        }
    }
}

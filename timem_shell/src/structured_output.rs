use crate::{ApiProtocol, ProviderConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutputHint {
    None,
    JsonObject,
}

pub fn plan_structured_output(config: &ProviderConfig) -> StructuredOutputHint {
    match config.api_protocol {
        ApiProtocol::OpenAiCompatible
            if supports_openai_compatible_json_object(&config.provider) =>
        {
            StructuredOutputHint::JsonObject
        }
        _ => StructuredOutputHint::None,
    }
}

fn supports_openai_compatible_json_object(provider: &str) -> bool {
    matches!(provider, "aliyun" | "dashscope" | "openai")
}

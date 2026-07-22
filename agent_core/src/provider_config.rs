use crate::{
    default_api_protocol_for_provider, default_base_url_for_provider, default_model_for_provider,
    parse_api_protocol, parse_token_count, ApiProtocol, OpenAiCompatibleOptions, ProviderConfig,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderConfigSource {
    pub provider: Option<String>,
    pub api_protocol: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_llm_output_tokens: Option<u32>,
    pub max_llm_input_tokens: Option<u32>,
    pub local_api_key: Option<String>,
    pub enable_thinking: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalLLMKeyFile {
    pub api_key: String,
    pub available_models: Vec<String>,
}

impl LocalLLMKeyFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut section = "";
        let mut api_key = String::new();
        let mut available_models = Vec::new();

        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.eq_ignore_ascii_case("key:") {
                section = "key";
                continue;
            }
            if line.eq_ignore_ascii_case("available_model:")
                || line.eq_ignore_ascii_case("available_models:")
            {
                section = "available_model";
                continue;
            }
            match section {
                "key" if api_key.is_empty() => api_key = line.to_string(),
                "available_model" => available_models.push(line.to_string()),
                _ => {}
            }
        }

        validate_provider_api_key(&api_key)?;
        if available_models.is_empty() {
            return Err("missing_available_model".to_string());
        }
        Ok(Self {
            api_key,
            available_models,
        })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    pub fn random_model(&self) -> &str {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as usize)
            .unwrap_or(0);
        let pid = std::process::id() as usize;
        let index = (now ^ pid) % self.available_models.len();
        &self.available_models[index]
    }

    pub fn to_provider_config(&self, model: &str) -> ProviderConfig {
        ProviderConfig {
            provider: "aliyun".to_string(),
            model: model.to_string(),
            base_url: default_base_url_for_provider("aliyun"),
            api_key: self.api_key.clone(),
            timeout_secs: 120,
            max_llm_output_tokens: 512,
            max_llm_input_tokens: 100_000,
            api_protocol: ApiProtocol::OpenAiCompatible,
            response_protocol: crate::ResponseProtocolKind::default(),
            openai_compatible: OpenAiCompatibleOptions::default(),
        }
    }
}

pub fn provider_config_from_sources(
    source: &ProviderConfigSource,
    env: &HashMap<String, String>,
) -> Result<ProviderConfig, String> {
    let provider = source
        .provider
        .clone()
        .or_else(|| env.get("TIMEM_GATEWAY_PROVIDER").cloned())
        .unwrap_or_else(|| "aliyun".to_string())
        .to_lowercase();
    let api_protocol = source
        .api_protocol
        .clone()
        .or_else(|| env.get("TIMEM_API_PROTOCOL").cloned())
        .map(|value| parse_api_protocol(&value))
        .transpose()?
        .unwrap_or_else(|| default_api_protocol_for_provider(&provider));
    let model = source
        .model
        .clone()
        .or_else(|| env.get("TIMEM_MODEL").cloned())
        .unwrap_or_else(|| default_model_for_provider(&provider).to_string());
    let base_url = source
        .base_url
        .clone()
        .or_else(|| env.get("TIMEM_BASE_URL").cloned())
        .or_else(|| vendor_base_url(&provider, env))
        .unwrap_or_else(|| default_base_url_for_provider(&provider));
    let api_key = source
        .api_key
        .clone()
        .or_else(|| env.get("TIMEM_API_KEY").cloned())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| vendor_api_key(&provider, env))
        .or_else(|| {
            if provider == "aliyun" || provider == "dashscope" {
                source.local_api_key.clone()
            } else {
                None
            }
        })
        .ok_or_else(|| {
            format!(
                "missing_api_key: set TIMEM_API_KEY, {}, or rust/key",
                vendor_key_hint(&provider)
            )
        })?;
    validate_provider_api_key(&api_key)?;
    let timeout_secs = source
        .timeout_secs
        .or_else(|| env.get("TIMEM_TIMEOUT").and_then(|v| v.parse().ok()))
        .unwrap_or(120);
    let max_llm_output_tokens = source
        .max_llm_output_tokens
        .or_else(|| {
            env.get("TIMEM_MAX_LLM_OUTPUT")
                .and_then(|value| parse_token_count(value))
        })
        .unwrap_or(10_000);
    let max_llm_input_tokens = source
        .max_llm_input_tokens
        .or_else(|| {
            env.get("TIMEM_MAX_LLM_INPUT")
                .and_then(|value| parse_token_count(value))
        })
        .unwrap_or(100_000);
    let enable_thinking = match source.enable_thinking {
        Some(value) => Some(value),
        None => env
            .get("TIMEM_ENABLE_THINKING")
            .map(|value| parse_bool_env("TIMEM_ENABLE_THINKING", value))
            .transpose()?,
    };
    let reasoning_effort = source
        .reasoning_effort
        .clone()
        .or_else(|| env.get("TIMEM_REASONING_EFFORT").cloned())
        .map(|value| validate_reasoning_effort(&value))
        .transpose()?;
    let stream = match source.stream {
        Some(value) => value,
        None => match env.get("TIMEM_STREAM") {
            Some(value) => parse_bool_env("TIMEM_STREAM", value)?,
            None => false,
        },
    };
    Ok(ProviderConfig {
        provider,
        model,
        base_url,
        api_key,
        timeout_secs,
        max_llm_output_tokens,
        max_llm_input_tokens,
        api_protocol,
        response_protocol: crate::ResponseProtocolKind::default(),
        openai_compatible: OpenAiCompatibleOptions {
            enable_thinking,
            reasoning_effort,
            stream,
        },
    })
}

fn parse_bool_env(key: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid_{key}: expected true or false")),
    }
}

fn validate_reasoning_effort(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("invalid_TIMEM_REASONING_EFFORT".to_string());
    }
    Ok(value.to_string())
}

pub fn apply_openai_compatible_env_value(
    options: &mut OpenAiCompatibleOptions,
    key: &str,
    value: &str,
) -> Result<bool, String> {
    match key {
        "TIMEM_ENABLE_THINKING" => {
            options.enable_thinking = Some(parse_bool_env(key, value)?);
        }
        "TIMEM_REASONING_EFFORT" => {
            options.reasoning_effort = Some(validate_reasoning_effort(value)?);
        }
        "TIMEM_STREAM" => {
            options.stream = parse_bool_env(key, value)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn vendor_api_key(provider: &str, env: &HashMap<String, String>) -> Option<String> {
    let key = match provider {
        "openai" => env.get("OPENAI_API_KEY").cloned(),
        "anthropic" => env
            .get("ANTHROPIC_API_KEY")
            .cloned()
            .or_else(|| env.get("ANTHROPIC_AUTH_TOKEN").cloned()),
        "aliyun" | "dashscope" => env.get("DASHSCOPE_API_KEY").cloned(),
        _ => None,
    };
    key.filter(|value| !value.trim().is_empty())
}

fn vendor_base_url(provider: &str, env: &HashMap<String, String>) -> Option<String> {
    match provider {
        "openai" => env.get("OPENAI_BASE_URL").cloned(),
        "anthropic" => env.get("ANTHROPIC_BASE_URL").cloned(),
        "aliyun" | "dashscope" => env.get("DASHSCOPE_BASE_URL").cloned(),
        _ => None,
    }
}

fn vendor_key_hint(provider: &str) -> &str {
    match provider {
        "openai" => "OPENAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        _ => "DASHSCOPE_API_KEY",
    }
}

pub fn validate_provider_api_key(api_key: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("missing_api_key".to_string());
    }
    if !api_key.is_ascii() {
        return Err("invalid_api_key_non_ascii".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/provider_config_tests.rs"]
mod tests;

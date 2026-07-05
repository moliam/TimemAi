use crate::{
    default_api_protocol_for_provider, default_base_url_for_provider, default_model_for_provider,
    parse_api_protocol, parse_token_count, ApiProtocol, ProviderConfig,
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
            response_protocol: crate::ResponseProtocolKind::Markdown,
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
    Ok(ProviderConfig {
        provider,
        model,
        base_url,
        api_key,
        timeout_secs,
        max_llm_output_tokens,
        max_llm_input_tokens,
        api_protocol,
        response_protocol: crate::ResponseProtocolKind::Markdown,
    })
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
mod tests {
    use super::*;
    use crate::ApiProtocol;

    fn env(items: &[(&str, &str)]) -> HashMap<String, String> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn generic_api_key_wins_over_vendor_key() {
        let config = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("aliyun".into()),
                ..ProviderConfigSource::default()
            },
            &env(&[
                ("TIMEM_API_KEY", "generic"),
                ("DASHSCOPE_API_KEY", "vendor"),
            ]),
        )
        .unwrap();
        assert_eq!(config.api_key, "generic");
    }

    #[test]
    fn default_gateway_provider_is_aliyun() {
        let config = provider_config_from_sources(
            &ProviderConfigSource::default(),
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(config.provider, "aliyun");
        assert_eq!(config.model, "qwen-plus");
        assert_eq!(
            config.base_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
    }

    #[test]
    fn known_providers_have_explicit_default_base_urls() {
        let cases = [
            (
                "aliyun",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                ApiProtocol::OpenAiCompatible,
            ),
            (
                "dashscope",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                ApiProtocol::OpenAiCompatible,
            ),
            (
                "openai",
                "https://api.openai.com/v1",
                ApiProtocol::OpenAiResponses,
            ),
            (
                "anthropic",
                "https://api.anthropic.com",
                ApiProtocol::Anthropic,
            ),
        ];

        for (provider, expected_base_url, expected_protocol) in cases {
            let config = provider_config_from_sources(
                &ProviderConfigSource {
                    provider: Some(provider.to_string()),
                    ..ProviderConfigSource::default()
                },
                &env(&[("TIMEM_API_KEY", "k")]),
            )
            .unwrap();
            assert_eq!(config.base_url, expected_base_url);
            assert_eq!(config.api_protocol, expected_protocol);
        }
    }

    #[test]
    fn empty_generic_api_key_falls_back_to_vendor_key() {
        let config = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("aliyun".into()),
                ..ProviderConfigSource::default()
            },
            &env(&[("TIMEM_API_KEY", ""), ("DASHSCOPE_API_KEY", "vendor")]),
        )
        .unwrap();
        assert_eq!(config.api_key, "vendor");
    }

    #[test]
    fn local_key_is_only_used_for_aliyun_like_providers() {
        let aliyun = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("aliyun".into()),
                local_api_key: Some("local-key".into()),
                ..ProviderConfigSource::default()
            },
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(aliyun.api_key, "local-key");

        let err = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("openai".into()),
                local_api_key: Some("local-key".into()),
                ..ProviderConfigSource::default()
            },
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(err.contains("missing_api_key"));
    }

    #[test]
    fn local_llm_key_file_parses_key_and_models() {
        let parsed =
            LocalLLMKeyFile::parse("\nkey:\nsk-test\n\navailable_model:\nqwen3.7-plus\nglm-5.2\n")
                .unwrap();
        assert_eq!(parsed.api_key, "sk-test");
        assert_eq!(parsed.available_models, vec!["qwen3.7-plus", "glm-5.2"]);
    }

    #[test]
    fn local_llm_key_file_rejects_missing_models() {
        let err = LocalLLMKeyFile::parse("key:\nsk-test\n").unwrap_err();
        assert_eq!(err, "missing_available_model");
    }

    #[test]
    fn local_llm_key_file_builds_aliyun_provider_config() {
        let parsed =
            LocalLLMKeyFile::parse("key:\nsk-test\navailable_model:\nqwen3.7-plus\n").unwrap();
        let config = parsed.to_provider_config("qwen3.7-plus");
        assert_eq!(config.provider, "aliyun");
        assert_eq!(config.model, "qwen3.7-plus");
        assert_eq!(config.api_key, "sk-test");
        assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
    }

    #[test]
    fn empty_api_key_reports_missing_key() {
        let err = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("openai".into()),
                ..ProviderConfigSource::default()
            },
            &env(&[("TIMEM_API_KEY", ""), ("OPENAI_API_KEY", "")]),
        )
        .unwrap_err();
        assert!(err.contains("missing_api_key"));
    }

    #[test]
    fn non_ascii_api_key_reports_clear_error() {
        let err = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("aliyun".into()),
                ..ProviderConfigSource::default()
            },
            &env(&[("TIMEM_API_KEY", "你的token")]),
        )
        .unwrap_err();
        assert!(err.contains("invalid_api_key_non_ascii"));
    }

    #[test]
    fn source_values_override_env_config_values() {
        let config = provider_config_from_sources(
            &ProviderConfigSource {
                provider: Some("custom".into()),
                api_protocol: Some("anthropic".into()),
                model: Some("cli-model".into()),
                base_url: Some("https://cli.example/v1".into()),
                timeout_secs: Some(33),
                max_llm_output_tokens: Some(1234),
                max_llm_input_tokens: Some(64_000),
                api_key: Some("cli-key".into()),
                ..ProviderConfigSource::default()
            },
            &env(&[
                ("TIMEM_GATEWAY_PROVIDER", "aliyun"),
                ("TIMEM_API_PROTOCOL", "openai-compatible"),
                ("TIMEM_MODEL", "env-model"),
                ("TIMEM_BASE_URL", "https://env.example/v1"),
                ("TIMEM_TIMEOUT", "99"),
                ("TIMEM_MAX_LLM_OUTPUT", "9999"),
                ("TIMEM_MAX_LLM_INPUT", "128K"),
                ("TIMEM_API_KEY", "env-key"),
            ]),
        )
        .unwrap();

        assert_eq!(config.provider, "custom");
        assert_eq!(config.api_protocol, ApiProtocol::Anthropic);
        assert_eq!(config.model, "cli-model");
        assert_eq!(config.base_url, "https://cli.example/v1");
        assert_eq!(config.timeout_secs, 33);
        assert_eq!(config.max_llm_output_tokens, 1234);
        assert_eq!(config.max_llm_input_tokens, 64_000);
        assert_eq!(config.api_key, "cli-key");
    }

    #[test]
    fn token_limits_default_and_can_come_from_env() {
        let defaulted = provider_config_from_sources(
            &ProviderConfigSource::default(),
            &env(&[("TIMEM_API_KEY", "k")]),
        )
        .unwrap();
        assert_eq!(defaulted.max_llm_input_tokens, 100_000);
        assert_eq!(defaulted.max_llm_output_tokens, 10_000);

        let configured = provider_config_from_sources(
            &ProviderConfigSource::default(),
            &env(&[
                ("TIMEM_API_KEY", "k"),
                ("TIMEM_MAX_LLM_INPUT", "128K"),
                ("TIMEM_MAX_LLM_OUTPUT", "8K"),
            ]),
        )
        .unwrap();
        assert_eq!(configured.max_llm_input_tokens, 128_000);
        assert_eq!(configured.max_llm_output_tokens, 8_000);
    }
}

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
    let parsed = LocalLLMKeyFile::parse("key:\nsk-test\navailable_model:\nqwen3.7-plus\n").unwrap();
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

#[test]
fn openai_compatible_thinking_options_are_loaded_from_env() {
    let config = provider_config_from_sources(
        &ProviderConfigSource::default(),
        &env(&[
            ("TIMEM_API_KEY", "k"),
            ("TIMEM_ENABLE_THINKING", "true"),
            ("TIMEM_REASONING_EFFORT", "max"),
            ("TIMEM_STREAM", "true"),
        ]),
    )
    .unwrap();

    assert_eq!(config.openai_compatible.enable_thinking, Some(true));
    assert_eq!(
        config.openai_compatible.reasoning_effort.as_deref(),
        Some("max")
    );
    assert!(config.openai_compatible.stream);
}

#[test]
fn openai_compatible_thinking_options_reject_invalid_env_values() {
    for (key, value) in [
        ("TIMEM_ENABLE_THINKING", "sometimes"),
        ("TIMEM_STREAM", "maybe"),
        ("TIMEM_REASONING_EFFORT", "max; rm"),
    ] {
        let error = provider_config_from_sources(
            &ProviderConfigSource::default(),
            &env(&[("TIMEM_API_KEY", "k"), (key, value)]),
        )
        .unwrap_err();
        assert!(error.contains(key), "unexpected error for {key}: {error}");
    }
}

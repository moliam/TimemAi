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
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource {
            ..ModelServiceConfigSource::default()
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
fn defaults_are_defined_without_a_service_identity() {
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource::default(),
        &env(&[("TIMEM_API_KEY", "k")]),
    )
    .unwrap();
    assert_eq!(config.model, "qwen-plus");
    assert_eq!(
        config.base_url,
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
    assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
}

#[test]
fn optional_api_key_config_supports_configurable_hosts_without_weakening_strict_startup() {
    let source = ModelServiceConfigSource::default();
    let empty_env = HashMap::new();

    let draft =
        model_service_config_from_sources_allow_missing_api_key(&source, &empty_env).unwrap();
    assert!(draft.api_key.is_empty());
    assert_eq!(draft.model, "qwen-plus");

    assert!(model_service_config_from_sources(&source, &empty_env)
        .unwrap_err()
        .starts_with("missing_api_key:"));
}

#[test]
fn api_protocols_have_explicit_default_base_urls() {
    let cases = [
        (
            "openai-compatible",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ApiProtocol::OpenAiCompatible,
        ),
        (
            "openai-responses",
            "https://api.openai.com/v1",
            ApiProtocol::OpenAiResponses,
        ),
        (
            "anthropic",
            "https://api.anthropic.com",
            ApiProtocol::Anthropic,
        ),
    ];

    for (protocol, expected_base_url, expected_protocol) in cases {
        let config = model_service_config_from_sources(
            &ModelServiceConfigSource {
                api_protocol: Some(protocol.to_string()),
                ..ModelServiceConfigSource::default()
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
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource {
            ..ModelServiceConfigSource::default()
        },
        &env(&[("TIMEM_API_KEY", ""), ("DASHSCOPE_API_KEY", "vendor")]),
    )
    .unwrap();
    assert_eq!(config.api_key, "vendor");
}

#[test]
fn local_key_is_available_to_the_default_model_service() {
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource {
            local_api_key: Some("local-key".into()),
            ..ModelServiceConfigSource::default()
        },
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(config.api_key, "local-key");
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
fn local_llm_key_file_builds_model_service_config() {
    let parsed = LocalLLMKeyFile::parse("key:\nsk-test\navailable_model:\nqwen3.7-plus\n").unwrap();
    let config = parsed.to_model_service_config("qwen3.7-plus");
    assert_eq!(config.model, "qwen3.7-plus");
    assert_eq!(config.api_key, "sk-test");
    assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
}

#[test]
fn empty_api_key_reports_missing_key() {
    let err = model_service_config_from_sources(
        &ModelServiceConfigSource {
            ..ModelServiceConfigSource::default()
        },
        &env(&[("TIMEM_API_KEY", ""), ("OPENAI_API_KEY", "")]),
    )
    .unwrap_err();
    assert!(err.contains("missing_api_key"));
}

#[test]
fn non_ascii_api_key_reports_clear_error() {
    let err = model_service_config_from_sources(
        &ModelServiceConfigSource {
            ..ModelServiceConfigSource::default()
        },
        &env(&[("TIMEM_API_KEY", "你的token")]),
    )
    .unwrap_err();
    assert!(err.contains("invalid_api_key_non_ascii"));
}

#[test]
fn api_key_rejects_control_characters_and_whitespace() {
    for key in ["sk-test\nInjected: yes", "sk-test token", "sk-test\tsecret"] {
        let err = model_service_config_from_sources(
            &ModelServiceConfigSource {
                ..ModelServiceConfigSource::default()
            },
            &env(&[("TIMEM_API_KEY", key)]),
        )
        .unwrap_err();
        assert!(
            err.contains("invalid_api_key_control_or_whitespace"),
            "{err}"
        );
    }
}

#[test]
fn source_values_override_env_config_values() {
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource {
            api_protocol: Some("anthropic".into()),
            model: Some("cli-model".into()),
            base_url: Some("https://cli.example/v1".into()),
            timeout_secs: Some(33),
            max_llm_output_tokens: Some(1234),
            max_llm_input_tokens: Some(64_000),
            api_key: Some("cli-key".into()),
            ..ModelServiceConfigSource::default()
        },
        &env(&[
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
    let defaulted = model_service_config_from_sources(
        &ModelServiceConfigSource::default(),
        &env(&[("TIMEM_API_KEY", "k")]),
    )
    .unwrap();
    assert_eq!(defaulted.max_llm_input_tokens, 100_000);
    assert_eq!(defaulted.max_llm_output_tokens, 20_000);

    let configured = model_service_config_from_sources(
        &ModelServiceConfigSource::default(),
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
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource::default(),
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
        let error = model_service_config_from_sources(
            &ModelServiceConfigSource::default(),
            &env(&[("TIMEM_API_KEY", "k"), (key, value)]),
        )
        .unwrap_err();
        assert!(error.contains(key), "unexpected error for {key}: {error}");
    }
}

#[test]
fn openai_cache_mode_defaults_to_auto_and_accepts_all_documented_values() {
    let default_config = model_service_config_from_sources(
        &ModelServiceConfigSource::default(),
        &env(&[("TIMEM_API_KEY", "k")]),
    )
    .unwrap();
    assert_eq!(
        default_config.openai_compatible.cache_mode,
        OpenAiCompatibleCacheMode::Auto
    );

    for (value, expected) in [
        ("auto", OpenAiCompatibleCacheMode::Auto),
        ("off", OpenAiCompatibleCacheMode::Off),
        ("ephemeral", OpenAiCompatibleCacheMode::Ephemeral),
    ] {
        let config = model_service_config_from_sources(
            &ModelServiceConfigSource::default(),
            &env(&[("TIMEM_API_KEY", "k"), ("TIMEM_OPENAI_CACHE_MODE", value)]),
        )
        .unwrap();
        assert_eq!(config.openai_compatible.cache_mode, expected);
    }
}

#[test]
fn openai_cache_mode_source_overrides_environment_and_invalid_values_fail() {
    let config = model_service_config_from_sources(
        &ModelServiceConfigSource {
            openai_cache_mode: Some("off".to_string()),
            ..ModelServiceConfigSource::default()
        },
        &env(&[
            ("TIMEM_API_KEY", "k"),
            ("TIMEM_OPENAI_CACHE_MODE", "ephemeral"),
        ]),
    )
    .unwrap();
    assert_eq!(
        config.openai_compatible.cache_mode,
        OpenAiCompatibleCacheMode::Off
    );

    let error = model_service_config_from_sources(
        &ModelServiceConfigSource::default(),
        &env(&[
            ("TIMEM_API_KEY", "k"),
            ("TIMEM_OPENAI_CACHE_MODE", "forever"),
        ]),
    )
    .unwrap_err();
    assert!(error.contains("invalid_TIMEM_OPENAI_CACHE_MODE"));
}

#[test]
fn dynamic_openai_cache_mode_application_is_validated() {
    let mut options = OpenAiCompatibleOptions::default();
    assert!(apply_openai_compatible_env_value(
        &mut options,
        "TIMEM_OPENAI_CACHE_MODE",
        "ephemeral",
    )
    .unwrap());
    assert_eq!(options.cache_mode, OpenAiCompatibleCacheMode::Ephemeral);
    assert!(
        apply_openai_compatible_env_value(&mut options, "TIMEM_OPENAI_CACHE_MODE", "invalid",)
            .unwrap_err()
            .contains("invalid_TIMEM_OPENAI_CACHE_MODE")
    );
}

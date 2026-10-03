use super::*;
use crate::{
    ApiProtocol, InteractionConfig, LlmResponse, NativeToolCall, OpenAiCompatibleOptions,
    ResponseProtocolKind, UsageStats,
};

struct ProbeClient {
    calls: usize,
}

struct TransientFailureClient {
    calls: usize,
}

struct CancelledThenNativeClient {
    calls: usize,
}

impl ModelClient for ProbeClient {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Err("unexpected_inline_call".to_string())
    }

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        request: &ModelInteractionRequest,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        let count = if request.parallel_tool_calls { 2 } else { 1 };
        Ok(LlmResponse {
            tool_calls: (0..count)
                .map(|index| NativeToolCall {
                    assistant_continuation: None,
                    id: format!("probe_{index}"),
                    name: PROBE_TOOL_NAME.to_string(),
                    arguments: json!({"slot": index + 1}),
                    raw_arguments: format!("{{\"slot\":{}}}", index + 1),
                })
                .collect(),
            content: String::new(),
            model_name: config.model.clone(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    }
}

impl ModelClient for CancelledThenNativeClient {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Err("unexpected_inline_call".to_string())
    }

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        request: &ModelInteractionRequest,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        if self.calls == 1 {
            return Err("cancelled_by_user".to_string());
        }
        let count = if request.parallel_tool_calls { 2 } else { 1 };
        Ok(LlmResponse {
            tool_calls: (0..count)
                .map(|index| NativeToolCall {
                    assistant_continuation: None,
                    id: format!("probe_{index}"),
                    name: PROBE_TOOL_NAME.to_string(),
                    arguments: json!({"slot": index + 1}),
                    raw_arguments: format!("{{\"slot\":{}}}", index + 1),
                })
                .collect(),
            content: String::new(),
            model_name: config.model.clone(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    }
}

impl ModelClient for TransientFailureClient {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Err("unexpected_inline_call".to_string())
    }

    fn call_model_interaction(
        &mut self,
        _config: &ModelServiceConfig,
        _request: &ModelInteractionRequest,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        Err("model_network_error: temporary probe outage".to_string())
    }
}

fn auto_config(model: &str) -> ModelServiceConfig {
    ModelServiceConfig {
        interaction: InteractionConfig {
            tool_call_mode: ToolCallMode::Auto,
            parallel_tool_calls: ParallelToolCalls::Auto,
            native_tools_supported: None,
            ..InteractionConfig::default()
        },
        model: model.to_string(),
        base_url: "https://gateway.example.test/v1/?secret=redacted".to_string(),
        api_key: "not-a-real-key".to_string(),
        http_headers: Default::default(),
        request_fields: Default::default(),
        timeout_secs: 1,
        max_llm_output_tokens: 128,
        max_llm_input_tokens: 4096,
        api_protocol: ApiProtocol::OpenAiCompatible,
        response_protocol: ResponseProtocolKind::Xml,
        openai_compatible: OpenAiCompatibleOptions::default(),
        http_transport: Default::default(),
    }
}

#[test]
fn auto_probe_detects_native_parallel_and_reuses_process_cache() {
    let model = format!("probe-test-{}", std::process::id());
    let config = auto_config(&model);
    let audit = std::env::temp_dir().join(format!("timem-negotiation-{model}.json"));
    let mut first_client = ProbeClient { calls: 0 };
    let first = negotiate_interaction(&mut first_client, &config, &audit, &mut || false);
    assert_eq!(first.resolved_mode, ToolCallMode::Native);
    assert_eq!(first.active_prompt_protocol, "json");
    assert!(first.parallel_supported);
    assert!(first.parallel_enabled);
    assert_eq!(first.observed_tool_calls, 2);
    assert_eq!(first_client.calls, 2);

    let mut second_client = ProbeClient { calls: 0 };
    let cached = negotiate_interaction(&mut second_client, &config, &audit, &mut || false);
    assert_eq!(cached.source, CapabilityProbeSource::Cache);
    assert_eq!(second_client.calls, 0);
}

#[test]
fn transient_probe_failure_does_not_permanently_pin_inline_mode() {
    let model = format!("transient-probe-test-{}", std::process::id());
    let config = auto_config(&model);
    let audit = std::env::temp_dir().join(format!("timem-negotiation-{model}.json"));
    let mut failing_client = TransientFailureClient { calls: 0 };
    let inconclusive = negotiate_interaction(&mut failing_client, &config, &audit, &mut || false);
    assert_eq!(inconclusive.resolved_mode, ToolCallMode::Native);
    assert_eq!(inconclusive.source, CapabilityProbeSource::Probe);
    assert!(inconclusive
        .reason
        .starts_with("native_probe_inconclusive:"));
    assert_eq!(failing_client.calls, 1);

    let mut recovered_client = ProbeClient { calls: 0 };
    let recovered = negotiate_interaction(&mut recovered_client, &config, &audit, &mut || false);
    assert_eq!(recovered.resolved_mode, ToolCallMode::Native);
    assert_eq!(recovered.source, CapabilityProbeSource::Probe);
    assert_eq!(recovered_client.calls, 2);
}

#[test]
fn cancelled_probe_is_not_cached_and_next_turn_can_restore_native_mode() {
    let model = format!("cancelled-probe-test-{}", std::process::id());
    let config = auto_config(&model);
    let audit = std::env::temp_dir().join(format!("timem-negotiation-{model}.json"));
    let mut client = CancelledThenNativeClient { calls: 0 };

    let cancelled = negotiate_interaction(&mut client, &config, &audit, &mut || false);
    assert_eq!(cancelled.resolved_mode, ToolCallMode::Native);
    assert_eq!(cancelled.source, CapabilityProbeSource::Probe);
    assert_eq!(
        cancelled.reason,
        "native_probe_inconclusive:cancelled_by_user"
    );
    assert_eq!(client.calls, 1);

    let recovered = negotiate_interaction(&mut client, &config, &audit, &mut || false);
    assert_eq!(recovered.resolved_mode, ToolCallMode::Native);
    assert_eq!(recovered.source, CapabilityProbeSource::Probe);
    assert_eq!(recovered.active_prompt_protocol, "json");
    assert_eq!(client.calls, 3);
}

struct AlwaysFailureClient {
    calls: usize,
    error: &'static str,
}

impl ModelClient for AlwaysFailureClient {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Err("unexpected_inline_call".to_string())
    }

    fn call_model_interaction(
        &mut self,
        _config: &ModelServiceConfig,
        _request: &ModelInteractionRequest,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        Err(self.error.to_string())
    }
}

struct ParallelTransientClient {
    calls: usize,
}

impl ModelClient for ParallelTransientClient {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Err("unexpected_inline_call".to_string())
    }

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        _request: &ModelInteractionRequest,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        if self.calls == 2 {
            return Err("model_http_503: temporary parallel probe outage".to_string());
        }
        Ok(LlmResponse {
            tool_calls: vec![NativeToolCall {
                assistant_continuation: None,
                id: "probe_1".to_string(),
                name: PROBE_TOOL_NAME.to_string(),
                arguments: json!({"slot": 1}),
                raw_arguments: "{\"slot\":1}".to_string(),
            }],
            content: String::new(),
            model_name: config.model.clone(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    }
}

fn durable_auto_config(label: &str) -> ModelServiceConfig {
    let mut config = auto_config(&format!("durable-{label}-{}", std::process::id()));
    config.interaction.capability_probe_endpoint_id = Some(format!("endpoint-{label}"));
    config
}

fn persisted_for(config: &ModelServiceConfig) -> PersistedCapabilityProbe {
    PersistedCapabilityProbe {
        identity: capability_probe_identity(config).expect("endpoint identity"),
        native_supported: true,
        parallel_supported: true,
        observed_tool_calls: 2,
        reason: "persisted_native_and_parallel".to_string(),
    }
}

#[test]
fn capability_probe_identity_is_strict_serializable_and_secret_free() {
    let mut config = durable_auto_config("identity");
    config.base_url =
        "HTTPS://user:password@Gateway.Example.test/v1/?api_key=secret#token".to_string();
    config.openai_compatible.enable_thinking = Some(true);
    config.openai_compatible.reasoning_effort = Some("high".to_string());

    let identity = capability_probe_identity(&config).expect("identity");
    assert_eq!(identity.endpoint_id, "endpoint-identity");
    assert_eq!(identity.api_protocol, "openai-compatible");
    assert_eq!(identity.gateway, "https://gateway.example.test/v1");
    assert_eq!(identity.model, config.model);
    assert_eq!(identity.enable_thinking, Some(true));
    assert_eq!(identity.reasoning_effort.as_deref(), Some("high"));

    let encoded = serde_json::to_string(&identity).unwrap();
    let decoded: CapabilityProbeIdentity = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, identity);
    for secret in ["password", "api_key", "secret", "token", "not-a-real-key"] {
        assert!(
            !encoded.contains(secret),
            "identity leaked {secret}: {encoded}"
        );
    }
}

#[test]
fn exact_persisted_identity_is_reused_without_model_calls() {
    let mut config = durable_auto_config("persisted-hit");
    config.interaction.persisted_capability_probe = Some(persisted_for(&config));
    let audit = std::env::temp_dir().join("timem-negotiation-persisted-hit.json");
    let mut client = ProbeClient { calls: 0 };

    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);
    assert_eq!(outcome.profile.source, CapabilityProbeSource::Cache);
    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
    assert!(outcome.profile.parallel_enabled);
    assert!(!outcome.probe_performed);
    assert!(outcome.persisted_probe.is_none());
    assert_eq!(client.calls, 0);
}

#[test]
fn every_persisted_identity_dimension_must_match() {
    type Mutator = fn(&mut CapabilityProbeIdentity);
    let cases: [(&str, Mutator); 6] = [
        ("endpoint", |identity| {
            identity.endpoint_id.push_str("-other")
        }),
        ("protocol", |identity| {
            identity.api_protocol.push_str("-other")
        }),
        ("gateway", |identity| identity.gateway.push_str("/other")),
        ("model", |identity| identity.model.push_str("-other")),
        ("thinking", |identity| identity.enable_thinking = Some(true)),
        ("reasoning", |identity| {
            identity.reasoning_effort = Some("high".to_string())
        }),
    ];

    for (label, mutate) in cases {
        let mut config = durable_auto_config(&format!("mismatch-{label}"));
        let mut persisted = persisted_for(&config);
        mutate(&mut persisted.identity);
        config.interaction.persisted_capability_probe = Some(persisted);
        let audit = std::env::temp_dir().join(format!("timem-negotiation-{label}.json"));
        let mut client = ProbeClient { calls: 0 };
        let outcome =
            negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);
        assert!(
            outcome.probe_performed,
            "mismatch {label} reused persisted data"
        );
        assert_eq!(outcome.profile.source, CapabilityProbeSource::Probe);
        assert_eq!(client.calls, 2, "mismatch {label} did not probe");
        assert!(outcome.persisted_probe.is_some());
    }
}

#[test]
fn only_durable_probe_outcomes_are_returned_for_persistence() {
    let audit = std::env::temp_dir().join("timem-negotiation-durable-results.json");

    let permanent_config = durable_auto_config("permanent-failure");
    let mut permanent = AlwaysFailureClient {
        calls: 0,
        error: "model_http_400: tools are not supported by this endpoint",
    };
    let outcome = negotiate_interaction_outcome(
        &mut permanent,
        &permanent_config,
        &audit,
        &mut || false,
        false,
    );
    let record = outcome.persisted_probe.expect("permanent result");
    assert!(!record.native_supported);
    assert_eq!(
        record.identity,
        capability_probe_identity(&permanent_config).unwrap()
    );

    let transient_config = durable_auto_config("transient-failure");
    let mut transient = TransientFailureClient { calls: 0 };
    let outcome = negotiate_interaction_outcome(
        &mut transient,
        &transient_config,
        &audit,
        &mut || false,
        false,
    );
    assert!(outcome.persisted_probe.is_none());

    let cancelled_config = durable_auto_config("cancelled-failure");
    let mut cancelled = AlwaysFailureClient {
        calls: 0,
        error: "cancelled_by_user",
    };
    let outcome = negotiate_interaction_outcome(
        &mut cancelled,
        &cancelled_config,
        &audit,
        &mut || false,
        false,
    );
    assert!(outcome.persisted_probe.is_none());

    let parallel_config = durable_auto_config("parallel-transient");
    let mut parallel = ParallelTransientClient { calls: 0 };
    let outcome = negotiate_interaction_outcome(
        &mut parallel,
        &parallel_config,
        &audit,
        &mut || false,
        false,
    );
    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
    assert!(outcome.persisted_probe.is_none());
    assert_eq!(parallel.calls, 2);
}

struct TextOnlyProbeClient {
    calls: usize,
}

impl ModelClient for TextOnlyProbeClient {
    fn call_model(
        &mut self,
        _config: &ModelServiceConfig,
        _prompt: &str,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        Err("unexpected_inline_call".to_string())
    }

    fn call_model_interaction(
        &mut self,
        config: &ModelServiceConfig,
        _request: &ModelInteractionRequest,
        _audit_file: &Path,
        _should_cancel: &mut dyn FnMut() -> bool,
    ) -> Result<LlmResponse, String> {
        self.calls += 1;
        Ok(LlmResponse {
            tool_calls: Vec::new(),
            content: "I cannot follow that instruction.".to_string(),
            model_name: config.model.clone(),
            usage: UsageStats::zero(),
            truncated: false,
        })
    }
}

#[test]
fn inconclusive_probe_results_keep_native_and_are_not_persisted() {
    let audit = std::env::temp_dir().join("timem-negotiation-inconclusive-results.json");
    let cases = [
        "model_http_400: invalid request payload",
        "model_http_401: tools are not supported until authentication succeeds",
        "model_http_429: tools are not supported while rate limited",
        "model_http_500: tools are not supported due to server failure",
        "model_network_error: tools are not supported because connection reset",
        "random provider failure",
    ];
    for (index, error) in cases.into_iter().enumerate() {
        let config = durable_auto_config(&format!("inconclusive-{index}"));
        let mut client = AlwaysFailureClient { calls: 0, error };
        let outcome =
            negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);
        assert_eq!(
            outcome.profile.resolved_mode,
            ToolCallMode::Native,
            "{error}"
        );
        assert_eq!(
            outcome.profile.source,
            CapabilityProbeSource::Probe,
            "{error}"
        );
        assert!(outcome.persisted_probe.is_none(), "{error}");
        assert_eq!(client.calls, 1, "{error}");
    }

    let config = durable_auto_config("text-only");
    let mut client = TextOnlyProbeClient { calls: 0 };
    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);
    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
    assert_eq!(
        outcome.profile.reason,
        "native_probe_inconclusive:returned_no_tool_calls"
    );
    assert!(outcome.persisted_probe.is_none());
    assert_eq!(client.calls, 1);
}

#[test]
fn forced_reprobe_bypasses_persisted_and_process_caches() {
    let mut config = durable_auto_config("forced");
    let mut persisted = persisted_for(&config);
    persisted.native_supported = false;
    persisted.parallel_supported = false;
    persisted.observed_tool_calls = 0;
    persisted.reason = "stale_unsupported".to_string();
    config.interaction.persisted_capability_probe = Some(persisted);
    let audit = std::env::temp_dir().join("timem-negotiation-forced.json");

    let mut cached_client = ProbeClient { calls: 0 };
    let cached =
        negotiate_interaction_outcome(&mut cached_client, &config, &audit, &mut || false, false);
    assert_eq!(cached.profile.resolved_mode, ToolCallMode::Inline);
    assert_eq!(cached_client.calls, 0);

    let mut first_force = ProbeClient { calls: 0 };
    let first = force_reprobe_interaction(&mut first_force, &config, &audit, &mut || false);
    assert_eq!(first.profile.resolved_mode, ToolCallMode::Native);
    assert_eq!(first_force.calls, 2);
    assert!(first.persisted_probe.is_some());

    let mut second_force = ProbeClient { calls: 0 };
    let second = force_reprobe_interaction(&mut second_force, &config, &audit, &mut || false);
    assert_eq!(second.profile.resolved_mode, ToolCallMode::Native);
    assert_eq!(second_force.calls, 2);
}

#[test]
fn known_native_support_skips_probe_and_uses_native() {
    let mut config = auto_config("known-native-support");
    config.interaction.native_tools_supported = Some(true);
    let audit = std::env::temp_dir().join("timem-negotiation-known-native.json");
    let mut client = ProbeClient { calls: 0 };

    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
    assert_eq!(outcome.profile.source, CapabilityProbeSource::Explicit);
    assert!(!outcome.probe_performed);
    assert!(outcome.persisted_probe.is_none());
    assert_eq!(client.calls, 0);
}

#[test]
fn disabled_native_tools_skip_probe_and_use_inline() {
    let mut config = auto_config("native-tools-disabled");
    config.interaction.native_tools_supported = Some(false);
    let audit = std::env::temp_dir().join("timem-negotiation-native-disabled.json");
    let mut client = ProbeClient { calls: 0 };

    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Inline);
    assert_eq!(outcome.profile.source, CapabilityProbeSource::Explicit);
    assert!(!outcome.probe_performed);
    assert!(outcome.persisted_probe.is_none());
    assert_eq!(client.calls, 0);
}

#[test]
fn responses_protocol_is_agent_native_and_enables_parallel_auto_without_probe() {
    let mut config = auto_config("responses-native-support");
    config.api_protocol = ApiProtocol::OpenAiResponses;
    config.interaction.native_tools_supported = None;
    let audit = std::env::temp_dir().join("timem-negotiation-responses-native.json");
    let mut client = ProbeClient { calls: 0 };

    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
    assert_eq!(outcome.profile.source, CapabilityProbeSource::Explicit);
    assert!(outcome.profile.parallel_supported);
    assert!(outcome.profile.parallel_enabled);
    assert!(!outcome.probe_performed);
    assert!(outcome.persisted_probe.is_none());
    assert_eq!(client.calls, 0);
}

#[test]
fn responses_protocol_respects_explicitly_disabled_parallel_calls() {
    let mut config = auto_config("responses-parallel-disabled");
    config.api_protocol = ApiProtocol::OpenAiResponses;
    config.interaction.parallel_tool_calls = ParallelToolCalls::Disabled;
    let audit = std::env::temp_dir().join("timem-negotiation-responses-parallel-disabled.json");
    let mut client = ProbeClient { calls: 0 };

    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

    assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
    assert!(!outcome.profile.parallel_supported);
    assert!(!outcome.profile.parallel_enabled);
    assert!(!outcome.probe_performed);
    assert_eq!(client.calls, 0);
}

#[test]
fn known_native_protocols_enable_parallel_auto_without_probing() {
    for protocol in [ApiProtocol::OpenAiCompatible, ApiProtocol::Anthropic] {
        let mut config = auto_config(&format!("known-native-auto-{protocol:?}"));
        config.api_protocol = protocol;
        config.interaction.native_tools_supported = Some(true);
        let audit = std::env::temp_dir().join(format!(
            "timem-negotiation-known-native-auto-{protocol:?}.json"
        ));
        let mut client = ProbeClient { calls: 0 };

        let outcome =
            negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

        assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
        assert!(outcome.profile.parallel_supported, "{protocol:?}");
        assert!(outcome.profile.parallel_enabled, "{protocol:?}");
        assert!(!outcome.probe_performed);
        assert_eq!(client.calls, 0);
    }
}

#[test]
fn known_native_parallel_modes_respect_explicit_enabled_and_disabled() {
    for mode in [ParallelToolCalls::Enabled, ParallelToolCalls::Disabled] {
        for protocol in [
            ApiProtocol::OpenAiCompatible,
            ApiProtocol::OpenAiResponses,
            ApiProtocol::Anthropic,
        ] {
            let mut config = auto_config(&format!("known-native-{mode:?}-{protocol:?}"));
            config.api_protocol = protocol;
            config.interaction.native_tools_supported = Some(true);
            config.interaction.parallel_tool_calls = mode;
            let audit = std::env::temp_dir().join(format!(
                "timem-negotiation-known-native-{mode:?}-{protocol:?}.json"
            ));
            let mut client = ProbeClient { calls: 0 };

            let outcome =
                negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);
            let expected = mode == ParallelToolCalls::Enabled;
            assert_eq!(outcome.profile.parallel_enabled, expected, "{protocol:?}");
            assert_eq!(outcome.profile.parallel_supported, expected, "{protocol:?}");
            assert!(!outcome.probe_performed);
            assert_eq!(client.calls, 0);
        }
    }
}

#[test]
fn known_native_auto_reuses_exact_parallel_control_fallback_record() {
    for protocol in [
        ApiProtocol::OpenAiCompatible,
        ApiProtocol::OpenAiResponses,
        ApiProtocol::Anthropic,
    ] {
        let mut config = auto_config(&format!("parallel-fallback-cache-{protocol:?}"));
        config.api_protocol = protocol;
        config.interaction.native_tools_supported = Some(true);
        config.interaction.capability_probe_endpoint_id =
            Some(format!("parallel-fallback-cache-{protocol:?}"));
        config.interaction.persisted_capability_probe = Some(PersistedCapabilityProbe {
            identity: capability_probe_identity(&config).unwrap(),
            native_supported: true,
            parallel_supported: false,
            observed_tool_calls: 2,
            reason: PARALLEL_CONTROL_UNSUPPORTED_REASON.to_string(),
        });
        let audit = std::env::temp_dir().join(format!(
            "timem-negotiation-parallel-fallback-cache-{protocol:?}.json"
        ));
        let mut client = ProbeClient { calls: 0 };

        let outcome =
            negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

        assert_eq!(outcome.profile.source, CapabilityProbeSource::Cache);
        assert_eq!(outcome.profile.resolved_mode, ToolCallMode::Native);
        assert!(!outcome.profile.parallel_supported);
        assert!(!outcome.profile.parallel_enabled);
        assert_eq!(outcome.profile.reason, PARALLEL_CONTROL_UNSUPPORTED_REASON);
        assert!(!outcome.probe_performed);
        assert_eq!(client.calls, 0);
    }
}

#[test]
fn explicit_parallel_enabled_ignores_auto_fallback_record() {
    let mut config = auto_config("parallel-fallback-explicit-enabled");
    config.api_protocol = ApiProtocol::OpenAiResponses;
    config.interaction.native_tools_supported = Some(true);
    config.interaction.parallel_tool_calls = ParallelToolCalls::Enabled;
    config.interaction.capability_probe_endpoint_id =
        Some("parallel-fallback-explicit-enabled".to_string());
    config.interaction.persisted_capability_probe = Some(PersistedCapabilityProbe {
        identity: capability_probe_identity(&config).unwrap(),
        native_supported: true,
        parallel_supported: false,
        observed_tool_calls: 2,
        reason: PARALLEL_CONTROL_UNSUPPORTED_REASON.to_string(),
    });
    let audit =
        std::env::temp_dir().join("timem-negotiation-parallel-fallback-explicit-enabled.json");
    let mut client = ProbeClient { calls: 0 };

    let outcome = negotiate_interaction_outcome(&mut client, &config, &audit, &mut || false, false);

    assert_eq!(outcome.profile.source, CapabilityProbeSource::Explicit);
    assert!(outcome.profile.parallel_enabled);
    assert!(outcome.profile.parallel_supported);
    assert_eq!(client.calls, 0);
}

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
    let fallback = negotiate_interaction(&mut failing_client, &config, &audit, &mut || false);
    assert_eq!(fallback.resolved_mode, ToolCallMode::Inline);
    assert_eq!(fallback.source, CapabilityProbeSource::Fallback);
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
    assert_eq!(cancelled.resolved_mode, ToolCallMode::Inline);
    assert_eq!(cancelled.source, CapabilityProbeSource::Fallback);
    assert_eq!(cancelled.reason, "native_probe_failed:cancelled_by_user");
    assert_eq!(client.calls, 1);

    let recovered = negotiate_interaction(&mut client, &config, &audit, &mut || false);
    assert_eq!(recovered.resolved_mode, ToolCallMode::Native);
    assert_eq!(recovered.source, CapabilityProbeSource::Probe);
    assert_eq!(recovered.active_prompt_protocol, "json");
    assert_eq!(client.calls, 3);
}

use crate::{
    CapabilityProbeIdentity, CapabilityProbeSource, InteractionProfile, ModelClient,
    ModelInteractionRequest, ModelServiceConfig, NativeToolChoice, ParallelToolCalls,
    PersistedCapabilityProbe, ToolCallMode, ToolDefinition,
};
use serde_json::json;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

const PROBE_TOOL_NAME: &str = "timem_capability_probe";
const SINGLE_PROBE_PROMPT: &str =
    "Call the provided capability probe exactly once with slot=1. Do not answer in text.";
pub(crate) const PARALLEL_CONTROL_UNSUPPORTED_REASON: &str =
    "formal_request_parallel_control_unsupported";
const PARALLEL_PROBE_PROMPT: &str = "Call the provided capability probe twice in the same response, once with slot=1 and once with slot=2. Do not answer in text.";

#[derive(Clone, Eq)]
struct ProbeKey {
    endpoint_id: Option<String>,
    protocol: String,
    gateway: String,
    model: String,
    enable_thinking: Option<bool>,
    reasoning_effort: Option<String>,
}

impl PartialEq for ProbeKey {
    fn eq(&self, other: &Self) -> bool {
        self.endpoint_id == other.endpoint_id
            && self.protocol == other.protocol
            && self.gateway == other.gateway
            && self.model == other.model
            && self.enable_thinking == other.enable_thinking
            && self.reasoning_effort == other.reasoning_effort
    }
}

impl Hash for ProbeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.endpoint_id.hash(state);
        self.protocol.hash(state);
        self.gateway.hash(state);
        self.model.hash(state);
        self.enable_thinking.hash(state);
        self.reasoning_effort.hash(state);
    }
}

enum ProbeState {
    Running,
    Ready {
        profile: InteractionProfile,
        expires_at: Option<Instant>,
    },
}

enum ProbeCacheDisposition {
    Permanent,
    None,
}

struct ProbeOutcome {
    profile: InteractionProfile,
    cache: ProbeCacheDisposition,
    native_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiationOutcome {
    pub profile: InteractionProfile,
    /// Present only when this call completed a durable, non-transient probe.
    pub persisted_probe: Option<PersistedCapabilityProbe>,
    pub probe_performed: bool,
}

struct ProbeCache {
    entries: Mutex<HashMap<ProbeKey, ProbeState>>,
    ready: Condvar,
}

fn cache() -> &'static ProbeCache {
    static CACHE: OnceLock<ProbeCache> = OnceLock::new();
    CACHE.get_or_init(|| ProbeCache {
        entries: Mutex::new(HashMap::new()),
        ready: Condvar::new(),
    })
}

pub fn negotiate_interaction(
    model_client: &mut dyn ModelClient,
    config: &ModelServiceConfig,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> InteractionProfile {
    negotiate_interaction_outcome(model_client, config, audit_file, should_cancel, false).profile
}

pub fn force_reprobe_interaction(
    model_client: &mut dyn ModelClient,
    config: &ModelServiceConfig,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> NegotiationOutcome {
    negotiate_interaction_outcome(model_client, config, audit_file, should_cancel, true)
}

pub fn negotiate_interaction_outcome(
    model_client: &mut dyn ModelClient,
    config: &ModelServiceConfig,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
    force_probe: bool,
) -> NegotiationOutcome {
    negotiate_interaction_outcome_with_observer(
        model_client,
        config,
        audit_file,
        should_cancel,
        force_probe,
        &mut || {},
    )
}

pub(crate) fn negotiate_interaction_outcome_with_observer(
    model_client: &mut dyn ModelClient,
    config: &ModelServiceConfig,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
    force_probe: bool,
    on_probe_started: &mut dyn FnMut(),
) -> NegotiationOutcome {
    if config.interaction.tool_call_mode == ToolCallMode::Inline {
        return NegotiationOutcome {
            profile: explicit_inline_profile(config),
            persisted_probe: None,
            probe_performed: false,
        };
    }
    if config.interaction.tool_call_mode == ToolCallMode::Native
        || config.api_protocol == crate::ApiProtocol::OpenAiResponses
        || config.interaction.native_tools_supported == Some(true)
    {
        // Known-native paths intentionally skip proactive probes. Reuse only the
        // exact durable Auto fallback that says native tools work but the optional
        // parallel-control field does not. Explicit Enabled must still surface a
        // provider rejection instead of being silently downgraded.
        if config.interaction.parallel_tool_calls == ParallelToolCalls::Auto {
            if let (Some(expected), Some(persisted)) = (
                capability_probe_identity(config).as_ref(),
                config.interaction.persisted_capability_probe.as_ref(),
            ) {
                if &persisted.identity == expected
                    && persisted.native_supported
                    && !persisted.parallel_supported
                    && persisted.reason == PARALLEL_CONTROL_UNSUPPORTED_REASON
                {
                    return NegotiationOutcome {
                        profile: profile_from_capabilities(
                            config,
                            true,
                            false,
                            persisted.observed_tool_calls,
                            CapabilityProbeSource::Cache,
                            persisted.reason.clone(),
                            None,
                        ),
                        persisted_probe: None,
                        probe_performed: false,
                    };
                }
            }
        }
        return NegotiationOutcome {
            profile: assumed_native_profile(config),
            persisted_probe: None,
            probe_performed: false,
        };
    }
    if config.interaction.native_tools_supported == Some(false) {
        return NegotiationOutcome {
            profile: known_unsupported_profile(config),
            persisted_probe: None,
            probe_performed: false,
        };
    }

    let identity = capability_probe_identity(config);
    if !force_probe {
        if let (Some(expected), Some(persisted)) = (
            identity.as_ref(),
            config.interaction.persisted_capability_probe.as_ref(),
        ) {
            if &persisted.identity == expected {
                return NegotiationOutcome {
                    profile: profile_from_capabilities(
                        config,
                        persisted.native_supported,
                        persisted.parallel_supported,
                        persisted.observed_tool_calls,
                        CapabilityProbeSource::Cache,
                        persisted.reason.clone(),
                        None,
                    ),
                    persisted_probe: None,
                    probe_performed: false,
                };
            }
        }
    }

    let key = probe_key(config);
    let cache = cache();
    let mut entries = cache
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        match entries.get(&key) {
            Some(ProbeState::Running) => {
                entries = cache
                    .ready
                    .wait(entries)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            Some(ProbeState::Ready {
                profile,
                expires_at,
            }) if !force_probe && expires_at.is_none_or(|deadline| deadline > Instant::now()) => {
                let mut cached = profile.clone();
                cached.source = CapabilityProbeSource::Cache;
                cached.active_prompt_protocol =
                    active_prompt_protocol(config, cached.resolved_mode).to_string();
                return NegotiationOutcome {
                    profile: cached,
                    persisted_probe: None,
                    probe_performed: false,
                };
            }
            Some(ProbeState::Ready { .. }) | None => {
                entries.insert(key.clone(), ProbeState::Running);
                break;
            }
        }
    }
    drop(entries);

    // This callback is deliberately after the process-wide probe slot is
    // acquired: waiters and cache hits must never report that they probed.
    on_probe_started();
    let outcome = run_probe(model_client, config, audit_file, should_cancel);
    let persisted_probe = match (&outcome.cache, identity) {
        (ProbeCacheDisposition::Permanent, Some(identity)) => Some(PersistedCapabilityProbe {
            identity,
            native_supported: outcome.native_supported,
            parallel_supported: outcome.profile.parallel_supported,
            observed_tool_calls: outcome.profile.observed_tool_calls,
            reason: outcome.profile.reason.clone(),
        }),
        _ => None,
    };
    let mut entries = cache
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match outcome.cache {
        ProbeCacheDisposition::Permanent => {
            entries.insert(
                key,
                ProbeState::Ready {
                    profile: outcome.profile.clone(),
                    expires_at: None,
                },
            );
        }
        ProbeCacheDisposition::None => {
            entries.remove(&key);
        }
    }
    cache.ready.notify_all();
    NegotiationOutcome {
        profile: outcome.profile,
        persisted_probe,
        probe_performed: true,
    }
}

pub fn capability_probe_identity(config: &ModelServiceConfig) -> Option<CapabilityProbeIdentity> {
    let endpoint_id = config
        .interaction
        .capability_probe_endpoint_id
        .as_deref()?
        .trim();
    if endpoint_id.is_empty() {
        return None;
    }
    Some(CapabilityProbeIdentity {
        endpoint_id: endpoint_id.to_string(),
        api_protocol: config.api_protocol.label().to_string(),
        gateway: normalized_gateway(&config.base_url),
        model: config.model.clone(),
        enable_thinking: config.openai_compatible.enable_thinking,
        reasoning_effort: config.openai_compatible.reasoning_effort.clone(),
    })
}

fn probe_key(config: &ModelServiceConfig) -> ProbeKey {
    ProbeKey {
        endpoint_id: config.interaction.capability_probe_endpoint_id.clone(),
        protocol: config.api_protocol.label().to_string(),
        gateway: normalized_gateway(&config.base_url),
        model: config.model.clone(),
        enable_thinking: config.openai_compatible.enable_thinking,
        reasoning_effort: config.openai_compatible.reasoning_effort.clone(),
    }
}

fn run_probe(
    model_client: &mut dyn ModelClient,
    config: &ModelServiceConfig,
    audit_file: &Path,
    should_cancel: &mut dyn FnMut() -> bool,
) -> ProbeOutcome {
    let started = Instant::now();
    let requested = config.interaction.tool_call_mode;
    let single = probe_request(SINGLE_PROBE_PROMPT, false);
    let single_result =
        model_client.call_model_interaction(config, &single, audit_file, should_cancel);

    if !single_result
        .as_ref()
        .is_ok_and(|response| !response.tool_calls.is_empty())
    {
        let explicitly_unsupported = single_result
            .as_ref()
            .err()
            .is_some_and(|error| crate::retry_policy::is_explicit_native_tools_unsupported(error));
        let native_supported = !explicitly_unsupported;
        let resolved_mode = if explicitly_unsupported && requested == ToolCallMode::Auto {
            ToolCallMode::Inline
        } else {
            ToolCallMode::Native
        };
        return ProbeOutcome {
            profile: InteractionProfile {
                api_protocol: config.api_protocol.label().to_string(),
                model: config.model.clone(),
                gateway: normalized_gateway(&config.base_url),
                requested_mode: requested,
                resolved_mode,
                active_prompt_protocol: active_prompt_protocol(config, resolved_mode).to_string(),
                parallel_supported: false,
                parallel_enabled: false,
                source: if requested == ToolCallMode::Native {
                    CapabilityProbeSource::Explicit
                } else if explicitly_unsupported {
                    CapabilityProbeSource::Fallback
                } else {
                    CapabilityProbeSource::Probe
                },
                reason: if explicitly_unsupported {
                    probe_failure_reason(single_result)
                } else {
                    inconclusive_probe_reason(single_result)
                },
                probe_latency_ms: Some(elapsed_millis(started)),
                observed_tool_calls: 0,
            },
            cache: if explicitly_unsupported {
                ProbeCacheDisposition::Permanent
            } else {
                // A transport/auth/server failure, cancellation, generic 4xx,
                // malformed response, or a successful text-only response is
                // not evidence that native tools are unsupported.
                ProbeCacheDisposition::None
            },
            native_supported,
        };
    }

    let parallel_result = if config.interaction.parallel_tool_calls == ParallelToolCalls::Disabled {
        None
    } else {
        Some(model_client.call_model_interaction(
            config,
            &probe_request(PARALLEL_PROBE_PROMPT, true),
            audit_file,
            should_cancel,
        ))
    };
    let observed_tool_calls = parallel_result
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|response| response.tool_calls.len())
        .unwrap_or(1);
    let parallel_supported = observed_tool_calls >= 2;
    let parallel_explicitly_unsupported = parallel_result
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .is_some_and(|error| {
            error.to_ascii_lowercase().contains("parallel_tool_calls")
                && crate::retry_policy::is_explicit_native_tools_unsupported(error)
        });
    let parallel_enabled = match config.interaction.parallel_tool_calls {
        ParallelToolCalls::Disabled => false,
        ParallelToolCalls::Auto => parallel_supported,
        ParallelToolCalls::Enabled => true,
    };
    let conclusive_parallel_result = parallel_supported || parallel_explicitly_unsupported;
    ProbeOutcome {
        profile: InteractionProfile {
            api_protocol: config.api_protocol.label().to_string(),
            model: config.model.clone(),
            gateway: normalized_gateway(&config.base_url),
            requested_mode: requested,
            resolved_mode: ToolCallMode::Native,
            active_prompt_protocol: active_prompt_protocol(config, ToolCallMode::Native)
                .to_string(),
            parallel_supported,
            parallel_enabled,
            source: if requested == ToolCallMode::Native {
                CapabilityProbeSource::Explicit
            } else {
                CapabilityProbeSource::Probe
            },
            reason: if parallel_supported {
                "native_and_parallel_probe_succeeded".to_string()
            } else if parallel_explicitly_unsupported {
                "native_probe_succeeded_parallel_explicitly_unsupported".to_string()
            } else if let Some(result) = parallel_result {
                format!(
                    "native_probe_succeeded_parallel_probe_inconclusive:{}",
                    compact_reason(&probe_result_reason(result))
                )
            } else {
                "native_probe_succeeded_parallel_not_tested".to_string()
            },
            probe_latency_ms: Some(elapsed_millis(started)),
            observed_tool_calls,
        },
        cache: if conclusive_parallel_result {
            ProbeCacheDisposition::Permanent
        } else {
            ProbeCacheDisposition::None
        },
        native_supported: true,
    }
}

fn probe_request(prompt: &str, parallel: bool) -> ModelInteractionRequest {
    ModelInteractionRequest {
        rendered_prompt: prompt.to_string(),
        images: Vec::new(),
        static_tool_count: 1,
        tools: vec![ToolDefinition {
            name: PROBE_TOOL_NAME.to_string(),
            description: "Records one capability-negotiation slot without side effects."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"slot": {"type": "integer", "enum": [1, 2]}},
                "required": ["slot"],
                "additionalProperties": false,
            }),
        }],
        native_exchanges: Vec::new(),
        resolved_mode: ToolCallMode::Native,
        parallel_tool_calls: parallel,
        send_parallel_tool_calls: true,
        tool_choice: NativeToolChoice::Required,
        critical_reasoning: false,
    }
}

fn profile_from_capabilities(
    config: &ModelServiceConfig,
    native_supported: bool,
    parallel_supported: bool,
    observed_tool_calls: usize,
    source: CapabilityProbeSource,
    reason: String,
    probe_latency_ms: Option<u64>,
) -> InteractionProfile {
    let requested_mode = config.interaction.tool_call_mode;
    let resolved_mode = if native_supported || requested_mode == ToolCallMode::Native {
        ToolCallMode::Native
    } else {
        ToolCallMode::Inline
    };
    let parallel_enabled = native_supported
        && match config.interaction.parallel_tool_calls {
            ParallelToolCalls::Disabled => false,
            ParallelToolCalls::Auto => parallel_supported,
            ParallelToolCalls::Enabled => true,
        };
    InteractionProfile {
        api_protocol: config.api_protocol.label().to_string(),
        model: config.model.clone(),
        gateway: normalized_gateway(&config.base_url),
        requested_mode,
        resolved_mode,
        active_prompt_protocol: active_prompt_protocol(config, resolved_mode).to_string(),
        parallel_supported: native_supported && parallel_supported,
        parallel_enabled,
        source,
        reason,
        probe_latency_ms,
        observed_tool_calls,
    }
}

fn assumed_native_profile(config: &ModelServiceConfig) -> InteractionProfile {
    // Native-capable endpoints are assumed to implement the standard provider
    // parallel-control field unless the user explicitly disables parallelism.
    // Auto must not silently become serial merely because this path skips the
    // capability probe (Responses and catalog-known native endpoints do).
    let parallel_enabled = config.interaction.parallel_tool_calls != ParallelToolCalls::Disabled;
    InteractionProfile {
        api_protocol: config.api_protocol.label().to_string(),
        model: config.model.clone(),
        gateway: normalized_gateway(&config.base_url),
        requested_mode: config.interaction.tool_call_mode,
        resolved_mode: ToolCallMode::Native,
        active_prompt_protocol: active_prompt_protocol(config, ToolCallMode::Native).to_string(),
        parallel_supported: parallel_enabled,
        parallel_enabled,
        source: CapabilityProbeSource::Explicit,
        reason: if config.interaction.tool_call_mode == ToolCallMode::Native {
            "native_selected_by_configuration".to_string()
        } else if config.api_protocol == crate::ApiProtocol::OpenAiResponses {
            "agent_native_protocol_assumed_supported".to_string()
        } else {
            "known_model_capability_assumed_supported".to_string()
        },
        probe_latency_ms: None,
        observed_tool_calls: 0,
    }
}

fn known_unsupported_profile(config: &ModelServiceConfig) -> InteractionProfile {
    InteractionProfile {
        api_protocol: config.api_protocol.label().to_string(),
        model: config.model.clone(),
        gateway: normalized_gateway(&config.base_url),
        requested_mode: config.interaction.tool_call_mode,
        resolved_mode: ToolCallMode::Inline,
        active_prompt_protocol: active_prompt_protocol(config, ToolCallMode::Inline).to_string(),
        parallel_supported: false,
        parallel_enabled: false,
        source: CapabilityProbeSource::Explicit,
        reason: "native_tools_disabled_by_endpoint_configuration".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 0,
    }
}

fn explicit_inline_profile(config: &ModelServiceConfig) -> InteractionProfile {
    InteractionProfile {
        api_protocol: config.api_protocol.label().to_string(),
        model: config.model.clone(),
        gateway: normalized_gateway(&config.base_url),
        requested_mode: ToolCallMode::Inline,
        resolved_mode: ToolCallMode::Inline,
        active_prompt_protocol: active_prompt_protocol(config, ToolCallMode::Inline).to_string(),
        parallel_supported: false,
        parallel_enabled: false,
        source: CapabilityProbeSource::Explicit,
        reason: "inline_selected_by_configuration".to_string(),
        probe_latency_ms: None,
        observed_tool_calls: 0,
    }
}

fn active_prompt_protocol(config: &ModelServiceConfig, mode: ToolCallMode) -> &'static str {
    if mode == ToolCallMode::Native {
        "json"
    } else {
        config.response_protocol.name()
    }
}

fn probe_failure_reason(result: Result<crate::LlmResponse, String>) -> String {
    match result {
        Ok(_) => "native_probe_returned_no_tool_calls".to_string(),
        Err(error) => format!("native_probe_failed:{}", compact_reason(&error)),
    }
}

fn inconclusive_probe_reason(result: Result<crate::LlmResponse, String>) -> String {
    format!(
        "native_probe_inconclusive:{}",
        compact_reason(&probe_result_reason(result))
    )
}

fn probe_result_reason(result: Result<crate::LlmResponse, String>) -> String {
    match result {
        Ok(_) => "returned_no_tool_calls".to_string(),
        Err(error) => error,
    }
}

fn compact_reason(reason: &str) -> String {
    reason
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

fn normalized_gateway(base_url: &str) -> String {
    let without_suffix = base_url
        .split(['?', '#'])
        .next()
        .unwrap_or(base_url)
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let Some((scheme, remainder)) = without_suffix.split_once("://") else {
        return without_suffix;
    };
    let (authority, path) = remainder
        .split_once('/')
        .map(|(authority, path)| (authority, format!("/{path}")))
        .unwrap_or((remainder, String::new()));
    let authority = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    format!("{scheme}://{authority}{path}")
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "../tests/unit/negotiation_tests.rs"]
mod tests;

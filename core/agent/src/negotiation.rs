use crate::{
    CapabilityProbeSource, InteractionProfile, ModelClient, ModelInteractionRequest,
    ModelServiceConfig, NativeToolChoice, ParallelToolCalls, ToolCallMode, ToolDefinition,
};
use serde_json::json;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

const PROBE_TOOL_NAME: &str = "timem_capability_probe";
const SINGLE_PROBE_PROMPT: &str =
    "Call the provided capability probe exactly once with slot=1. Do not answer in text.";
const PARALLEL_PROBE_PROMPT: &str = "Call the provided capability probe twice in the same response, once with slot=1 and once with slot=2. Do not answer in text.";
#[cfg(not(test))]
const TRANSIENT_PROBE_FAILURE_TTL: Duration = Duration::from_secs(30);
#[cfg(test)]
const TRANSIENT_PROBE_FAILURE_TTL: Duration = Duration::ZERO;

#[derive(Clone, Eq)]
struct ProbeKey {
    protocol: &'static str,
    gateway: String,
    model: String,
    enable_thinking: Option<bool>,
    reasoning_effort: Option<String>,
}

impl PartialEq for ProbeKey {
    fn eq(&self, other: &Self) -> bool {
        self.protocol == other.protocol
            && self.gateway == other.gateway
            && self.model == other.model
            && self.enable_thinking == other.enable_thinking
            && self.reasoning_effort == other.reasoning_effort
    }
}

impl Hash for ProbeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
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
    Transient(Duration),
    None,
}

struct ProbeOutcome {
    profile: InteractionProfile,
    cache: ProbeCacheDisposition,
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
    if config.interaction.tool_call_mode == ToolCallMode::Inline {
        return explicit_inline_profile(config);
    }

    let key = ProbeKey {
        protocol: config.api_protocol.label(),
        gateway: normalized_gateway(&config.base_url),
        model: config.model.clone(),
        enable_thinking: config.openai_compatible.enable_thinking,
        reasoning_effort: config.openai_compatible.reasoning_effort.clone(),
    };
    let cache = cache();
    let mut entries = cache
        .entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        match entries.get(&key) {
            Some(ProbeState::Ready {
                profile,
                expires_at,
            }) if expires_at.is_none_or(|deadline| deadline > Instant::now()) => {
                let mut cached = profile.clone();
                cached.source = CapabilityProbeSource::Cache;
                cached.active_prompt_protocol =
                    active_prompt_protocol(config, cached.resolved_mode).to_string();
                return cached;
            }
            Some(ProbeState::Ready { .. }) => {
                entries.insert(key.clone(), ProbeState::Running);
                break;
            }
            Some(ProbeState::Running) => {
                entries = cache
                    .ready
                    .wait(entries)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            None => {
                entries.insert(key.clone(), ProbeState::Running);
                break;
            }
        }
    }
    drop(entries);

    let outcome = run_probe(model_client, config, audit_file, should_cancel);
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
        ProbeCacheDisposition::Transient(ttl) => {
            entries.insert(
                key,
                ProbeState::Ready {
                    profile: outcome.profile.clone(),
                    expires_at: Some(Instant::now() + ttl),
                },
            );
        }
        ProbeCacheDisposition::None => {
            entries.remove(&key);
        }
    }
    cache.ready.notify_all();
    outcome.profile
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
    let native_supported = single_result
        .as_ref()
        .is_ok_and(|response| !response.tool_calls.is_empty());
    if !native_supported {
        let cancelled = single_result
            .as_ref()
            .err()
            .is_some_and(|error| error.trim().eq_ignore_ascii_case("cancelled_by_user"));
        let transient_failure = single_result
            .as_ref()
            .err()
            .is_some_and(|error| crate::retry_policy::is_retryable_model_system_error(error));
        let resolved_mode = if requested == ToolCallMode::Native {
            ToolCallMode::Native
        } else {
            ToolCallMode::Inline
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
                } else {
                    CapabilityProbeSource::Fallback
                },
                reason: probe_failure_reason(single_result),
                probe_latency_ms: Some(elapsed_millis(started)),
                observed_tool_calls: 0,
            },
            cache: if cancelled {
                // Cancellation says nothing about provider capability. Returning
                // the fallback profile lets the cancelled turn unwind, while
                // removing the cache entry makes the next turn probe again.
                ProbeCacheDisposition::None
            } else if transient_failure {
                ProbeCacheDisposition::Transient(TRANSIENT_PROBE_FAILURE_TTL)
            } else {
                ProbeCacheDisposition::Permanent
            },
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
    let parallel_enabled = match config.interaction.parallel_tool_calls {
        ParallelToolCalls::Disabled => false,
        ParallelToolCalls::Auto => parallel_supported,
        ParallelToolCalls::Enabled => true,
    };
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
            } else {
                "native_probe_succeeded_parallel_not_observed".to_string()
            },
            probe_latency_ms: Some(elapsed_millis(started)),
            observed_tool_calls,
        },
        cache: ProbeCacheDisposition::Permanent,
    }
}

fn probe_request(prompt: &str, parallel: bool) -> ModelInteractionRequest {
    ModelInteractionRequest {
        rendered_prompt: prompt.to_string(),
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
        tool_choice: NativeToolChoice::Required,
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

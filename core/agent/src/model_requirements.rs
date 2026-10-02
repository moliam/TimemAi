//! Protocol-neutral endpoint contract and per-call requirements.
//! No provider payload field names or URL heuristics belong in this layer.
use crate::model_catalog::CatalogModel;
use crate::reasoning::{EffectiveReasoning, ReasoningDemand, ReasoningPreference};
use crate::ModelServiceConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointRequirements {
    /// Zero denotes legacy policy, one denotes explicit daily/adaptive policy.
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub provider: Option<String>,
    /// None means unknown; an empty declared set is invalid.
    #[serde(default)]
    pub allowed_reasoning: Option<Vec<String>>,
    #[serde(default)]
    pub adaptive_reasoning: Option<bool>,
    /// Template suggestions vs explicit user edits. Never contains credentials.
    #[serde(default)]
    pub field_sources: BTreeMap<String, FieldSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSource {
    Template,
    User,
}

pub fn provider(config: &ModelServiceConfig) -> Option<&str> {
    config
        .openai_compatible
        .requirements
        .provider
        .as_deref()
        .or_else(|| {
            if config.openai_compatible.requirements.version > 0 {
                return None;
            }
            config
                .openai_compatible
                .catalog_id
                .as_deref()
                .and_then(|id| crate::model_catalog::models().iter().find(|m| m.id == id))
                .map(|m| m.provider.as_str())
        })
}

pub fn capabilities(config: &ModelServiceConfig) -> Option<&'static CatalogModel> {
    let provider = provider(config)?;
    crate::model_catalog::models()
        .iter()
        .find(|m| m.provider == provider && m.model == config.model)
}

impl EndpointRequirements {
    pub fn validate(&self) -> Result<(), String> {
        if self.version > 1 {
            return Err("unsupported_model_requirements_version".into());
        }
        if let Some(provider) = &self.provider {
            if !matches!(provider.as_str(), "openai" | "zhipu") {
                return Err("model_provider_adapter_not_implemented".into());
            }
        }
        if let Some(levels) = &self.allowed_reasoning {
            if levels.is_empty()
                || levels.len() > 16
                || levels.iter().enumerate().any(|(i, v)| {
                    v.is_empty()
                        || v.len() > 32
                        || !v
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                        || levels[..i].contains(v)
                })
            {
                return Err("invalid_allowed_reasoning".into());
            }
        }
        const FIELDS: &[&str] = &[
            "name",
            "model",
            "provider",
            "api_protocol",
            "base_url",
            "stream",
            "reasoning_effort",
            "allowed_reasoning",
            "adaptive_reasoning",
            "max_llm_input_tokens",
            "max_llm_output_tokens",
        ];
        if self
            .field_sources
            .keys()
            .any(|k| !FIELDS.contains(&k.as_str()))
        {
            return Err("invalid_endpoint_field_source".into());
        }
        Ok(())
    }
}

pub fn validate_config(config: &ModelServiceConfig) -> Result<(), String> {
    let settings = &config.openai_compatible.requirements;
    settings.validate()?;
    if let Some(p) = provider(config) {
        if (p == "zhipu" && config.api_protocol != crate::ApiProtocol::OpenAiCompatible)
            || (p == "openai" && config.api_protocol == crate::ApiProtocol::Anthropic)
        {
            return Err("provider_protocol_adapter_not_implemented".into());
        }
    }
    let selected = if config.openai_compatible.enable_thinking == Some(false) {
        Some("none")
    } else {
        config
            .openai_compatible
            .reasoning_effort
            .as_deref()
            .map(|e| if e == "disabled" { "none" } else { e })
    };
    let selected = selected.or_else(|| {
        if settings.version > 0 {
            capabilities(config).map(|m| m.default_effort.as_str())
        } else {
            None
        }
    });
    if let Some(allowed) = &settings.allowed_reasoning {
        if selected.is_some_and(|e| !allowed.iter().any(|v| v == e)) {
            return Err("daily_reasoning_not_in_allowed_set".into());
        }
    }
    if let Some(model) = capabilities(config) {
        if let Some(allowed) = &settings.allowed_reasoning {
            if allowed.iter().any(|e| !model.efforts.contains(e)) {
                return Err("allowed_reasoning_exceeds_model_capability".into());
            }
        }
        crate::model_catalog::validate(
            &model.id,
            &config.model,
            config.api_protocol.label(),
            selected,
            config.max_llm_input_tokens,
            config.max_llm_output_tokens,
        )?;
    } else if provider(config) == Some("zhipu") {
        // Zhipu has model-dependent disable semantics; require a descriptor.
        return Err("model_capabilities_not_declared".into());
    }
    Ok(())
}

pub fn resolve_reasoning(config: &ModelServiceConfig, critical: bool) -> EffectiveReasoning {
    let settings = &config.openai_compatible.requirements;
    let model = capabilities(config);
    let preference = ReasoningPreference::from_legacy(
        config.openai_compatible.enable_thinking,
        config.openai_compatible.reasoning_effort.as_deref(),
    );
    if settings.version == 0 {
        return preference.resolve(
            if critical || config.openai_compatible.catalog_id.is_some() {
                ReasoningDemand::Required
            } else {
                ReasoningDemand::Ordinary
            },
        );
    }
    let daily = match preference {
        ReasoningPreference::Unspecified => model.map(|m| m.default_effort.as_str()),
        ReasoningPreference::Disabled => Some("none"),
        ReasoningPreference::Enabled { .. } => config
            .openai_compatible
            .reasoning_effort
            .as_deref()
            .or_else(|| model.map(|m| m.default_effort.as_str())),
    };
    // An unset daily value still has to respect the declared capability subset.
    // Validation rejects an excluded default rather than silently selecting one.
    // Ordering is model-declared, not UI checkbox order. Without a descriptor,
    // a declared list is the user's ordered capability statement.
    let levels: Vec<String> = match model {
        Some(m) => m
            .efforts
            .iter()
            .filter(|e| {
                settings
                    .allowed_reasoning
                    .as_ref()
                    .is_none_or(|a| a.contains(e))
            })
            .cloned()
            .collect(),
        None => settings.allowed_reasoning.clone().unwrap_or_default(),
    };
    let fixed = model
        .and_then(|m| {
            m.protocols
                .iter()
                .find(|p| p.protocol == config.api_protocol.label())
        })
        .and_then(|p| p.fixed_effort.as_deref());
    crate::reasoning::resolve_daily(
        daily,
        config.openai_compatible.enable_thinking == Some(true),
        settings.adaptive_reasoning,
        &levels,
        critical && fixed.is_none(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasoningUpgrade {
    pub from: String,
    pub to: String,
}

/// UI-neutral projection: only a real, known upward change is a notice.
pub fn reasoning_upgrade(config: &ModelServiceConfig, critical: bool) -> Option<ReasoningUpgrade> {
    if !critical || config.openai_compatible.requirements.version == 0 {
        return None;
    }
    let label = |r: EffectiveReasoning| match r {
        EffectiveReasoning::Disabled => Some("none".to_owned()),
        EffectiveReasoning::Enabled { intensity } => intensity,
        EffectiveReasoning::Unspecified => None,
    };
    let from = label(resolve_reasoning(config, false))?;
    let to = label(resolve_reasoning(config, true))?;
    (from != to).then_some(ReasoningUpgrade { from, to })
}

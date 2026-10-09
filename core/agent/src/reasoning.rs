//! Protocol-independent reasoning policy. No wire-field names belong here.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningPreference {
    Unspecified,
    Disabled,
    Enabled { intensity: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningDemand {
    Ordinary,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveReasoning {
    Unspecified,
    Disabled,
    Enabled { intensity: Option<String> },
}

impl ReasoningPreference {
    /// Compatibility boundary for existing persisted settings. Either explicit
    /// disable wins; an intensity alone is an opt-in. Unknown provider levels
    /// remain intact for upstream validation, never silently downgraded.
    pub fn from_legacy(enabled: Option<bool>, intensity: Option<&str>) -> Self {
        if enabled == Some(false) || matches!(intensity, Some("disabled" | "none")) {
            Self::Disabled
        } else if enabled == Some(true) || intensity.is_some() {
            Self::Enabled {
                intensity: intensity.map(str::to_owned),
            }
        } else {
            Self::Unspecified
        }
    }

    pub fn resolve(&self, demand: ReasoningDemand) -> EffectiveReasoning {
        match self {
            Self::Unspecified => EffectiveReasoning::Unspecified,
            Self::Disabled => EffectiveReasoning::Disabled,
            Self::Enabled { .. } if demand == ReasoningDemand::Ordinary => {
                EffectiveReasoning::Disabled
            }
            Self::Enabled { intensity } => EffectiveReasoning::Enabled {
                intensity: intensity.clone(),
            },
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/reasoning_tests.rs"]
mod tests;

/// H0 always applies. H1 selects the second-highest declared eligible level,
/// never downgrades, and cannot invent an ordering when capabilities are unknown.
pub fn resolve_daily(
    daily: Option<&str>,
    explicitly_enabled: bool,
    adaptive: Option<bool>,
    ordered: &[String],
    critical: bool,
) -> EffectiveReasoning {
    let disabled = matches!(daily, Some("none" | "disabled"));
    let baseline = if disabled {
        EffectiveReasoning::Disabled
    } else if daily.is_some() || explicitly_enabled {
        EffectiveReasoning::Enabled {
            intensity: daily.map(str::to_owned),
        }
    } else {
        EffectiveReasoning::Unspecified
    };
    if !critical || !adaptive.unwrap_or(!disabled && (daily.is_some() || explicitly_enabled)) {
        return baseline;
    }
    let start = if disabled {
        0
    } else if let Some(daily) = daily {
        let Some(index) = ordered.iter().position(|v| v == daily) else {
            return baseline;
        };
        index
    } else {
        return baseline;
    };
    let eligible: Vec<_> = ordered
        .iter()
        .skip(start)
        .filter(|v| !matches!(v.as_str(), "none" | "disabled"))
        .collect();
    if eligible.is_empty() {
        return baseline;
    }
    EffectiveReasoning::Enabled {
        intensity: Some(eligible[eligible.len().saturating_sub(2)].clone()),
    }
}

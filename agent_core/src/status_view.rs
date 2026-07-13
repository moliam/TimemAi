use crate::{notification::CoreMemoryActivity, UsageStats};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDirection {
    Upstream,
    Downstream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStatusSnapshot {
    pub provider: String,
    pub model: String,
    pub intent: String,
    pub memory_activity: CoreMemoryActivity,
    pub model_round: u32,
    pub direction: ModelDirection,
    pub usage: UsageStats,
    pub latest_usage: Option<UsageStats>,
    pub tick: usize,
    pub elapsed_secs: u64,
    pub max_llm_input_tokens: u32,
    pub retry: Option<RuntimeRetryStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRetryStatus {
    pub until_epoch_ms: Option<u128>,
    pub error: Option<String>,
    pub attempt: Option<u32>,
    pub max_attempts: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRetryStatusView {
    pub remaining_secs: u64,
    pub attempt: u32,
    pub max_attempts: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatusLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostStatusMessage {
    pub level: HostStatusLevel,
    pub text: String,
}

impl HostStatusMessage {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            level: HostStatusLevel::Info,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            level: HostStatusLevel::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            level: HostStatusLevel::Error,
            text: text.into(),
        }
    }
}

pub fn runtime_active_elapsed_secs(total_elapsed: Duration, paused_total: Duration) -> u64 {
    total_elapsed.saturating_sub(paused_total).as_secs()
}

pub fn runtime_retry_status_view(
    retry: &RuntimeRetryStatus,
    now_epoch_ms: u128,
) -> RuntimeRetryStatusView {
    let remaining_ms = retry
        .until_epoch_ms
        .unwrap_or(now_epoch_ms)
        .saturating_sub(now_epoch_ms);
    RuntimeRetryStatusView {
        remaining_secs: remaining_ms.div_ceil(1000) as u64,
        attempt: retry.attempt.unwrap_or(1),
        max_attempts: retry.max_attempts.unwrap_or(5),
        error: retry
            .error
            .as_deref()
            .map(str::trim)
            .filter(|error| !error.is_empty())
            .map(ToString::to_string),
    }
}

pub fn compact_runtime_status_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let compacted = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>();
    if compacted.chars().count() <= max_chars {
        return compacted;
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let mut out = compacted
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
#[path = "../tests/unit/status_view_tests.rs"]
mod tests;

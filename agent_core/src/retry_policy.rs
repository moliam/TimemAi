use std::time::Duration;

pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRIES: u32 = 5;

#[cfg(not(test))]
pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY: Duration = Duration::from_secs(10);
#[cfg(test)]
pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY: Duration = Duration::ZERO;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSystemRetryPolicy {
    pub max_attempts: u32,
    pub delay: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCallOutcome<T> {
    pub response: T,
    pub model_wait: Duration,
    pub retry_wait: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelRetryDecision {
    pub retry_attempt: u32,
    pub max_attempts: u32,
    pub delay: Duration,
}

impl Default for ModelSystemRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MODEL_SYSTEM_ERROR_RETRIES,
            delay: DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY,
        }
    }
}

pub fn model_retry_decision(
    error: &str,
    attempt: u32,
    policy: ModelSystemRetryPolicy,
    is_cancelled: bool,
) -> Option<ModelRetryDecision> {
    if is_cancelled || !is_retryable_model_system_error(error) || attempt >= policy.max_attempts {
        return None;
    }
    Some(ModelRetryDecision {
        retry_attempt: attempt.saturating_add(1),
        max_attempts: policy.max_attempts,
        delay: policy.delay,
    })
}

pub fn is_retryable_model_system_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    if lower == "cancelled_by_user" {
        return false;
    }
    if lower.starts_with("provider_network_error")
        || lower.starts_with("provider_timeout")
        || lower.starts_with("curl_failed")
        || lower.contains("curl:")
        || lower.contains("http2 framing")
        || lower.contains("operation timed out")
        || lower.contains("connection reset")
        || lower.contains("could not resolve host")
    {
        return true;
    }
    if let Some(status_text) = lower.strip_prefix("provider_http_") {
        let status: u16 = status_text
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        return matches!(status, 408 | 409 | 425 | 429) || status >= 500;
    }
    false
}

pub fn is_model_input_too_large_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let input_subject =
        lower.contains("input") || lower.contains("prompt") || lower.contains("context");
    let size_subject = lower.contains("token") || lower.contains("length");
    let exceeds_limit = lower.contains("too long")
        || lower.contains("too large")
        || lower.contains("too many")
        || lower.contains("exceed");
    lower.contains("argument list too long")
        || lower.contains("os error 7")
        || lower.contains("e2big")
        || lower.starts_with("provider_http_413")
        || lower.contains("context_length_exceeded")
        || lower.contains("maximum context length")
        || lower.contains("max context length")
        || lower.contains("input context is too long")
        || lower.contains("input is too long")
        || lower.contains("input too long")
        || lower.contains("too many input tokens")
        || lower.contains("request body too large")
        || lower.contains("payload too large")
        || (input_subject && size_subject && exceeds_limit)
}

#[cfg(test)]
#[path = "../tests/unit/retry_policy_tests.rs"]
mod tests;

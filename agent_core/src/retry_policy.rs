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
mod tests {
    use super::*;

    #[test]
    fn retry_policy_defaults_match_user_visible_contract() {
        let policy = ModelSystemRetryPolicy::default();
        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.delay, Duration::ZERO);
    }

    #[test]
    fn retryable_model_system_errors_cover_network_and_transient_http() {
        for error in [
            "provider_network_error: curl: (16) Error in the HTTP2 framing layer",
            "provider_timeout: request exceeded timeout",
            "curl_failed",
            "curl: (28) operation timed out",
            "connection reset by peer",
            "could not resolve host: example.invalid",
            "provider_http_408: timeout",
            "provider_http_409: conflict",
            "provider_http_425: too early",
            "provider_http_429: rate limit",
            "provider_http_500: upstream overloaded",
            "provider_http_503",
        ] {
            assert!(is_retryable_model_system_error(error), "{error}");
        }
    }

    #[test]
    fn non_retryable_model_errors_do_not_waste_rounds() {
        for error in [
            "cancelled_by_user",
            "provider_http_400: invalid model",
            "provider_http_401: unauthorized",
            "provider_http_403: forbidden",
            "provider_http_404: model not found",
            "invalid_json",
            "status_required",
            "next_actions[0].args_required",
        ] {
            assert!(!is_retryable_model_system_error(error), "{error}");
        }
    }

    #[test]
    fn input_too_large_errors_are_detected_without_matching_unrelated_failures() {
        for error in [
            "Argument list too long (os error 7)",
            "E2BIG while spawning provider transport",
            "provider_http_413: payload too large",
            "provider_http_400: context_length_exceeded",
            "provider_http_400: maximum context length is 100000 tokens",
            "provider_http_400: too many input tokens",
            "provider_http_400: prompt is too long: 200001 tokens > 200000 maximum",
            "provider_http_400: input token length exceeds the model limit",
        ] {
            assert!(is_model_input_too_large_error(error), "{error}");
        }
        for error in [
            "provider_http_400: invalid model",
            "provider_http_401: unauthorized",
            "provider_http_500: overloaded",
            "output token limit exceeded",
        ] {
            assert!(!is_model_input_too_large_error(error), "{error}");
        }
    }

    #[test]
    fn retry_decision_is_structured_and_ui_neutral() {
        let policy = ModelSystemRetryPolicy {
            max_attempts: 5,
            delay: Duration::from_secs(10),
        };
        let decision =
            model_retry_decision("provider_http_503: overloaded", 0, policy, false).unwrap();
        assert_eq!(
            decision,
            ModelRetryDecision {
                retry_attempt: 1,
                max_attempts: 5,
                delay: Duration::from_secs(10),
            }
        );
        assert!(model_retry_decision("provider_http_400: bad request", 0, policy, false).is_none());
        assert!(model_retry_decision("provider_http_503", 5, policy, false).is_none());
        assert!(model_retry_decision("provider_http_503", 0, policy, true).is_none());

        let debug = format!("{decision:?}");
        for forbidden in ["重试", "网络错误", "\x1b"] {
            assert!(
                !debug.contains(forbidden),
                "core retry decision leaked UI text {forbidden:?}: {debug}"
            );
        }
    }
}

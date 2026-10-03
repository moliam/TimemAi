use std::time::Duration;

pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRIES: u32 = 100;

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
    /// The formal request succeeded only after omitting the optional provider
    /// parallel-tool control field. The caller must keep native tools enabled
    /// but schedule returned sibling calls sequentially.
    pub parallel_tool_control_omitted: bool,
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
    if is_legacy_response_header_transport_error(&lower)
        || lower.starts_with("model_network_error")
        || lower.starts_with("model_dns_error")
        || lower.starts_with("model_connect_error")
        || lower.starts_with("model_proxy_error")
        || lower.starts_with("model_body_error")
        || lower.starts_with("model_timeout")
        || lower.starts_with("curl_failed")
        || lower.contains("curl:")
        || lower.contains("http2 framing")
        || lower.contains("operation timed out")
        || lower.contains("connection reset")
        || lower.contains("could not resolve host")
    {
        return true;
    }
    if let Some(details) = lower.strip_prefix("model_responses_stream_failed:") {
        return [
            "server_is_overloaded",
            "server_error",
            "internal_error",
            "service_unavailable",
            "temporarily_unavailable",
            "rate_limit_exceeded",
            "rate_limited",
            "timeout",
        ]
        .iter()
        .any(|signal| details.contains(signal));
    }
    if let Some(status_text) = lower.strip_prefix("model_http_") {
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

fn is_legacy_response_header_transport_error(lower_error: &str) -> bool {
    // Older transport versions exposed send-time reqwest failures under the
    // generic request prefix. The stage is the stable semantic signal: request
    // construction has already completed before response headers are awaited.
    lower_error.starts_with("model_request_error:")
        && lower_error.contains("stage=response_headers")
}

/// Returns true only when a non-transient 4xx response explicitly rejects
/// native tool-request fields. This is intentionally narrower than generic
/// request validation so authentication, routing, quota, cancellation, and
/// transport failures never invalidate a capability result.
pub fn is_explicit_native_tools_unsupported(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let Some(status_text) = lower.strip_prefix("model_http_") else {
        return false;
    };
    let status: u16 = status_text
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    if !(400..500).contains(&status) || matches!(status, 401 | 403 | 408 | 409 | 413 | 425 | 429) {
        return false;
    }
    let names_native_field = [
        "parallel_tool_calls",
        "tool_choice",
        "tool_calls",
        "tools",
        "function_call",
        "functions",
    ]
    .iter()
    .any(|field| lower.contains(field));
    let explicitly_rejects_field = [
        "unsupported",
        "not supported",
        "does not support",
        "unknown field",
        "unknown parameter",
        "unrecognized",
        "unexpected field",
        "unexpected parameter",
        "not allowed",
        "not permitted",
        "extra inputs are not permitted",
    ]
    .iter()
    .any(|signal| lower.contains(signal));
    names_native_field && explicitly_rejects_field
}

/// Returns true only when a non-transient 4xx response explicitly rejects the
/// provider's parallel-tool control field. This is narrower than native-tool
/// rejection: callers may retry with only that optional control omitted while
/// retaining native tools.
pub fn is_explicit_parallel_tool_control_unsupported(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let Some(status_text) = lower.strip_prefix("model_http_") else {
        return false;
    };
    let status: u16 = status_text
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    if !(400..500).contains(&status) || matches!(status, 401 | 403 | 408 | 409 | 413 | 425 | 429) {
        return false;
    }
    let names_parallel_field =
        lower.contains("parallel_tool_calls") || lower.contains("disable_parallel_tool_use");
    let explicitly_rejects_field = [
        "unsupported",
        "not supported",
        "does not support",
        "unknown field",
        "unknown parameter",
        "unrecognized",
        "unexpected field",
        "unexpected parameter",
        "not allowed",
        "not permitted",
        "extra inputs are not permitted",
    ]
    .iter()
    .any(|signal| lower.contains(signal));
    names_parallel_field && explicitly_rejects_field
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
        || lower.starts_with("model_http_413")
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

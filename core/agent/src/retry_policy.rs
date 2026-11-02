use std::time::Duration;

pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRIES: u32 = 100;

#[cfg(not(test))]
pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY: Duration = Duration::from_secs(10);
#[cfg(test)]
pub const DEFAULT_MODEL_SYSTEM_ERROR_RETRY_DELAY: Duration = Duration::ZERO;

/// Consecutive rate-limit failures on the active endpoint that trigger a
/// switch to the next chain entry. Rate-limit windows recover on hour scales,
/// so extended in-place backoff only burns wall-clock time.
pub const MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_FAILURES: u32 = 3;

/// Consecutive retryable system failures on the active endpoint that trigger
/// a switch when an ordered failover chain exists. A chain with a single
/// endpoint keeps the full in-place backoff policy instead.
pub const MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_FAILURES: u32 = 10;

/// In-place wait between consecutive strikes while an ordered failover chain
/// exists. Rate-limit strikes wait nothing (quota windows need hours); other
/// retryable system errors use a short probe cadence instead of the full
/// single-endpoint backoff.
pub const MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_STRIKE_DELAY: Duration = Duration::ZERO;
#[cfg(not(test))]
pub const MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_STRIKE_DELAY: Duration = Duration::from_secs(1);
#[cfg(test)]
pub const MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_STRIKE_DELAY: Duration = Duration::ZERO;

/// The wait applied before the next in-place strike when a failover chain is
/// available. See the two constants above.
pub fn model_failover_strike_delay(error: &str) -> Duration {
    if is_model_rate_limited_error(error) {
        MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_STRIKE_DELAY
    } else {
        MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_STRIKE_DELAY
    }
}

/// The consecutive-failure count after which the active endpoint hands control
/// to the ordered failover chain. Rate-limit errors get the tighter threshold;
/// every other retryable error uses the system-error threshold.
pub fn model_failover_failure_threshold(error: &str) -> u32 {
    if is_model_rate_limited_error(error) {
        MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_FAILURES
    } else {
        MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_FAILURES
    }
}

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
        // A response past the transport's 16 MB guard is almost always an
        // abnormal sampling run (a generation loop or a malformed stream from
        // the provider), so a fresh sample usually arrives within bounds. The
        // byte guard itself stays: the retry loop bounds the attempts, and a
        // request that deterministically overflows still terminates.
        || lower.starts_with("model_response_too_large")
        // Runtime TLS failures (handshake, certificate verification through a
        // filtering proxy) are network-path problems and recover when the
        // path heals. The `stage=` marker separates them from the local
        // `model_tls_error: invalid private CA PEM` configuration errors,
        // which are deterministic and must not burn retry rounds.
        || lower.starts_with("model_tls_error: stage=")
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
            "overloaded_error",
            "api_error",
        ]
        .iter()
        .any(|signal| details.contains(signal));
    }
    // A stream failure without extractable details, or a stream that ended
    // without its terminal message, means the provider broke the stream:
    // server-side, transient, and exactly what a fresh attempt is for.
    if lower == "model_responses_stream_failed"
        || lower == "model_responses_stream_missing_terminal"
    {
        return true;
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

/// Deterministic "this endpoint is currently unusable" failures: a missing
/// credential, a rejected key, or an exhausted prepaid balance cannot recover
/// by waiting, so the ordered failover chain must hand over immediately. Kept
/// separate from `is_retryable_model_system_error`, which also feeds
/// same-endpoint retry decisions where burning attempts on a missing key
/// would be pointless.
pub fn is_endpoint_unavailable_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.starts_with("missing_api_key")
        || lower.starts_with("model_http_401")
        || lower.starts_with("model_http_402")
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
    lower.starts_with("model_request_too_large")
        || lower.contains("argument list too long")
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

/// True when the model error is an explicit provider rate/quota rejection
/// (HTTP 429 or a Responses-protocol rate-limit terminal event). This is the
/// trigger for endpoint-chain failover: the endpoint's quota is exhausted and
/// in-place backoff retries would only burn wall-clock time.
pub fn is_model_rate_limited_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    if lower.starts_with("model_http_429:") {
        return true;
    }
    if let Some(details) = lower.strip_prefix("model_responses_stream_failed:") {
        return ["rate_limit_exceeded", "rate_limited", "quota"]
            .iter()
            .any(|signal| details.contains(signal));
    }
    false
}

#[cfg(test)]
#[path = "../tests/unit/retry_policy_tests.rs"]
mod tests;

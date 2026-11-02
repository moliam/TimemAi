use super::*;

#[test]
fn retry_policy_defaults_match_user_visible_contract() {
    let policy = ModelSystemRetryPolicy::default();
    assert_eq!(policy.max_attempts, DEFAULT_MODEL_SYSTEM_ERROR_RETRIES);
    assert_eq!(policy.delay, Duration::ZERO);
}

#[test]
fn retryable_model_system_errors_cover_network_and_transient_http() {
    for error in [
        "model_network_error: curl: (16) Error in the HTTP2 framing layer",
        "model_dns_error: stage=response_headers lookup failed",
        "model_connect_error: stage=response_headers connection refused",
        "model_proxy_error: stage=response_headers proxy unavailable",
        "model_body_error: stage=response_body connection reset",
        "model_timeout: request exceeded timeout",
        "curl_failed",
        "curl: (28) operation timed out",
        "connection reset by peer",
        "model_network_error: stage=response_headers connection closed before message completed",
        "model_network_error: stage=response_headers incomplete message",
        "model_request_error: stage=response_headers error sending request for url (https://example.invalid/v1/chat/completions): connection error: unexpected end of file",
        "model_request_error: stage=response_headers dependency changed its transport wording",
        "model_network_error: stage=response_headers error sending request: Can't assign requested address (os error 49)",
        "could not resolve host: example.invalid",
        "model_http_408: timeout",
        "model_http_409: conflict",
        "model_http_425: too early",
        "model_http_429: rate limit",
        "model_http_500: upstream overloaded",
        "model_http_503",
        "model_responses_stream_failed: event_type=error code=server_is_overloaded type=error message=Our servers are currently overloaded. Please try again later.",
        "model_responses_stream_failed: event_type=response.failed code=rate_limit_exceeded type=server_error message=retry later",
        "model_response_too_large: response exceeds 16777216 bytes",
        "model_tls_error: stage=response_headers handshake failed",
        "model_responses_stream_missing_terminal",
        "model_responses_stream_failed: event_type=error code=overloaded_error type=error message=Overloaded",
        "model_responses_stream_failed: event_type=error code=api_error type=error message=Internal server error",
    ] {
        assert!(is_retryable_model_system_error(error), "{error}");
    }
}

#[test]
fn non_retryable_model_errors_do_not_waste_rounds() {
    for error in [
        "cancelled_by_user",
        "model_http_400: invalid model",
        "model_http_401: unauthorized",
        "model_http_403: forbidden",
        "model_http_404: model not found",
        "model_tls_error: invalid peer certificate",
        "model_request_url_error: relative URL without a base",
        "model_request_error: invalid request configuration",
        "model_request_error: stage=request_build invalid request configuration",
        "model_internal_error: runtime initialization failed",
        "model_internal_error: client initialization failed",
        "model_internal_error: reentrant model HTTP call",
        "model_request_too_large: request body exceeds limit",
        "model_redirect_blocked: cross-origin redirect",
        "model_responses_stream_failed: event_type=response.failed code=invalid_request_error type=invalid_request_error message=bad tools",
        // "model_responses_stream_failed" (bare) moved to the retryable set:
        // a provider stream failure without extractable details is server-side
        // and transient, exactly like connection resets.
        "invalid_json",
        "status_required",
        "next_actions[0].args_required",
    ] {
        assert!(!is_retryable_model_system_error(error), "{error}");
    }
}

#[test]
fn explicit_native_tool_rejection_requires_status_field_and_rejection_signal() {
    for error in [
        "model_http_400: unknown parameter: tools",
        "model_http_400: tool_choice is not supported for this model",
        "model_http_404: functions are not allowed by this endpoint",
        "model_http_422: extra inputs are not permitted: parallel_tool_calls",
    ] {
        assert!(is_explicit_native_tools_unsupported(error), "{error}");
    }
    for error in [
        "cancelled_by_user",
        "model_network_error: tools connection reset",
        "model_http_500: tools unsupported upstream",
        "model_http_401: tools are not permitted without authentication",
        "model_http_403: tools are not allowed for this account",
        "model_http_408: unknown parameter tools",
        "model_http_409: tools not supported while deployment starts",
        "model_http_413: tools request body too large",
        "model_http_425: tools unsupported before deployment is ready",
        "model_http_429: tools are not allowed over quota",
        "model_http_400: invalid model",
        "model_http_400: invalid parameter: tools",
        "model_http_400: tools must be a non-empty array",
    ] {
        assert!(!is_explicit_native_tools_unsupported(error), "{error}");
    }
}

#[test]
fn parallel_control_rejection_requires_exact_field_non_transient_4xx_and_rejection_signal() {
    for error in [
        "model_http_400: unknown parameter: parallel_tool_calls",
        "model_http_404: disable_parallel_tool_use is not supported by this endpoint",
        "model_http_422: extra inputs are not permitted: parallel_tool_calls",
    ] {
        assert!(
            is_explicit_parallel_tool_control_unsupported(error),
            "{error}"
        );
    }
    for error in [
        "cancelled_by_user",
        "model_network_error: parallel_tool_calls connection reset",
        "model_http_500: parallel_tool_calls unsupported upstream",
        "model_http_401: parallel_tool_calls not permitted without authentication",
        "model_http_403: disable_parallel_tool_use is not allowed for this account",
        "model_http_408: unknown parameter parallel_tool_calls",
        "model_http_409: parallel_tool_calls not supported while deployment starts",
        "model_http_413: parallel_tool_calls payload too large",
        "model_http_425: parallel_tool_calls unsupported before deployment is ready",
        "model_http_429: parallel_tool_calls not allowed over quota",
        "model_http_400: invalid model",
        "model_http_400: invalid parameter: parallel_tool_calls",
        "model_http_400: tools are not supported",
    ] {
        assert!(
            !is_explicit_parallel_tool_control_unsupported(error),
            "{error}"
        );
    }
}

#[test]
fn input_too_large_errors_are_detected_without_matching_unrelated_failures() {
    for error in [
        "Argument list too long (os error 7)",
        "E2BIG while spawning model transport",
        "model_http_413: payload too large",
        "model_http_400: context_length_exceeded",
        "model_http_400: maximum context length is 100000 tokens",
        "model_http_400: too many input tokens",
        "model_http_400: prompt is too long: 200001 tokens > 200000 maximum",
        "model_http_400: input token length exceeds the model limit",
        "model_request_too_large: request body exceeds 16777216 bytes",
    ] {
        assert!(is_model_input_too_large_error(error), "{error}");
    }
    for error in [
        "model_http_400: invalid model",
        "model_http_401: unauthorized",
        "model_http_500: overloaded",
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
    let decision = model_retry_decision("model_http_503: overloaded", 0, policy, false).unwrap();
    assert_eq!(
        decision,
        ModelRetryDecision {
            retry_attempt: 1,
            max_attempts: 5,
            delay: Duration::from_secs(10),
        }
    );
    assert!(model_retry_decision("model_http_400: bad request", 0, policy, false).is_none());
    assert!(model_retry_decision("model_http_503", 5, policy, false).is_none());
    assert!(model_retry_decision("model_http_503", 0, policy, true).is_none());

    let debug = format!("{decision:?}");
    for forbidden in ["重试", "网络错误", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core retry decision leaked UI text {forbidden:?}: {debug}"
        );
    }
}

#[test]
fn rate_limit_errors_trigger_endpoint_failover_classification() {
    for error in [
        "model_http_429: rate limit exceeded",
        "model_http_429: Too Many Requests",
        "model_responses_stream_failed: event_type=response.failed code=rate_limit_exceeded type=server_error message=retry later",
        "model_responses_stream_failed: event_type=error code=rate_limited message=slow down",
        "model_responses_stream_failed: event_type=response.failed code=insufficient_quota type=server_error message=quota exceeded",
    ] {
        assert!(is_model_rate_limited_error(error), "{error}");
    }
    for error in [
        "model_http_428: precondition",
        "model_http_4290: malformed",
        "model_http_500: overloaded",
        "model_http_408: timeout",
        "cancelled_by_user",
        "model_network_error: connection reset",
        "model_responses_stream_failed: event_type=response.failed code=server_is_overloaded type=server_error message=overloaded",
        "model_responses_stream_failed: event_type=response.failed code=invalid_request_error type=invalid_request_error message=bad tools",
        "model_responses_stream_failed",
    ] {
        assert!(!is_model_rate_limited_error(error), "{error}");
    }
}

#[test]
fn failover_failure_thresholds_match_tight_rate_limit_contract() {
    assert_eq!(
        model_failover_failure_threshold("model_http_429: The usage limit has been reached"),
        MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_FAILURES
    );
    assert_eq!(MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_FAILURES, 3);
    assert_eq!(
        model_failover_failure_threshold("model_http_500: overloaded"),
        MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_FAILURES
    );
    assert_eq!(
        model_failover_failure_threshold("model_network_error: connection reset"),
        MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_FAILURES
    );
    assert_eq!(MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_FAILURES, 10);
    // Rate-limit strikes wait nothing between attempts (hour-scale quota
    // windows); other retryable errors probe on the short cadence.
    assert_eq!(
        model_failover_strike_delay("model_http_429: quota"),
        Duration::ZERO
    );
    assert_eq!(
        model_failover_strike_delay("model_timeout: slow"),
        MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_STRIKE_DELAY
    );
}

#[test]
fn missing_key_rejected_credential_and_exhausted_balance_mark_endpoint_unavailable() {
    assert!(is_endpoint_unavailable_error(
        "missing_api_key: set TIMEM_API_KEY or configure the Session API key"
    ));
    assert!(is_endpoint_unavailable_error(
        "model_http_401: invalid credentials"
    ));
    // An exhausted prepaid balance (HTTP 402 Payment Required) cannot recover
    // by waiting either: the failover chain must take over immediately.
    assert!(is_endpoint_unavailable_error(
        "model_http_402: Insufficient Balance (request_id: c48827d7)"
    ));
    assert!(!is_endpoint_unavailable_error(
        "model_http_403: forbidden for this account"
    ));
    assert!(!is_endpoint_unavailable_error(
        "model_http_429: rate limit exceeded"
    ));
    assert!(!is_endpoint_unavailable_error(
        "model_network_error: connection reset"
    ));
}

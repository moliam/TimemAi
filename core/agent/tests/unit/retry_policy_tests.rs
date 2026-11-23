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

#[test]
fn invalid_native_tool_call_payloads_are_protocol_violations() {
    for error in [
        "invalid_tool_call[1].arguments_json:expected `,` or `]` at line 1 column 68",
        "invalid_tool_call[0].arguments_must_be_object",
        "invalid_tool_call[2].input_must_be_object",
        "invalid_tool_call[0].id_required",
        "invalid_tool_call[0].arguments_must_be_json",
    ] {
        assert!(is_invalid_native_tool_call_error(error), "{error}");
        // A malformed native payload must not be retried against the same
        // endpoint as a transport failure; the turn loop handles it as a
        // bounded protocol repair instead.
        assert!(!is_retryable_model_system_error(error), "{error}");
    }
    for error in [
        "invalid_json",
        "model_http_400: invalid_tool_call[1].arguments_json",
        "invalid_tool_call[]...",
        "invalid_tool_call[x].arguments_json",
        "too_many_tool_calls: received 5, limit 4",
    ] {
        assert!(!is_invalid_native_tool_call_error(error), "{error}");
    }
}

#[test]
fn reasoning_content_rejection_requires_exact_field_non_transient_4xx_and_rejection_signal() {
    for error in [
        "model_http_400: unknown parameter: reasoning_content",
        "model_http_404: reasoning_content is not supported by this endpoint",
        "model_http_422: extra inputs are not permitted: reasoning_content",
        "model_http_422: unexpected field `reasoning_content` in assistant message",
    ] {
        assert!(
            is_reasoning_content_input_rejected(error),
            "must detect: {error}"
        );
    }
    for error in [
        "model_http_400: invalid request payload",
        "model_http_400: reasoning_content has an invalid value",
        "model_http_500: reasoning_content is unsupported",
        "model_http_429: reasoning_content not allowed right now",
        "model_http_401: reasoning_content unauthorized",
        "model_network_error: reasoning_content rejected",
        "model_http_400: parallel_tool_calls unsupported",
    ] {
        assert!(
            !is_reasoning_content_input_rejected(error),
            "must not detect: {error}"
        );
    }
}

#[test]
fn provider_business_codes_drive_retry_failover_and_overflow() {
    // Zhipu 1113 (arrears) is a 429 status but not a rate limit: no retry,
    // hand over to the failover chain immediately.
    let arrears = "model_http_429[billing,zhipu/1113]: 您的账户已欠费，请充值后重试";
    assert!(!is_retryable_model_system_error(arrears));
    assert!(is_endpoint_unavailable_error(arrears));
    assert!(!is_model_rate_limited_error(arrears));

    // 1308 usage window: retryable with the tight quota-window failover.
    let window = "model_http_429[quota,zhipu/1308]: 已达到 100 次的使用上限";
    assert!(is_retryable_model_system_error(window));
    assert!(!is_endpoint_unavailable_error(window));
    assert!(is_model_rate_limited_error(window));
    assert_eq!(
        model_failover_failure_threshold(window),
        MODEL_ENDPOINT_FAILOVER_RATE_LIMIT_FAILURES
    );
    assert_eq!(model_failover_strike_delay(window), Duration::ZERO);

    // 1305 overload is transient but not a quota window.
    let overloaded = "model_http_429[overloaded,zhipu/1305]: 该模型当前访问量过大，请您稍后再试";
    assert!(is_retryable_model_system_error(overloaded));
    assert!(!is_model_rate_limited_error(overloaded));
    assert_eq!(
        model_failover_failure_threshold(overloaded),
        MODEL_ENDPOINT_FAILOVER_SYSTEM_ERROR_FAILURES
    );

    // 1261 prompt too long routes to the context-overflow recovery, not retries.
    let too_long = "model_http_400[input_too_large,zhipu/1261]: Prompt 超长";
    assert!(is_model_input_too_large_error(too_long));
    assert!(!is_retryable_model_system_error(too_long));

    // 1211 missing model id is a configuration error: fail closed, do not
    // silently rotate the failover chain.
    let missing = "model_http_400[model_not_found,zhipu/1211]: 模型不存在，请检查模型代码";
    assert!(!is_retryable_model_system_error(missing));
    assert!(!is_endpoint_unavailable_error(missing));

    // 1301 content policy is terminal for the turn.
    let policy =
        "model_http_400[content_policy,zhipu/1301]: 系统检测到输入或生成内容可能包含不安全或敏感内容";
    assert!(!is_retryable_model_system_error(policy));

    // 1000 and 1315 are credential problems: unusable endpoint, no retry.
    for error in [
        "model_http_401[authentication,zhipu/1000]: 身份验证失败",
        "model_http_429[authentication,zhipu/1315]: 该 API Key 仅限企业编程套餐场景使用",
    ] {
        assert!(!is_retryable_model_system_error(error), "{error}");
        assert!(is_endpoint_unavailable_error(error), "{error}");
    }

    // 1309/1314 expired plans cannot recover by waiting either.
    for error in [
        "model_http_429[subscription,zhipu/1309]: 您的 GLM Coding Plan 套餐已到期",
        "model_http_429[subscription,zhipu/1314]: 您的企业套餐已失效，请联系企业管理员",
    ] {
        assert!(is_endpoint_unavailable_error(error), "{error}");
        assert!(!is_retryable_model_system_error(error), "{error}");
    }

    // 1220 permission stays a configuration error: surfaced, never failed over.
    let permission = "model_http_403[permission,zhipu/1220]: 您无权访问该 API";
    assert!(!is_endpoint_unavailable_error(permission));
    assert!(!is_retryable_model_system_error(permission));

    // An undocumented provider keeps the provider-neutral status class.
    let foreign = "model_http_429[rate_limited,acme/1113]: slow down";
    assert!(is_retryable_model_system_error(foreign));
    assert!(is_model_rate_limited_error(foreign));
}

#[test]
fn zhipu_body_code_is_classified_end_to_end() {
    let body =
        serde_json::json!({"error":{"code":"1113","message":"您的账户已欠费，请充值后重试"}});
    let configured = crate::model_api::model_http_error_message_for(429, Some("zhipu"), &body);
    assert_eq!(
        configured,
        "model_http_429[billing,zhipu/1113]: 您的账户已欠费，请充值后重试"
    );
    assert!(is_endpoint_unavailable_error(&configured));
    assert!(!is_retryable_model_system_error(&configured));

    // Without a declared provider the code is kept for diagnostics but the
    // HTTP status decides the class.
    let unconfigured = crate::model_api::model_http_error_message_for(429, None, &body);
    assert_eq!(
        unconfigured,
        "model_http_429[rate_limited,1113]: 您的账户已欠费，请充值后重试"
    );
    assert!(is_retryable_model_system_error(&unconfigured));
}

#[test]
fn kimi_plan_rejection_uses_the_vendor_table() {
    // Kimi documents 403 for plan/subscription denial ("Your current subscription
    // does not have access to ..."). With the vendor declared, that endpoint can
    // be skipped for another one; the same status without a vendor stays a
    // configuration error and is neither retried nor failed over.
    let body = serde_json::json!({
        "error": {"message": "Your current subscription does not have access to k3"}
    });
    // No business code in the body, so the header carries the class only.
    let kimi = crate::model_api::model_http_error_message_for(403, Some("kimi"), &body);
    assert_eq!(
        kimi,
        "model_http_403[subscription]: Your current subscription does not have access to k3"
    );
    assert!(is_endpoint_unavailable_error(&kimi));
    assert!(!is_retryable_model_system_error(&kimi));

    let coding_plan = crate::model_api::model_http_error_message_for(403, Some("kimi_code"), &body);
    assert!(coding_plan.starts_with("model_http_403[subscription]"));
    assert!(is_endpoint_unavailable_error(&coding_plan));

    for provider in [Some("zhipu"), Some("openai"), Some("deepseek"), None] {
        let generic = crate::model_api::model_http_error_message_for(403, provider, &body);
        assert!(
            generic.starts_with("model_http_403[permission"),
            "{generic}"
        );
        assert!(!is_endpoint_unavailable_error(&generic));
    }
}

use super::*;

fn class_of(provider: &str, code: &str) -> ModelErrorClass {
    classify_provider_code(Some(provider), code).unwrap_or_else(|| panic!("{provider}/{code}"))
}

#[test]
fn zhipu_documented_business_codes_map_to_their_classes() {
    // Table from the official Zhipu error-code reference. Keep this list and
    // ZHIPU_CODES in sync: a new documented code belongs in both places.
    let expected = [
        (1000, ModelErrorClass::Authentication),
        (1001, ModelErrorClass::Authentication),
        (1003, ModelErrorClass::Authentication),
        (1005, ModelErrorClass::Authentication),
        (1113, ModelErrorClass::Billing),
        (1200, ModelErrorClass::ServerError),
        (1210, ModelErrorClass::InvalidRequest),
        (1211, ModelErrorClass::ModelNotFound),
        (1212, ModelErrorClass::InvalidRequest),
        (1213, ModelErrorClass::InvalidRequest),
        (1214, ModelErrorClass::InvalidRequest),
        (1215, ModelErrorClass::InvalidRequest),
        (1220, ModelErrorClass::Permission),
        (1221, ModelErrorClass::InvalidRequest),
        (1222, ModelErrorClass::InvalidRequest),
        (1230, ModelErrorClass::ServerError),
        (1234, ModelErrorClass::Transport),
        (1261, ModelErrorClass::InputTooLarge),
        (1301, ModelErrorClass::ContentPolicy),
        (1302, ModelErrorClass::RateLimited),
        (1305, ModelErrorClass::Overloaded),
        (1308, ModelErrorClass::Quota),
        (1309, ModelErrorClass::Subscription),
        (1310, ModelErrorClass::Quota),
        (1311, ModelErrorClass::Subscription),
        (1313, ModelErrorClass::RateLimited),
        (1314, ModelErrorClass::Subscription),
        (1315, ModelErrorClass::Authentication),
        (1316, ModelErrorClass::Quota),
        (1317, ModelErrorClass::Quota),
        (1318, ModelErrorClass::Quota),
        (1319, ModelErrorClass::Quota),
        (1320, ModelErrorClass::Quota),
        (1321, ModelErrorClass::Quota),
    ];
    assert_eq!(expected.len(), ZHIPU_CODES.len());
    for (code, class) in expected {
        assert_eq!(class_of("zhipu", &code.to_string()), class, "{code}");
    }
    // Unknown or undocumented codes fall back to the HTTP status.
    assert_eq!(classify_provider_code(Some("zhipu"), "9999"), None);
    assert_eq!(classify_provider_code(Some("zhipu"), "rate_limit"), None);
}

#[test]
fn provider_tables_do_not_leak_across_providers() {
    // 1113 is a Zhipu business code; the same digits mean nothing for OpenAI.
    assert_eq!(classify_provider_code(Some("openai"), "1113"), None);
    assert_eq!(classify_provider_code(None, "1113"), None);
    assert_eq!(
        classify_http(429, Some("openai"), Some("1113")),
        ModelErrorClass::RateLimited
    );
    assert_eq!(
        classify_http(429, Some("zhipu"), Some("1113")),
        ModelErrorClass::Billing
    );
}

#[test]
fn openai_symbolic_codes_and_stream_codes_map_to_classes() {
    for (code, class) in [
        ("invalid_request_error", ModelErrorClass::InvalidRequest),
        ("invalid_api_key", ModelErrorClass::Authentication),
        ("authentication_error", ModelErrorClass::Authentication),
        ("permission_error", ModelErrorClass::Permission),
        ("model_not_found", ModelErrorClass::ModelNotFound),
        ("insufficient_quota", ModelErrorClass::Quota),
        ("rate_limit_exceeded", ModelErrorClass::RateLimited),
        ("context_length_exceeded", ModelErrorClass::InputTooLarge),
        ("content_policy_violation", ModelErrorClass::ContentPolicy),
        ("server_error", ModelErrorClass::ServerError),
        ("engine_overloaded", ModelErrorClass::Overloaded),
        ("timeout", ModelErrorClass::Timeout),
    ] {
        assert_eq!(class_of("openai", code), class, "{code}");
        assert_eq!(classify_stream_code(Some(code)), Some(class), "{code}");
    }
    assert_eq!(classify_stream_code(Some("something_new")), None);
    assert_eq!(classify_stream_code(None), None);
}

#[test]
fn status_only_providers_use_the_provider_neutral_table() {
    // DeepSeek documents statuses only (400/401/402/408/413/422/429/500/503/504).
    // Vendors without a status override keep the provider-neutral meaning.
    for (status, class) in [
        (400, ModelErrorClass::InvalidRequest),
        (401, ModelErrorClass::Authentication),
        (402, ModelErrorClass::Billing),
        (403, ModelErrorClass::Permission),
        (404, ModelErrorClass::ModelNotFound),
        (408, ModelErrorClass::Timeout),
        (409, ModelErrorClass::ServerError),
        (413, ModelErrorClass::InputTooLarge),
        (422, ModelErrorClass::InvalidRequest),
        (429, ModelErrorClass::RateLimited),
        (500, ModelErrorClass::ServerError),
        (503, ModelErrorClass::Overloaded),
        (504, ModelErrorClass::Timeout),
        (418, ModelErrorClass::InvalidRequest),
        (599, ModelErrorClass::ServerError),
        (0, ModelErrorClass::Unknown),
        (4290, ModelErrorClass::Unknown),
    ] {
        for provider in [Some("deepseek"), Some("zhipu"), Some("openai"), None] {
            assert_eq!(classify_http(status, provider, None), class, "{status}");
        }
    }
}

#[test]
fn vendor_status_table_follows_the_endpoint_vendor_selection() {
    // Kimi's model-call 403 entries are all plan/subscription statements, so the
    // Kimi rows carry a vendor-specific class; every other vendor keeps the
    // generic meaning of 403 (a scoped key or a model permission mistake).
    for provider in ["kimi", "kimi_code"] {
        assert_eq!(
            classify_http(403, Some(provider), None),
            ModelErrorClass::Subscription,
            "{provider}"
        );
        // Only the documented row is overridden; neighbouring statuses stay generic.
        assert_eq!(
            classify_http(404, Some(provider), None),
            ModelErrorClass::ModelNotFound
        );
        assert_eq!(
            classify_http(429, Some(provider), None),
            ModelErrorClass::RateLimited
        );
        assert_eq!(classify_vendor_status(Some(provider), 500), None);
    }
    for provider in ["zhipu", "openai", "deepseek", "acme", ""] {
        assert_eq!(
            classify_http(403, Some(provider), None),
            ModelErrorClass::Permission,
            "{provider}"
        );
    }
    assert_eq!(classify_http(403, None, None), ModelErrorClass::Permission);
    assert_eq!(classify_vendor_status(None, 403), None);
    assert_eq!(
        classify_vendor_status(Some("KIMI"), 403),
        Some(ModelErrorClass::Subscription)
    );

    // A documented business code still wins over the vendor status row.
    assert_eq!(
        classify_http(429, Some("zhipu"), Some("1113")),
        ModelErrorClass::Billing
    );
}

#[test]
fn canonical_header_round_trips_class_provider_and_code() {
    let rendered = format_http_error(
        429,
        ModelErrorClass::Billing,
        Some("zhipu"),
        Some("1113"),
        Some("您的账户已欠费，请充值后重试"),
    );
    assert_eq!(
        rendered,
        "model_http_429[billing,zhipu/1113]: 您的账户已欠费，请充值后重试"
    );
    let facts = parse_model_error(&rendered);
    assert_eq!(facts.status, Some(429));
    assert_eq!(facts.class, ModelErrorClass::Billing);
    assert_eq!(facts.provider.as_deref(), Some("zhipu"));
    assert_eq!(facts.code.as_deref(), Some("1113"));

    // No provider code keeps the class readable on its own.
    let plain = format_http_error(500, ModelErrorClass::ServerError, None, None, None);
    assert_eq!(plain, "model_http_500[server_error]");
    let facts = parse_model_error(&plain);
    assert_eq!(facts.class, ModelErrorClass::ServerError);
    assert_eq!(facts.code, None);

    // An unrecognized provider still records the code for diagnostics.
    let unknown = format_http_error(
        429,
        ModelErrorClass::RateLimited,
        Some("private-gateway"),
        Some("E429"),
        Some("slow down"),
    );
    assert_eq!(
        unknown,
        "model_http_429[rate_limited,private-gateway/e429]: slow down"
    );
    let facts = parse_model_error(&unknown);
    assert_eq!(facts.provider.as_deref(), Some("private-gateway"));
    assert_eq!(facts.code.as_deref(), Some("e429"));
}

#[test]
fn legacy_http_strings_classify_by_status_without_reading_the_message() {
    // Records written before the canonical header, and hand-built fixtures,
    // carry no class token: they must still classify by status only.
    for (error, class) in [
        (
            "model_http_400: invalid model",
            ModelErrorClass::InvalidRequest,
        ),
        (
            "model_http_401: invalid credentials",
            ModelErrorClass::Authentication,
        ),
        (
            "model_http_402: Insufficient Balance",
            ModelErrorClass::Billing,
        ),
        ("model_http_403: forbidden", ModelErrorClass::Permission),
        (
            "model_http_404: model not found",
            ModelErrorClass::ModelNotFound,
        ),
        (
            "model_http_413: payload too large",
            ModelErrorClass::InputTooLarge,
        ),
        ("model_http_429: rate limit", ModelErrorClass::RateLimited),
        (
            "model_http_500: upstream overloaded",
            ModelErrorClass::ServerError,
        ),
        ("model_http_503", ModelErrorClass::Overloaded),
        ("model_http_408: timeout", ModelErrorClass::Timeout),
    ] {
        let facts = parse_model_error(error);
        assert_eq!(facts.class, class, "{error}");
    }
    // A message that merely mentions another class does not move the class:
    // "overloaded" text under 429 stays a rate limit, exactly as the status says.
    assert_eq!(
        parse_model_error("model_http_429: upstream overloaded").class,
        ModelErrorClass::RateLimited
    );
    assert_eq!(
        parse_model_error("model_network_error: connection reset").class,
        ModelErrorClass::Transport
    );
    assert_eq!(
        parse_model_error("model_timeout: request exceeded timeout").class,
        ModelErrorClass::Timeout
    );
    assert_eq!(
        parse_model_error("cancelled_by_user").class,
        ModelErrorClass::Unknown
    );
}

#[test]
fn stream_failure_strings_carry_or_derive_their_class() {
    let canonical = parse_model_error(
        "model_responses_stream_failed: class=overloaded event_type=error code=server_is_overloaded message=Our servers are currently overloaded.",
    );
    assert_eq!(canonical.class, ModelErrorClass::Overloaded);

    // Legacy stream strings (no class token) derive the class from the code.
    let legacy = parse_model_error(
        "model_responses_stream_failed: event_type=response.failed code=invalid_request_error type=invalid_request_error message=bad tools",
    );
    assert_eq!(legacy.class, ModelErrorClass::InvalidRequest);
    assert_eq!(legacy.code.as_deref(), Some("invalid_request_error"));

    let bare = parse_model_error("model_responses_stream_failed");
    assert_eq!(bare.class, ModelErrorClass::Unknown);
    let missing_terminal = parse_model_error("model_responses_stream_missing_terminal");
    assert_eq!(missing_terminal.class, ModelErrorClass::Unknown);
}

#[test]
fn class_semantics_drive_retry_failover_and_overflow() {
    use ModelErrorClass::*;
    for class in [
        RateLimited,
        Quota,
        Overloaded,
        Timeout,
        ServerError,
        Transport,
    ] {
        assert!(class.retryable(), "{class:?}");
        assert!(!class.endpoint_unavailable(), "{class:?}");
    }
    for class in [Authentication, Billing, Subscription] {
        assert!(class.endpoint_unavailable(), "{class:?}");
        assert!(!class.retryable(), "{class:?}");
    }
    // Permission is a configuration error: neither retried nor failed over.
    assert!(!Permission.retryable() && !Permission.endpoint_unavailable());
    assert!(RateLimited.quota_window() && Quota.quota_window());
    assert!(!Overloaded.quota_window() && !Billing.quota_window());
    for class in [InvalidRequest, ModelNotFound, InputTooLarge, ContentPolicy] {
        assert!(class.terminal(), "{class:?}");
        assert!(!class.retryable(), "{class:?}");
    }
    assert!(InputTooLarge.terminal());
    assert!(!Unknown.retryable() && !Unknown.terminal());
}

#[test]
fn business_code_interpretation_requires_a_declared_provider() {
    // The canonical string records the provider and code so an operator can see
    // why a class was chosen; without a documented table the status decides.
    let facts = parse_model_error(&format_http_error(
        429,
        classify_http(429, Some("zhipu"), Some("1113")),
        Some("zhipu"),
        Some("1113"),
        Some("您的账户已欠费，请充值后重试"),
    ));
    assert_eq!(facts.class, ModelErrorClass::Billing);

    let unknown_provider = parse_model_error(&format_http_error(
        429,
        classify_http(429, Some("acme"), Some("1113")),
        Some("acme"),
        Some("1113"),
        None,
    ));
    assert_eq!(unknown_provider.class, ModelErrorClass::RateLimited);
    assert_eq!(unknown_provider.provider.as_deref(), Some("acme"));
}

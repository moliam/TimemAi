//! Structured classification of provider model errors.
//!
//! The wire failure (HTTP status, provider business code, provider message) is
//! parsed exactly once, where the response is interpreted, and encoded into a
//! canonical header. Every consumer (same-endpoint retry, endpoint failover,
//! context-overflow recovery, Web guidance) then reads that header instead of
//! matching each provider's natural-language wording:
//!
//! ```text
//! model_http_429[billing,zhipu/1113]: 您的账户已欠费，请充值后重试
//! model_http_400[invalid_request]: invalid model
//! model_http_401[authentication]: invalid credentials
//! model_responses_stream_failed: class=overloaded event_type=error code=server_is_overloaded message=...
//! ```
//!
//! Grammar: `model_http_<status>[<class>]`, optionally
//! `[<class>,<provider>/<code>]` when the provider business code is known.
//! A header without a class token (older records, hand-built fixtures) is read
//! as the provider-neutral HTTP-status class, so classification never depends
//! on message text.
//!
//! Extending: add one documented table row per provider code. Providers that
//! only publish HTTP statuses (DeepSeek, Kimi) need no table: the status class
//! covers them. Never key a decision off provider message substrings here.

/// Semantic class of a model API failure, derived from the provider's
/// documented error code (or the HTTP status when the provider publishes no
/// business codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelErrorClass {
    /// Credential missing, rejected or expired.
    Authentication,
    /// Credential valid but the account may not use this API/model.
    Permission,
    /// Prepaid balance exhausted; the account must be topped up.
    Billing,
    /// Coding/enterprise plan expired, disabled or missing the model.
    Subscription,
    /// Usage window (5h/7d/monthly) or hard quota reached.
    Quota,
    /// Request rate or concurrency limit reached.
    RateLimited,
    /// Upstream model/service temporarily overloaded.
    Overloaded,
    /// Request or gateway timeout.
    Timeout,
    /// Provider 5xx / internal failure.
    ServerError,
    /// Network, DNS, proxy or body-transfer failure.
    Transport,
    /// Malformed request, unsupported parameter or retired API.
    InvalidRequest,
    /// The configured model id does not exist.
    ModelNotFound,
    /// Prompt/context exceeds the model window or the request byte limit.
    InputTooLarge,
    /// Provider content-safety rejection.
    ContentPolicy,
    #[default]
    Unknown,
}

impl ModelErrorClass {
    /// Stable token written into the canonical header and read back by
    /// consumers (Core retry policy, Web guidance).
    pub const fn token(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Permission => "permission",
            Self::Billing => "billing",
            Self::Subscription => "subscription",
            Self::Quota => "quota",
            Self::RateLimited => "rate_limited",
            Self::Overloaded => "overloaded",
            Self::Timeout => "timeout",
            Self::ServerError => "server_error",
            Self::Transport => "transport",
            Self::InvalidRequest => "invalid_request",
            Self::ModelNotFound => "model_not_found",
            Self::InputTooLarge => "input_too_large",
            Self::ContentPolicy => "content_policy",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_token(value: &str) -> Option<Self> {
        const CLASSES: &[ModelErrorClass] = &[
            ModelErrorClass::Authentication,
            ModelErrorClass::Permission,
            ModelErrorClass::Billing,
            ModelErrorClass::Subscription,
            ModelErrorClass::Quota,
            ModelErrorClass::RateLimited,
            ModelErrorClass::Overloaded,
            ModelErrorClass::Timeout,
            ModelErrorClass::ServerError,
            ModelErrorClass::Transport,
            ModelErrorClass::InvalidRequest,
            ModelErrorClass::ModelNotFound,
            ModelErrorClass::InputTooLarge,
            ModelErrorClass::ContentPolicy,
            ModelErrorClass::Unknown,
        ];
        CLASSES
            .iter()
            .copied()
            .find(|class| class.token().eq_ignore_ascii_case(value))
    }

    /// A fresh attempt on the same endpoint can plausibly succeed.
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::Quota
                | Self::Overloaded
                | Self::Timeout
                | Self::ServerError
                | Self::Transport
        )
    }

    /// Deterministic "this endpoint cannot serve" failure: retrying or waiting
    /// is pointless, so the ordered failover chain must take over at once.
    /// Permission is deliberately excluded: a scoped-key or model-permission
    /// mistake is a configuration error, not an endpoint outage.
    pub const fn endpoint_unavailable(self) -> bool {
        matches!(
            self,
            Self::Authentication | Self::Billing | Self::Subscription
        )
    }

    /// Quota-window failures: recover on hour scales, so the failover chain
    /// switches after the tight threshold without in-place waiting.
    pub const fn quota_window(self) -> bool {
        matches!(self, Self::RateLimited | Self::Quota)
    }

    /// Terminal for the turn: the request itself must change (prompt, model,
    /// credentials or content), so retrying the same body cannot help.
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Authentication
                | Self::Permission
                | Self::Billing
                | Self::Subscription
                | Self::InvalidRequest
                | Self::ModelNotFound
                | Self::InputTooLarge
                | Self::ContentPolicy
        )
    }
}

/// Facts decoded from a canonical model error string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelErrorFacts {
    pub status: Option<u16>,
    pub class: ModelErrorClass,
    pub provider: Option<String>,
    pub code: Option<String>,
}

impl ModelErrorFacts {
    fn new(status: Option<u16>, class: ModelErrorClass) -> Self {
        Self {
            status,
            class,
            provider: None,
            code: None,
        }
    }
}

/// Zhipu business codes, from the official "错误码" table
/// (`docs.bigmodel.cn`). Numeric codes are provider-specific: they are only
/// read for the `zhipu` provider.
const ZHIPU_CODES: &[(u32, ModelErrorClass)] = &[
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

/// Symbolic codes returned by the OpenAI family (HTTP error bodies and
/// Responses stream failure events).
const OPENAI_CODES: &[(&str, ModelErrorClass)] = &[
    ("invalid_request_error", ModelErrorClass::InvalidRequest),
    ("invalid_api_key", ModelErrorClass::Authentication),
    ("authentication_error", ModelErrorClass::Authentication),
    ("permission_error", ModelErrorClass::Permission),
    ("model_not_found", ModelErrorClass::ModelNotFound),
    ("not_found_error", ModelErrorClass::ModelNotFound),
    ("insufficient_quota", ModelErrorClass::Quota),
    ("rate_limit_exceeded", ModelErrorClass::RateLimited),
    ("rate_limited", ModelErrorClass::RateLimited),
    ("context_length_exceeded", ModelErrorClass::InputTooLarge),
    ("content_policy_violation", ModelErrorClass::ContentPolicy),
    ("server_error", ModelErrorClass::ServerError),
    ("internal_error", ModelErrorClass::ServerError),
    ("api_error", ModelErrorClass::ServerError),
    ("service_unavailable", ModelErrorClass::Overloaded),
    ("temporarily_unavailable", ModelErrorClass::Overloaded),
    ("server_is_overloaded", ModelErrorClass::Overloaded),
    ("overloaded_error", ModelErrorClass::Overloaded),
    ("engine_overloaded", ModelErrorClass::Overloaded),
    ("timeout", ModelErrorClass::Timeout),
];

/// Provider-neutral class for an HTTP status. DeepSeek and Kimi publish only
/// statuses (401/402/403/404/408/413/422/429/5xx), so this table is their
/// complete mapping.
pub fn classify_status(status: u16) -> ModelErrorClass {
    match status {
        400 | 422 => ModelErrorClass::InvalidRequest,
        401 => ModelErrorClass::Authentication,
        402 => ModelErrorClass::Billing,
        403 => ModelErrorClass::Permission,
        404 => ModelErrorClass::ModelNotFound,
        408 => ModelErrorClass::Timeout,
        // Conflict / Too Early are transient by the transport contract.
        409 => ModelErrorClass::ServerError,
        413 => ModelErrorClass::InputTooLarge,
        425 => ModelErrorClass::Timeout,
        429 => ModelErrorClass::RateLimited,
        500 | 502 => ModelErrorClass::ServerError,
        503 => ModelErrorClass::Overloaded,
        504 => ModelErrorClass::Timeout,
        _ => {
            if (400..500).contains(&status) {
                ModelErrorClass::InvalidRequest
            } else if (500..600).contains(&status) {
                ModelErrorClass::ServerError
            } else {
                ModelErrorClass::Unknown
            }
        }
    }
}

/// Class of a documented provider business code. Unknown providers and
/// unknown codes return `None` so the caller falls back to the HTTP status
/// instead of guessing.
pub fn classify_provider_code(provider: Option<&str>, code: &str) -> Option<ModelErrorClass> {
    let provider = provider?.trim().to_ascii_lowercase();
    match provider.as_str() {
        "zhipu" => code
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(|code| ZHIPU_CODES.iter().find(|(id, _)| *id == code))
            .map(|(_, class)| *class),
        "openai" => OPENAI_CODES
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(code.trim()))
            .map(|(_, class)| *class),
        _ => None,
    }
}

/// Class of a Responses stream failure code (OpenAI family event payloads).
pub fn classify_stream_code(code: Option<&str>) -> Option<ModelErrorClass> {
    let code = code?.trim();
    OPENAI_CODES
        .iter()
        .find(|(id, _)| id.eq_ignore_ascii_case(code))
        .map(|(_, class)| *class)
}

/// Vendor status overrides. The table follows the endpoint's declared vendor,
/// so a status can mean something narrower for one vendor than the generic HTTP
/// meaning. Only statuses where the vendor's own error reference deviates live
/// here; everything else keeps the provider-neutral table.
///
/// Kimi's model-call 403 entries are all plan/subscription statements
/// ("Your current subscription does not have access to ...", "Your current plan
/// supports only ... up to 256K context", "unable to verify your membership
/// benefits"), so the vendor table classifies 403 as a subscription failure:
/// switching endpoints can still succeed, while a credential/permission mistake
/// stays visible in the message. Kimi 429 stays rate_limited: that one code also
/// covers raw volume and engine overload, so the generic class is the honest
/// one. DeepSeek publishes statuses only and needs no override.
const VENDOR_STATUS: &[(&str, &[(u16, ModelErrorClass)])] = &[
    ("kimi", &[(403, ModelErrorClass::Subscription)]),
    ("kimi_code", &[(403, ModelErrorClass::Subscription)]),
];

pub fn classify_vendor_status(provider: Option<&str>, status: u16) -> Option<ModelErrorClass> {
    let provider = provider?.trim().to_ascii_lowercase();
    VENDOR_STATUS
        .iter()
        .find(|(id, _)| *id == provider)
        .and_then(|(_, table)| {
            table
                .iter()
                .find(|(code, _)| *code == status)
                .map(|(_, class)| *class)
        })
}

/// Class for an HTTP failure, resolved in the order the endpoint declares:
/// the vendor business code first, then the vendor status table, then the
/// provider-neutral status table.
pub fn classify_http(status: u16, provider: Option<&str>, code: Option<&str>) -> ModelErrorClass {
    code.and_then(|code| classify_provider_code(provider, code))
        .or_else(|| classify_vendor_status(provider, status))
        .unwrap_or_else(|| classify_status(status))
}

fn sanitize_token(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    let token: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .take(32)
        .collect();
    (!token.is_empty()).then_some(token)
}

/// Build the canonical HTTP error string consumed by retry policy and UI.
pub fn format_http_error(
    status: u16,
    class: ModelErrorClass,
    provider: Option<&str>,
    code: Option<&str>,
    reason: Option<&str>,
) -> String {
    let mut header = format!("model_http_{status}[{}", class.token());
    match (sanitize_token(provider), sanitize_token(code)) {
        (Some(provider), Some(code)) => header.push_str(&format!(",{provider}/{code}")),
        (None, Some(code)) => header.push_str(&format!(",{code}")),
        _ => {}
    }
    header.push(']');
    match reason {
        Some(reason) if !reason.trim().is_empty() => format!("{header}: {reason}"),
        _ => header,
    }
}

fn parse_header(header: &str) -> (Option<ModelErrorClass>, Option<String>, Option<String>) {
    // header is the bracket content: "<class>" | "<class>,<code>" |
    // "<class>,<provider>/<code>".
    let mut parts = header.split(',');
    let class = parts
        .next()
        .and_then(ModelErrorClass::from_token)
        .unwrap_or_default();
    let mut provider = None;
    let mut code = None;
    if let Some(source) = parts.next() {
        match source.split_once('/') {
            Some((provider_id, provider_code)) => {
                provider = sanitize_token(Some(provider_id));
                code = sanitize_token(Some(provider_code));
            }
            None => code = sanitize_token(Some(source)),
        }
    }
    (Some(class), provider, code)
}

fn parse_key_value(error: &str, key: &str) -> Option<String> {
    error
        .split_whitespace()
        .find_map(|part| part.strip_prefix(key))
        .and_then(|value| sanitize_token(Some(value.trim_end_matches(','))))
}

/// Decode a canonical model error string. Strings written before the canonical
/// header existed (or by hand in fixtures) are still classified by status; the
/// provider message text is never inspected.
pub fn parse_model_error(error: &str) -> ModelErrorFacts {
    let text = error.trim();
    let lower = text.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix("model_responses_stream_failed") {
        let _ = rest;
        let class = parse_key_value(text, "class=")
            .and_then(|token| ModelErrorClass::from_token(&token))
            .or_else(|| classify_stream_code(parse_key_value(text, "code=").as_deref()))
            .or_else(|| classify_stream_code(parse_key_value(text, "type=").as_deref()));
        let mut facts = ModelErrorFacts::new(None, class.unwrap_or(ModelErrorClass::Unknown));
        facts.code = parse_key_value(text, "code=");
        return facts;
    }
    if lower == "model_responses_stream_missing_terminal" {
        return ModelErrorFacts::new(None, ModelErrorClass::Unknown);
    }
    if lower.starts_with("model_timeout") {
        return ModelErrorFacts::new(None, ModelErrorClass::Timeout);
    }
    for prefix in [
        "model_network_error",
        "model_dns_error",
        "model_connect_error",
        "model_proxy_error",
        "model_body_error",
    ] {
        if lower.starts_with(prefix) {
            return ModelErrorFacts::new(None, ModelErrorClass::Transport);
        }
    }

    let Some(rest) = text.strip_prefix("model_http_") else {
        return ModelErrorFacts::new(None, ModelErrorClass::Unknown);
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let Ok(status) = digits.parse::<u16>() else {
        return ModelErrorFacts::new(None, ModelErrorClass::Unknown);
    };
    let class = rest[digits.len()..]
        .strip_prefix('[')
        .and_then(|tail| tail.split_once(']'))
        .map(|(header, _)| parse_header(header));
    match class {
        Some((class, provider, code)) => ModelErrorFacts {
            status: Some(status),
            class: class.unwrap_or_default(),
            provider,
            code,
        },
        None => ModelErrorFacts::new(Some(status), classify_status(status)),
    }
}

#[cfg(test)]
#[path = "../tests/unit/model_error_tests.rs"]
mod tests;

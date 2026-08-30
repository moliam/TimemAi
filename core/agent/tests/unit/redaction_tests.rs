use super::*;
use serde_json::json;

#[test]
fn redacts_secret_fields_recursively() {
    let redacted = redact_value(&json!({
        "api_key": "abc",
        "nested": {"Authorization": "Bearer abc"},
        "headers": [{"x-api-key": "secret"}],
        "ok": "v"
    }));

    assert_eq!(redacted["api_key"], REDACTED);
    assert_eq!(redacted["nested"]["Authorization"], REDACTED);
    assert_eq!(redacted["headers"][0]["x-api-key"], REDACTED);
    assert_eq!(redacted["ok"], "v");
}

#[test]
fn redacts_inline_sk_tokens_in_strings() {
    let redacted = redact_value(&json!({
        "error": "bad token sk-sensitive-token, retry later",
        "ok": "ask-user"
    }));

    assert_eq!(redacted["error"], "bad token ***REDACTED***, retry later");
    assert_eq!(redacted["ok"], "ask-user");
}

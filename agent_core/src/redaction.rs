use serde_json::Value;

pub const REDACTED: &str = "***REDACTED***";

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (key, val) in map {
                if is_secret_key(key) {
                    next.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else {
                    next.insert(key.clone(), redact_value(val));
                }
            }
            Value::Object(next)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        Value::String(text) => Value::String(redact_inline_secrets(text)),
        _ => value.clone(),
    }
}

fn is_secret_key(key: &str) -> bool {
    key.to_lowercase().contains("key")
        || key.eq_ignore_ascii_case("authorization")
        || key.eq_ignore_ascii_case("x-api-key")
}

fn redact_inline_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find("sk-") {
        out.push_str(&rest[..idx]);
        let token = &rest[idx..];
        let starts_at_boundary = idx == 0
            || rest[..idx]
                .chars()
                .last()
                .map(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
                .unwrap_or(true);
        if !starts_at_boundary {
            out.push_str("sk-");
            rest = &token[3..];
            continue;
        }
        let token_len = token
            .char_indices()
            .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            .map(|(offset, ch)| offset + ch.len_utf8())
            .last()
            .unwrap_or(0);
        if token_len > 3 {
            out.push_str(REDACTED);
            rest = &token[token_len..];
        } else {
            out.push_str("sk-");
            rest = &token[3..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
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
}

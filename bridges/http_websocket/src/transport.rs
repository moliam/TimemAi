//! Axum WebSocket JSON framing for reconnectable HTTP/WebSocket clients.
//!
//! Product hosts own authentication, snapshots, and command semantics. This
//! module owns transport framing, bounded text decoding, and JSON wire I/O.

use axum::{
    extract::ws::{Message, WebSocket},
    http::{header, HeaderMap, HeaderName, HeaderValue},
    response::Response,
};
use futures_util::{stream::SplitSink, stream::SplitStream, SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt;

/// Applies cache, referrer, content-type, and active-content protections.
///
/// Authentication remains product policy and is intentionally not handled here.
pub fn apply_browser_security_headers(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:; font-src 'self' data:; form-action 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    response.headers_mut().insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
}

/// Accepts requests without `Origin`, but fails closed when a present Origin
/// does not match Host. Token/cookie authentication remains product policy.
pub fn request_origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin_authority(origin).is_some_and(|authority| authority.eq_ignore_ascii_case(host.trim()))
}

fn origin_authority(origin: &str) -> Option<&str> {
    let origin = origin.trim();
    let scheme_end = origin.find("://")?;
    let after_scheme = &origin[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    (!authority.is_empty()).then_some(authority)
}

#[derive(Debug, PartialEq, Eq)]
pub enum InboundJson<T> {
    Item(T),
    TooLarge,
    InvalidJson(String),
    Closed,
}

/// Checks the text-frame byte bound before one-pass JSON deserialization.
pub fn decode_text<T: DeserializeOwned>(text: &str, max_bytes: usize) -> InboundJson<T> {
    if text.len() > max_bytes {
        return InboundJson::TooLarge;
    }
    match serde_json::from_str(text) {
        Ok(value) => InboundJson::Item(value),
        Err(error) => InboundJson::InvalidJson(error.to_string()),
    }
}

pub struct JsonWebSocketSender {
    sender: SplitSink<WebSocket, Message>,
}

pub struct JsonWebSocketReceiver {
    receiver: SplitStream<WebSocket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonWebSocketSendError {
    Serialize(String),
    Socket(String),
}

impl fmt::Display for JsonWebSocketSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(error) => write!(formatter, "websocket_json_serialize_failed:{error}"),
            Self::Socket(error) => write!(formatter, "websocket_send_failed:{error}"),
        }
    }
}

impl std::error::Error for JsonWebSocketSendError {}

/// Splits transport halves so Hosts can concurrently select inbound commands,
/// worker results, and broadcasts without adding a mutex.
pub fn split_json_websocket(socket: WebSocket) -> (JsonWebSocketSender, JsonWebSocketReceiver) {
    let (sender, receiver) = socket.split();
    (
        JsonWebSocketSender { sender },
        JsonWebSocketReceiver { receiver },
    )
}

impl JsonWebSocketReceiver {
    /// Receives the next semantic text frame and classifies size, JSON, and close outcomes.
    pub async fn receive<T: DeserializeOwned>(&mut self, max_bytes: usize) -> InboundJson<T> {
        loop {
            match self.receiver.next().await {
                Some(Ok(Message::Text(text))) => return decode_text(text.as_str(), max_bytes),
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    return InboundJson::Closed;
                }
                Some(Ok(_)) => {}
            }
        }
    }
}

impl JsonWebSocketSender {
    /// Serializes and sends one wire value exactly once.
    pub async fn send<T: Serialize + ?Sized>(
        &mut self,
        value: &T,
    ) -> Result<(), JsonWebSocketSendError> {
        let text = serde_json::to_string(value)
            .map_err(|error| JsonWebSocketSendError::Serialize(error.to_string()))?;
        self.send_serialized_text(text).await
    }

    async fn send_serialized_text(&mut self, text: String) -> Result<(), JsonWebSocketSendError> {
        self.sender
            .send(Message::Text(text))
            .await
            .map_err(|error| JsonWebSocketSendError::Socket(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Command {
        value: String,
    }

    #[test]
    fn bounded_json_decoder_distinguishes_valid_invalid_and_oversized_frames() {
        assert_eq!(
            decode_text::<Command>(r#"{"value":"ok"}"#, 64),
            InboundJson::Item(Command {
                value: "ok".to_string(),
            })
        );
        assert!(matches!(
            decode_text::<Command>("not-json", 64),
            InboundJson::InvalidJson(_)
        ));
        assert_eq!(
            decode_text::<Command>(r#"{"value":"too large"}"#, 4),
            InboundJson::TooLarge
        );
    }
    #[test]
    fn browser_security_headers_disable_cache_referrer_and_embedding() {
        let mut response = Response::new(axum::body::Body::empty());
        apply_browser_security_headers(&mut response);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'"));
    }

    #[test]
    fn origin_gate_accepts_absent_or_same_origin_and_rejects_cross_origin() {
        assert!(request_origin_allowed(&HeaderMap::new()));

        let mut same = HeaderMap::new();
        same.insert(header::HOST, HeaderValue::from_static("127.0.0.1:13764"));
        same.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:13764"),
        );
        assert!(request_origin_allowed(&same));

        same.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://attacker.example"),
        );
        assert!(!request_origin_allowed(&same));
    }
}

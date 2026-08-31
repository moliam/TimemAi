//! Fixed HTTP/WebSocket route composition for the reconnectable Web transport.
//!
//! Product hosts inject authenticated handlers and state. This module owns the
//! public transport paths, method placement, request-body bounds, fallback, and
//! browser-wide response protection without learning product command semantics.

use axum::{
    extract::DefaultBodyLimit, middleware, response::Response, routing::MethodRouter, Router,
};

use crate::apply_browser_security_headers;

/// Product handlers for Timem's reconnectable browser transport.
///
/// Authentication, snapshots, uploads, and WebSocket command semantics remain
/// in the product host. Keeping the handlers as `MethodRouter`s avoids forcing
/// product state or handler argument types into the Bridge contract.
pub struct BrowserRouteHandlers<S> {
    pub health: MethodRouter<S>,
    pub snapshot: MethodRouter<S>,
    pub upload: MethodRouter<S>,
    pub performance_trace: MethodRouter<S>,
    pub websocket: MethodRouter<S>,
    pub static_assets: MethodRouter<S>,
}

/// Builds the fixed HTTP/WebSocket route table around product-provided handlers.
///
/// `max_request_bytes` is the global hard bound. A smaller bound remains on the
/// performance endpoint because it accepts telemetry rather than file bytes.
pub fn build_browser_router<S>(
    state: S,
    handlers: BrowserRouteHandlers<S>,
    max_request_bytes: usize,
    max_performance_trace_bytes: usize,
) -> Router
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/health", handlers.health)
        .route("/api/snapshot", handlers.snapshot)
        .route("/api/upload", handlers.upload)
        .route(
            "/api/performance-trace",
            handlers
                .performance_trace
                .layer(DefaultBodyLimit::max(max_performance_trace_bytes)),
        )
        .route("/ws", handlers.websocket)
        .route("/", handlers.static_assets.clone())
        .route("/*path", handlers.static_assets)
        .layer(DefaultBodyLimit::max(max_request_bytes))
        .layer(middleware::map_response(
            |mut response: Response| async move {
                apply_browser_security_headers(&mut response);
                response
            },
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body, Bytes},
        http::{header, Request, StatusCode},
        response::IntoResponse,
        routing::{get, post},
    };
    use tower::ServiceExt;

    fn handlers() -> BrowserRouteHandlers<()> {
        BrowserRouteHandlers {
            health: get(|| async { "health" }),
            snapshot: get(|| async { "snapshot" }),
            upload: post(|_: Bytes| async { "upload" }),
            performance_trace: post(|| async { StatusCode::NO_CONTENT }),
            websocket: get(|| async { "websocket" }),
            static_assets: get(|request: Request<Body>| async move {
                format!("asset:{}", request.uri().path()).into_response()
            }),
        }
    }

    async fn body(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), 1024)
                .await
                .expect("bounded response body")
                .to_vec(),
        )
        .expect("utf-8 response")
    }

    #[tokio::test]
    async fn fixed_routes_dispatch_only_the_expected_http_methods() {
        let router = build_browser_router((), handlers(), 1024, 64);

        for (path, expected) in [
            ("/api/health", "health"),
            ("/api/snapshot", "snapshot"),
            ("/ws", "websocket"),
            ("/unknown", "asset:/unknown"),
        ] {
            let response = router
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(body(response).await, expected);
        }

        let response = router
            .oneshot(Request::post("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn route_composition_applies_bounds_and_browser_security_headers() {
        let router = build_browser_router((), handlers(), 8, 4);
        let oversized = router
            .clone()
            .oneshot(
                Request::post("/api/upload")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from("123456789"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let response = router
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    }
}

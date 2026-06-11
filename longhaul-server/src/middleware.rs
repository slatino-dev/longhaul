//! Tower middleware for the MCP HTTP transport.
//!
//! ## `Mcp-Method` consistency check
//!
//! The RC requires that the `Mcp-Method` header mirrors the `method` field of
//! the JSON-RPC body. This layer reads the body, checks the two match, then
//! puts the body back so downstream handlers can read it again. When they
//! differ the layer rejects the request with `400 Bad Request` before any MCP
//! parsing occurs, so no JSON-RPC id is available for a proper error response.
//!
//! Header-only checks (presence, format) do not require body buffering and are
//! performed first.

use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use longhaul_core::http::{HEADER_METHOD, HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION};

/// Axum middleware function: validate the `MCP-Protocol-Version` header and,
/// when a body exists, verify `Mcp-Method` matches the body's `method`.
///
/// Mount it with [`axum::middleware::from_fn`].
pub async fn mcp_method_check(req: Request<Body>, next: Next) -> Response {
    // 1. Protocol-version header must be present and must match.
    match req.headers().get(HEADER_PROTOCOL_VERSION) {
        None => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Missing required header: {HEADER_PROTOCOL_VERSION}"),
            )
                .into_response();
        }
        Some(v) if v != PROTOCOL_VERSION => {
            return (
                StatusCode::BAD_REQUEST,
                format!(
                    "Protocol version mismatch: expected {PROTOCOL_VERSION}, got {}",
                    v.to_str().unwrap_or("<non-utf8>")
                ),
            )
                .into_response();
        }
        _ => {}
    }

    // 2. `Mcp-Method` header, if present, must agree with the body's `method`.
    let method_header = req
        .headers()
        .get(HEADER_METHOD)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    if let Some(claimed_method) = method_header {
        // Buffer the body (MCP request bodies are small JSON-RPC envelopes).
        let (parts, body) = req.into_parts();
        let bytes = match axum::body::to_bytes(body, 4 * 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Could not read request body").into_response();
            }
        };

        // Parse just enough to read the `method` key.
        let body_method = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(str::to_owned));

        if let Some(bm) = body_method {
            if bm != claimed_method {
                return (
                    StatusCode::BAD_REQUEST,
                    format!(
                        "{HEADER_METHOD} header ({claimed_method:?}) disagrees with body method ({bm:?})"
                    ),
                )
                    .into_response();
            }
        }

        // Reassemble the request with the buffered body so downstream can read it.
        let req = Request::from_parts(parts, Body::from(bytes));
        next.run(req).await
    } else {
        next.run(req).await
    }
}

/// Axum middleware function: attach standard MCP response headers.
pub async fn mcp_response_headers(req: Request<Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        header::HeaderName::from_static("mcp-protocol-version"),
        header::HeaderValue::from_static(PROTOCOL_VERSION),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::post,
        Router,
    };
    use serde_json::json;
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/mcp", post(|| async { "ok" }))
            .layer(middleware::from_fn(mcp_method_check))
    }

    fn build_request(
        method_header: Option<&str>,
        version_header: Option<&str>,
        body: serde_json::Value,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json");

        if let Some(v) = version_header {
            builder = builder.header(HEADER_PROTOCOL_VERSION, v);
        }
        if let Some(m) = method_header {
            builder = builder.header(HEADER_METHOD, m);
        }

        builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    #[tokio::test]
    async fn rejects_missing_version_header() {
        let req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .body(Body::empty())
            .unwrap();
        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_wrong_version_header() {
        let req = build_request(None, Some("2025-01-01"), json!({"method":"tools/list"}));
        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn accepts_correct_version_no_method_header() {
        let req = build_request(None, Some(PROTOCOL_VERSION), json!({"method":"tools/list"}));
        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_matching_method_header_and_body() {
        let req = build_request(
            Some("tools/list"),
            Some(PROTOCOL_VERSION),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_mismatched_method_header() {
        let req = build_request(
            Some("tools/list"),
            Some(PROTOCOL_VERSION),
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call"}),
        );
        let resp = app().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn response_headers_layer_sets_mcp_protocol_version() {
        let app = Router::new()
            .route("/mcp", post(|| async { "ok" }))
            .layer(middleware::from_fn(mcp_response_headers))
            .layer(middleware::from_fn(mcp_method_check));

        let req = build_request(None, Some(PROTOCOL_VERSION), json!({"method":"tools/list"}));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let hdr = resp
            .headers()
            .get("mcp-protocol-version")
            .expect("mcp-protocol-version response header must be present");
        assert_eq!(hdr, PROTOCOL_VERSION);
    }
}

//! Router construction for the longhaul MCP HTTP server.
//!
//! [`build`] returns an `axum::Router` with:
//! * `POST /mcp` — the stateless JSON-RPC dispatch endpoint.
//! * `GET  /health` and `GET /ready` — liveness / readiness probes.
//!
//! All MCP requests pass through the [`crate::middleware::mcp_method_check`]
//! layer, which enforces the `MCP-Protocol-Version` header and verifies that
//! the `Mcp-Method` header (when present) matches the body `method`.

use std::sync::Arc;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};

use crate::{
    handlers::{dispatch, health, ready},
    middleware::mcp_method_check,
    ServerState,
};

/// Build and return the application router.
pub fn build(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route(
            "/mcp",
            post(dispatch).layer(middleware::from_fn(mcp_method_check)),
        )
        .with_state(state)
}

// longhaul-server — axum-based MCP HTTP server framework
//
// Phase A: scaffold only.  Modules are stubs with TODO markers.
// Real handler logic lands in Phase B once protocol types are defined in longhaul-core.

/// Router construction: mount the `/mcp` endpoint, health-check, and optional SSE stream.
/// TODO: build axum `Router` with `POST /mcp` (stateless JSON-RPC dispatch) +
///       `GET /mcp/events` (SSE for server-initiated notifications).
pub mod router {}

/// Dispatcher: match JSON-RPC `method` → handler function.
/// TODO: trait `McpHandler` + dispatch table.
pub mod dispatch {}

/// Middleware: request-id injection, structured logging, auth token validation.
/// TODO: tower `Layer` impls.
pub mod middleware {}

/// Server configuration: bind address, TLS cert paths, capability flags.
/// TODO: `ServerConfig` struct + builder.
pub mod config {}

/// Tasks extension HTTP handlers (experimental).
/// TODO: implement `tasks/create`, `tasks/get`, `tasks/update`, `tasks/cancel`, `tasks/list`.
pub mod tasks {}

/// Health and readiness endpoints (`/health`, `/ready`).
/// TODO: axum handlers returning JSON status payloads.
pub mod health {}

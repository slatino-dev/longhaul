//! # longhaul-server
//!
//! Axum-based MCP 2026-07-28 RC server framework.
//!
//! ## Architecture
//!
//! ```text
//!   POST /mcp
//!     │
//!     ├─ mcp_method_check middleware
//!     │    ├─ validates MCP-Protocol-Version header
//!     │    └─ verifies Mcp-Method header ↔ body method agreement
//!     │
//!     └─ dispatch handler
//!          ├─ server/discover   → DiscoverResult
//!          ├─ tools/list        → Registry::list_result
//!          ├─ tools/call        → ToolHandler::call → ToolCallOutcome
//!          ├─ tasks/get         → TaskStore::get
//!          ├─ tasks/update      → TaskStore::update
//!          └─ tasks/cancel      → TaskStore::cancel
//! ```
//!
//! ## Statelessness
//!
//! Every request is self-contained. The `TaskStore` trait abstracts the
//! backing store so that [`store::MemoryStore`] (single-process) or
//! [`store::SqliteStore`] (file-backed, shareable across OS processes) can be
//! plugged in. When two server instances mount the same [`store::SqliteStore`]
//! database file, a single client's task lifecycle can be round-robined across
//! both instances and still complete correctly — the statelessness test in
//! `tests/stateless.rs` verifies this property.

use std::sync::Arc;

use longhaul_core::discover::DiscoverResult;

pub mod handlers;
pub mod middleware;
pub mod registry;
pub mod router;
pub mod store;

pub use registry::{async_trait, Registry, ToolEntry, ToolError, ToolHandler};
pub use store::{MemoryStore, SqliteStore, StoreError, TaskStore};

/// Shared server state threaded through every axum handler via
/// [`axum::extract::State`].
pub struct ServerState {
    /// The capability discovery result returned by `server/discover`.
    pub discover_result: DiscoverResult,
    /// The tool registry: definitions and handlers.
    pub registry: Registry,
    /// The task backing store.
    pub store: Arc<dyn TaskStore>,
}

/// Build and bind the server, returning a future that resolves when the server
/// stops. Pair with `tokio::net::TcpListener::bind` in your `main`.
///
/// # Example
///
/// ```no_run
/// # use std::sync::Arc;
/// # use longhaul_server::{ServerState, MemoryStore, Registry, serve};
/// # use longhaul_core::discover::{DiscoverResult, Implementation, ServerCapabilities};
/// # use longhaul_core::http::PROTOCOL_VERSION;
/// # async fn example() {
/// let state = Arc::new(ServerState {
///     discover_result: DiscoverResult {
///         protocol_version: PROTOCOL_VERSION.to_owned(),
///         server_info: Implementation::new("my-server", "0.1.0"),
///         capabilities: ServerCapabilities::default(),
///         instructions: None,
///         meta: None,
///     },
///     registry: Registry::default(),
///     store: Arc::new(MemoryStore::new()),
/// });
///
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
/// serve(listener, state).await;
/// # }
/// ```
pub async fn serve(listener: tokio::net::TcpListener, state: Arc<ServerState>) {
    let app = router::build(state);
    axum::serve(listener, app).await.expect("server error");
}

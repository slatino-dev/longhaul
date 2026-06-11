//! # longhaul-core
//!
//! Typed Rust protocol structs (serde) for the **MCP 2026-07-28 release
//! candidate**.
//!
//! **Pinned spec revision: `2026-07-28` (release candidate)** — exposed as
//! [`http::PROTOCOL_VERSION`]. Wire names, error codes and lifecycle
//! semantics in this crate track that revision; where the RC text is
//! ambiguous, the interpretation is documented inline as a dated note next
//! to the type it affects.
//!
//! ## Covered (the longhaul subset of the RC)
//!
//! * [`jsonrpc`] — the JSON-RPC 2.0 envelope (request / notification /
//!   response; batch arrays stay removed, as since MCP 2025-06-18).
//! * [`meta`] — per-request `_meta`: client identity under
//!   `io.modelcontextprotocol/clientInfo`, W3C trace context
//!   (`traceparent` / `tracestate` / `baggage`), vendor-key pass-through.
//! * [`http`] — transport header constants (`MCP-Protocol-Version`,
//!   `Mcp-Method`, `Mcp-Name`).
//! * [`error`] — JSON-RPC error object + code constants, including the RC's
//!   `-32002` → `-32602` invalid-params consolidation.
//! * [`discover`] — `server/discover` capability discovery (the RC removed
//!   the `initialize` handshake).
//! * [`tools`] — `tools/list` (with `ttlMs` / `cacheScope` cache metadata)
//!   and `tools/call`; schemas are JSON Schema 2020-12 carried untyped as
//!   [`serde_json::Value`] with depth-bounded validation; the
//!   [`tools::ToolCallOutcome`] union covers content / task-handle /
//!   input-required results.
//! * [`tasks`] — the Tasks extension: task object + lifecycle state
//!   machine, `tasks/get` / `tasks/update` / `tasks/cancel`
//!   (`tasks/list` was removed by the RC), task-handle and
//!   input-required `tools/call` outcomes.
//!
//! ## Deliberately out of scope (this crate version)
//!
//! Resources, prompts, sampling, completion, roots and logging types are not
//! modelled yet; non-text tool content blocks pass through untyped. See the
//! repository README for the full coverage table.

#![warn(missing_docs)]

pub mod discover;
pub mod error;
pub mod http;
pub mod jsonrpc;
pub mod meta;
pub mod tasks;
pub mod tools;

mod tag;

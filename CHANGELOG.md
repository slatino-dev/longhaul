# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-06-11

Initial public release. Implements the MCP 2026-07-28 release candidate — the
first RC to specify the Tasks extension for long-running tool calls.

### Added

**`longhaul-core`**
- JSON-RPC 2.0 envelope: `Request`, `Response`, `Notification` with strict
  `"jsonrpc":"2.0"` zero-sized tag and typed `id` field.
- Per-request `_meta`: `io.modelcontextprotocol/clientInfo`, W3C `traceparent` /
  `tracestate` / `baggage`, vendor-key pass-through via `extra` map.
- HTTP transport constants: `MCP-Protocol-Version` (`"2026-07-28"`), `Mcp-Method`,
  `Mcp-Name`.
- Error object and code constants including the RC's `-32002` → `-32602`
  consolidation; `normalize_code` bridge for pre-RC peers.
- `server/discover` capability types (`DiscoverResult`, `ServerCapabilities`,
  `ToolsCapability`, `TasksCapability`, `Implementation`).
- `tools/list` with `ttlMs` + `cacheScope` cache metadata; `tools/call` with the
  `ToolCallOutcome` union covering `Content` / `Task` / `InputRequired` outcomes.
  Tool input/output schemas carried as untyped JSON Schema 2020-12 `Value`s with
  depth-bounded structural validation.
- Tasks extension: `Task`, `TaskStatus` (camelCase wire names), lifecycle state
  machine enforced by `can_transition_to` and `Task::cancel` (idempotent).
  `tasks/get`, `tasks/update`, `tasks/cancel` param + result types.
  `tasks/list` deliberately absent (removed in the RC).
- `InputRequiredResult` + `TaskHandleResult` with `"resultType"` zero-sized
  discriminator tags via `string_tag!`.
- Exhaustive 5×5 transition-matrix test, wire-string pinning tests, and
  hand-written JSON fixture round-trips.

**`longhaul-server`**
- axum MCP server with `POST /mcp`, `GET /health`, `GET /ready`.
- `mcp_method_check` middleware: validates `MCP-Protocol-Version` header and
  verifies `Mcp-Method` header agrees with body `method`.
- `mcp_response_headers` middleware: attaches `MCP-Protocol-Version` to every
  response; mounted on `/mcp` route.
- `dispatch` handler: routes all six MCP methods to typed handlers; maps
  `ToolError::InvalidParams` to `-32602`, store/internal failures to `-32603`.
- `ToolHandler` trait and `Registry`: append-only at construction, immutable +
  `Arc`-shared thereafter; `tools/list` result with stable sorted order.
- `TaskStore` trait with `MemoryStore` (`Mutex<HashMap>`) and `SqliteStore`
  (WAL-mode rusqlite with conditional-`WHERE` write guard to protect absorbing
  states under concurrent writes).
- `serve` function wrapping `axum::serve`.
- Integration test: two independent `SqliteStore` instances on one temp file,
  round-robining a full task lifecycle across them; idempotent double-cancel test;
  illegal-transition `-32602` test.

**`longhaul-conformance`**
- CLI runner (`--url`, `--filter`, `--json`, nonzero exit on failure).
- Three suites: `discovery` (schema + `protocolVersion` pin + JSON-RPC envelope),
  `tools` (list result shape + cache metadata types), `tasks` (error codes for
  missing-task operations + `-32601` for the removed `tasks/list`).
- JSON Schema fixtures for `jsonrpc_request`, `jsonrpc_response`,
  `tools_list_result`, `task`, `discover_result`; embedded at compile time via
  `include_str!`.
- In-process test suite (`tests/conformance.rs`) that boots a real server on a
  random port and exercises the full lifecycle.

**`examples/indexer`**
- `index_directory` tool: walks a directory tree counting words per file
  extension, returns a `TaskHandleResult` immediately, transitions to `completed`
  (or `failed`) on worker exit.
- Cancellation: monitor task polls store every 250 ms and fires the watch channel
  when `tasks/cancel` sets the store status to `Cancelled`; `walk_and_index`
  checks the channel between directories and aborts early.
- `InputRequired` round-trip: when multiple `src`-like sub-directories are found
  the tool pauses, returns `inputRequired` with a schema-constrained choice, and
  resumes on retry with `inputResponses` + echoed `requestState` token.
- SQLite-backed by default; falls back to `MemoryStore` if the db cannot be opened.
- CLI: `--dir`, `--bind`, `--db`.

**Tooling**
- CI matrix: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test --workspace`, scrub gate blocking private network hostnames and API-key
  patterns, on `ubuntu-latest` and `windows-latest`.
- `scripts/scrub_check.sh` — the scrub gate used in CI.

[0.1.0]: https://github.com/latinosammy2/longhaul/releases/tag/v0.1.0

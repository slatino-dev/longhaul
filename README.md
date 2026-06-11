# longhaul

A Rust workspace implementing an MCP (Model Context Protocol) 2026-07-28 release-candidate server: stateless HTTP core, the Tasks extension, and a conformance suite.

**Status: `longhaul-core` protocol types implemented (2026-07-28 RC subset, pinned) — server + conformance land next**

## Workspace members

| Crate | Kind | Purpose |
|---|---|---|
| `longhaul-core` | lib | Protocol types (JSON-RPC envelope, capabilities, resources, tools, tasks) |
| `longhaul-server` | lib | axum-based stateless HTTP dispatch layer |
| `longhaul-conformance` | bin | Conformance test runner — point at a URL, get a pass/fail report |
| `examples/indexer` | bin | Example: serve a local directory as MCP resources |

## Quickstart

```sh
cargo build
cargo test
```

Run the conformance tool against a local server:

```sh
cargo run -p longhaul-conformance -- --url http://localhost:3000
```

## Protocol coverage (`longhaul-core`, pinned to the 2026-07-28 RC)

| Area | Status |
|---|---|
| JSON-RPC 2.0 envelope (request / notification / response) | ✅ typed, with strict `"jsonrpc":"2.0"` tag |
| Per-request `_meta`: `io.modelcontextprotocol/clientInfo`, W3C `traceparent`/`tracestate`/`baggage` | ✅ typed + vendor-key pass-through |
| HTTP header constants: `MCP-Protocol-Version` (`2026-07-28`), `Mcp-Method`, `Mcp-Name` | ✅ |
| `server/discover` (replaces the removed `initialize` handshake) | ✅ |
| `tools/list` (incl. `ttlMs` / `cacheScope` cache metadata) + `tools/call` | ✅ |
| Tool schemas | carried as untyped JSON Schema 2020-12 `Value`s by design; depth-bounded structural validation only |
| Tasks extension: task object, lifecycle state machine, `tasks/get` / `tasks/update` / `tasks/cancel` | ✅ (`tasks/list` removed by the RC — intentionally absent) |
| `inputRequired` tool-call outcome + `inputResponses`/`requestState` retry | ✅ |
| Error codes (incl. the RC's `-32002` → `-32602` consolidation) | ✅ |

## Limitations

- Resources, prompts, sampling, completion, roots and logging types are not modelled yet.
- Non-text tool content blocks (image / audio / embedded resource) round-trip untyped.
- JSON-RPC batch arrays are not supported (removed from MCP in 2025-06-18; still absent in the RC).
- Where the RC is ambiguous (task status casing, the task-handle `resultType`, cancel-on-completed semantics), the chosen interpretation is documented as a dated note next to the type in `longhaul-core`.

# longhaul

A Rust implementation of an [MCP](https://modelcontextprotocol.io/) server targeting the **2026-07-28 release candidate** — the first RC to introduce the Tasks extension for long-running tool calls. The name reflects the goal: outlasting a single round-trip.

**Pinned to the 2026-07-28 RC revision.** Will track the July 28 final when published; the dated interpretive notes in `longhaul-core/src/tasks.rs` and `longhaul-core/src/tools.rs` mark the exact spots where the RC is ambiguous and where divergence risk lives.

---

## The problem

Most MCP server examples answer synchronously. The RC's Tasks extension changes that: a `tools/call` can return a task handle immediately and let the client poll `tasks/get` until the work completes — or cancel it midway. Implementing that correctly across multiple server instances requires a shared, consistent task store. This repo builds the full stack: typed protocol structs, an axum dispatch layer, both in-process and SQLite-backed stores, and a conformance runner to verify any server's behaviour.

---

## Architecture

```
POST /mcp
  │
  ├─ mcp_method_check middleware
  │    ├─ validates MCP-Protocol-Version header (must be "2026-07-28")
  │    └─ verifies Mcp-Method header ↔ body method agreement
  │
  ├─ mcp_response_headers middleware
  │    └─ attaches MCP-Protocol-Version to every response
  │
  └─ dispatch handler
       ├─ server/discover   → DiscoverResult
       ├─ tools/list        → Registry::list_result (ttlMs + cacheScope)
       ├─ tools/call        → ToolHandler::call → ToolCallOutcome
       │                         ├─ Content        (immediate result)
       │                         ├─ Task           (task handle)
       │                         └─ InputRequired  (pause for client input)
       ├─ tasks/get         → TaskStore::get
       ├─ tasks/update      → TaskStore::update (enforces state machine)
       └─ tasks/cancel      → TaskStore::cancel (idempotent)

TaskStore
  ├─ MemoryStore   — Mutex<HashMap>, single process
  └─ SqliteStore   — WAL-mode SQLite; multiple server instances
                     can share one file and service the same task lifecycle
```

The statelessness guarantee — two server instances round-robining a single client's `tasks/get` → `tasks/update` → `tasks/cancel` sequence against one `SqliteStore` file — is an integration test, not just a doc claim (`longhaul-server/tests/stateless.rs`).

---

## Workspace

| Crate | Kind | Purpose |
|---|---|---|
| `longhaul-core` | lib | Protocol types: JSON-RPC envelope, error codes, capabilities, tools, tasks |
| `longhaul-server` | lib | axum MCP server: middleware, dispatch, tool registry, TaskStore |
| `longhaul-conformance` | bin | CLI conformance runner — point at any URL, get a pass/fail report |
| `examples/indexer` | bin | Long-running directory indexer demonstrating Tasks + InputRequired |

**Stack:** Rust stable · [axum](https://docs.rs/axum) 0.8 · [rusqlite](https://docs.rs/rusqlite) (WAL) · [tokio](https://docs.rs/tokio) · [serde_json](https://docs.rs/serde_json) · [clap](https://docs.rs/clap) · [jsonschema](https://docs.rs/jsonschema)

---

## Quickstart

```sh
cargo build --workspace
cargo test --workspace
```

Run the indexer example (indexes the current directory):

```sh
cargo run -p indexer -- --dir . --bind 127.0.0.1:3000
```

Run the conformance suite against it in a second terminal:

```sh
cargo run -p longhaul-conformance -- --url http://localhost:3000
# or filter to a single suite:
cargo run -p longhaul-conformance -- --url http://localhost:3000 --filter tasks
# or get machine-readable JSON:
cargo run -p longhaul-conformance -- --url http://localhost:3000 --json
```

---

## Protocol coverage

| Area | Status |
|---|---|
| JSON-RPC 2.0 envelope (request / notification / response) | typed, strict `"jsonrpc":"2.0"` tag |
| Per-request `_meta`: `io.modelcontextprotocol/clientInfo`, W3C `traceparent`/`tracestate`/`baggage` | typed + vendor-key pass-through |
| HTTP header constants: `MCP-Protocol-Version` (`2026-07-28`), `Mcp-Method`, `Mcp-Name` | present on every response |
| `server/discover` (replaces the removed `initialize` handshake) | implemented |
| `tools/list` (incl. `ttlMs` / `cacheScope` cache metadata) + `tools/call` | implemented |
| Tool schemas | carried as untyped JSON Schema 2020-12 `Value`s; depth-bounded structural validation only |
| Tasks extension: task object, lifecycle state machine, `tasks/get` / `tasks/update` / `tasks/cancel` | implemented (`tasks/list` was removed in the RC — intentionally absent) |
| `inputRequired` tool-call outcome + `inputResponses`/`requestState` retry round-trip | implemented |
| Error codes (including the RC's `-32002` → `-32602` consolidation) | implemented |

---

## Test inventory (88 tests, all green)

- **`longhaul-core`** (34): exhaustive 5×5 transition-matrix test encodes the state machine spec; terminal-absorbing and same-state-is-not-a-transition properties asserted separately; wire strings pinned (including rejecting `"canceled"` and `"input_required"`); hand-written JSON round-trip fixtures that pin the *wire format* rather than the implementation's own serialiser.
- **`longhaul-server` unit** (20): store lifecycle, illegal-transition detection, idempotent cancel, tool registry ordering, middleware header validation.
- **`longhaul-server` integration** (8 in `tests/stateless.rs`): two-instance statelessness proof over a shared SQLite file; full HTTP-level task lifecycle; `tasks/cancel` idempotency; illegal transition returns `-32602`.
- **`longhaul-conformance` in-process** (1 suite, exercising discovery + tools + tasks): starts a real server on a random port; inserts tasks directly via the store; exercises error codes.

---

## What I would do differently

- **SqliteStore across *processes*** needs a `BEGIN IMMEDIATE` transaction wrapping the read-validate-write triple, not just a conditional `WHERE status = ?` guard. The current guard protects within a single connection (WAL readers + one writer) but won't catch a cross-process race where two writers both pass the status check before either commits. For this demo scale it's acceptable; for production use an `IMMEDIATE` transaction or Postgres row-level locking.
- **`tasks/cancel` cancellation propagation** is poll-based in the indexer (250 ms intervals). A production tool handler should accept an injection point — a `CancellationToken` à la `tokio_util::sync` — so that cancellation can interrupt blocking I/O rather than wait for a directory boundary.
- **No pagination** on `tools/list`. The RC's cursor-based pagination would require the registry to be sorted and sliceable on a stable key; for the registries this demo targets (≤dozens of tools) it's not a priority.
- Resources, prompts, sampling, completion, roots and logging are not modelled. The `extra` field on every struct round-trips them verbatim but gives no typed access.

---

## Engineering log

Built in a focused session against the RC text (2026-06-11). Commits stage the layers in dependency order: core types → tool registry → axum server → integration tests → conformance runner → indexer example. The four commits closest in time reflect that server and tests were written back-to-back; the layer boundaries are deliberate, not accidental.

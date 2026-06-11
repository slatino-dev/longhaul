# longhaul

A Rust workspace implementing an MCP (Model Context Protocol) 2026-07-28 release-candidate server: stateless HTTP core, the Tasks extension, and a conformance suite.

**Status: scaffolding — core lands next**

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

## Limitations

<!-- TODO: fill in once protocol logic is implemented -->

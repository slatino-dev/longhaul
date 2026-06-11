// longhaul-core — MCP 2026-07-28 RC protocol types
//
// Phase A: scaffold only.  Each module is a stub with a TODO marker.
// Implement the actual protocol types in Phase B.

/// JSON-RPC 2.0 message envelope types (request / response / notification / batch).
/// TODO: define `Request`, `Response`, `Notification`, `BatchRequest` structs.
pub mod jsonrpc {}

/// MCP capability negotiation — `initialize` / `initialized` lifecycle.
/// TODO: `ClientCapabilities`, `ServerCapabilities`, `InitializeParams`, `InitializeResult`.
pub mod capabilities {}

/// MCP resource primitives: `Resource`, `ResourceContents`, `ResourceTemplate`.
/// TODO: implement resource list/read/subscribe types per spec §5.
pub mod resources {}

/// MCP tool invocation types: `Tool`, `ToolCall`, `ToolResult`, `ToolAnnotations`.
/// TODO: implement per spec §6.
pub mod tools {}

/// MCP prompt types: `Prompt`, `PromptMessage`, `GetPromptResult`.
/// TODO: implement per spec §7.
pub mod prompts {}

/// Tasks extension (experimental, 2026-07-28 RC).
/// TODO: `Task`, `TaskStatus`, `TaskCreate/Update/Cancel/List` request/response types.
pub mod tasks {}

/// Sampling types: `CreateMessageRequest`, `CreateMessageResult`, `SamplingMessage`.
/// TODO: implement per spec §8.
pub mod sampling {}

/// Shared error codes and the `McpError` wrapper.
/// TODO: wire up JSON-RPC error codes (-32700 parse, -32600 invalid, -32601 method,
///       -32602 params, -32603 internal) plus MCP-layer codes.
pub mod error {}

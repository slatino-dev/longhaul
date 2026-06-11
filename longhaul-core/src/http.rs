//! HTTP transport model constants (MCP 2026-07-28 RC).
//!
//! The RC's streamable-HTTP transport identifies the protocol revision and
//! mirrors key request facts into headers so HTTP infrastructure (routers,
//! caches, rate limiters, log pipelines) can act without parsing the
//! JSON-RPC body.

/// The pinned protocol revision implemented by this crate.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// Header carrying the negotiated protocol revision on every HTTP request
/// and response. Value: [`PROTOCOL_VERSION`].
pub const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";

/// Header mirroring the JSON-RPC `method` of the request in the body
/// (e.g. `tools/call`), so intermediaries can route or meter per method.
pub const HEADER_METHOD: &str = "Mcp-Method";

/// Header mirroring the primary primitive name addressed by the request
/// (e.g. the tool name of a `tools/call`), when one exists.
pub const HEADER_NAME: &str = "Mcp-Name";

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the exact header names + revision string: these are wire
    /// constants, and a typo here is a silent interop break.
    #[test]
    fn wire_constants_are_pinned() {
        assert_eq!(PROTOCOL_VERSION, "2026-07-28");
        assert_eq!(HEADER_PROTOCOL_VERSION, "MCP-Protocol-Version");
        assert_eq!(HEADER_METHOD, "Mcp-Method");
        assert_eq!(HEADER_NAME, "Mcp-Name");
    }
}

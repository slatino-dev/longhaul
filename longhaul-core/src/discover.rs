//! `server/discover` — capability discovery (MCP 2026-07-28 RC).
//!
//! The 2026-07-28 RC removes the stateful `initialize` / `notifications/initialized`
//! handshake. In its place a client may call the stateless [`METHOD_DISCOVER`]
//! at any time to learn the server's identity, protocol revision and
//! capabilities. Client identity no longer travels in discovery params — it
//! rides on **every** request in `_meta` under
//! [`crate::meta::KEY_CLIENT_INFO`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::meta::Meta;

/// Method name for capability discovery.
pub const METHOD_DISCOVER: &str = "server/discover";

/// Params for `server/discover`. Discovery itself needs no arguments; the
/// optional `_meta` carries client identity and trace context like any
/// other request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverParams {
    /// Request metadata (client identity, trace context, vendor keys).
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Name/version (and optional human-readable title) of an implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Implementation {
    /// Machine-readable implementation name.
    pub name: String,
    /// Implementation version string.
    pub version: String,
    /// Optional display title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Implementation {
    /// Construct an `Implementation` from name + version.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            title: None,
        }
    }
}

/// Result of `server/discover`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResult {
    /// The protocol revision the server speaks, e.g.
    /// [`crate::http::PROTOCOL_VERSION`].
    pub protocol_version: String,
    /// Server identity.
    pub server_info: Implementation,
    /// What the server can do.
    pub capabilities: ServerCapabilities,
    /// Optional usage hints for the client/model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Result metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Server capability flags advertised by `server/discover`.
///
/// Only the capability sections this crate models are typed (`tools`,
/// `tasks`); other sections (`resources`, `prompts`, `logging`, …) round-trip
/// untyped through [`ServerCapabilities::extra`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Present iff the server offers tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Present iff the server supports the Tasks extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<TasksCapability>,
    /// Capability sections not modelled by this crate, preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// The `tools` capability section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    /// Whether the server emits `notifications/tools/list_changed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// The `tasks` capability section (Tasks extension). The RC defines no
/// required sub-fields; any it adds later round-trip via `extra`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasksCapability {
    /// Forward-compatible pass-through of sub-capabilities.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_result_round_trips() {
        let raw = r#"{
            "protocolVersion": "2026-07-28",
            "serverInfo": {"name": "longhaul-indexer", "version": "0.1.0", "title": "Longhaul Indexer"},
            "capabilities": {
                "tools": {"listChanged": true},
                "tasks": {},
                "resources": {"subscribe": false}
            },
            "instructions": "Index and search local directories."
        }"#;
        let result: DiscoverResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.protocol_version, crate::http::PROTOCOL_VERSION);
        assert_eq!(result.server_info.name, "longhaul-indexer");
        assert_eq!(
            result.capabilities.tools.as_ref().unwrap().list_changed,
            Some(true)
        );
        assert!(result.capabilities.tasks.is_some());
        // Unmodelled capability sections survive the round trip.
        assert!(result.capabilities.extra.contains_key("resources"));

        let back = serde_json::to_value(&result).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn discover_params_default_is_empty_object() {
        let params = DiscoverParams::default();
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            serde_json::json!({})
        );
    }
}

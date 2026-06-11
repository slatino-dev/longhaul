//! Request metadata: the `_meta` object (MCP 2026-07-28 RC).
//!
//! In the 2026-07-28 RC every request's `params` may carry a `_meta` object.
//! Two groups of keys are reserved by the spec:
//!
//! * `io.modelcontextprotocol/clientInfo` — with the `initialize` handshake
//!   removed, the client identifies itself **per request** under this
//!   reverse-DNS-prefixed key (see [`ClientInfo`]).
//! * The W3C Trace Context / Baggage keys `traceparent`, `tracestate` and
//!   `baggage`, carried verbatim so traces survive transport hops that strip
//!   HTTP headers.
//!
//! All other keys pass through untouched in [`Meta::extra`], so unknown
//! vendor-prefixed metadata round-trips without loss.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `_meta` key for per-request client identification (replaces the
/// `clientInfo` field of the removed `initialize` request).
pub const KEY_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// `_meta` key for the W3C Trace Context `traceparent` field.
pub const KEY_TRACEPARENT: &str = "traceparent";
/// `_meta` key for the W3C Trace Context `tracestate` field.
pub const KEY_TRACESTATE: &str = "tracestate";
/// `_meta` key for the W3C Baggage header value.
pub const KEY_BAGGAGE: &str = "baggage";

/// Client identification, sent per request under
/// [`KEY_CLIENT_INFO`] in `_meta`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Machine-readable client name, e.g. `"longhaul-conformance"`.
    pub name: String,
    /// Client version string, e.g. `"0.1.0"`.
    pub version: String,
}

impl ClientInfo {
    /// Construct a `ClientInfo` from name + version.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

/// The `_meta` object attached to request params (and, where the RC allows,
/// to results and protocol objects).
///
/// Spec-reserved keys are typed; everything else round-trips via
/// [`Meta::extra`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// Per-request client identity (`io.modelcontextprotocol/clientInfo`).
    #[serde(
        rename = "io.modelcontextprotocol/clientInfo",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_info: Option<ClientInfo>,
    /// W3C Trace Context `traceparent` (e.g. `00-<trace-id>-<span-id>-01`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    /// W3C Trace Context `tracestate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
    /// W3C Baggage value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baggage: Option<String>,
    /// All non-reserved `_meta` keys, preserved verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Meta {
    /// Convenience: a `_meta` carrying only client identification.
    pub fn with_client_info(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            client_info: Some(ClientInfo::new(name, version)),
            ..Self::default()
        }
    }

    /// True when no reserved key is set and no extra keys are present
    /// (i.e. serializing would produce `{}`).
    pub fn is_empty(&self) -> bool {
        self.client_info.is_none()
            && self.traceparent.is_none()
            && self.tracestate.is_none()
            && self.baggage.is_none()
            && self.extra.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_info_lives_under_the_reverse_dns_key() {
        let meta = Meta::with_client_info("longhaul-conformance", "0.1.0");
        let value = serde_json::to_value(&meta).unwrap();
        assert_eq!(
            value[KEY_CLIENT_INFO],
            serde_json::json!({"name": "longhaul-conformance", "version": "0.1.0"})
        );
        // No other keys serialized.
        assert_eq!(value.as_object().unwrap().len(), 1);
    }

    #[test]
    fn trace_context_keys_round_trip() {
        let json = r#"{
            "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
            "tracestate": "vendor=opaque",
            "baggage": "userId=alice"
        }"#;
        let meta: Meta = serde_json::from_str(json).unwrap();
        assert_eq!(
            meta.traceparent.as_deref(),
            Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
        );
        assert_eq!(meta.tracestate.as_deref(), Some("vendor=opaque"));
        assert_eq!(meta.baggage.as_deref(), Some("userId=alice"));
        assert!(meta.extra.is_empty());

        let back = serde_json::to_value(&meta).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(json).unwrap());
    }

    #[test]
    fn unknown_keys_are_preserved_in_extra() {
        let json = r#"{"com.example/requestTag": "abc", "traceparent": "00-x-y-01"}"#;
        let meta: Meta = serde_json::from_str(json).unwrap();
        assert_eq!(
            meta.extra.get("com.example/requestTag"),
            Some(&Value::String("abc".into()))
        );
        let back = serde_json::to_value(&meta).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(json).unwrap());
    }

    #[test]
    fn empty_meta_serializes_to_empty_object() {
        let meta = Meta::default();
        assert!(meta.is_empty());
        assert_eq!(serde_json::to_value(&meta).unwrap(), serde_json::json!({}));
    }
}

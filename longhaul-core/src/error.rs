//! JSON-RPC error object and error-code constants (MCP 2026-07-28 RC).
//!
//! ## RC change note — `-32002` → `-32602` (recorded 2026-06-11)
//!
//! Pre-RC MCP revisions reported invalid request/tool parameters with the
//! implementation-defined code `-32002`. The 2026-07-28 RC retires that code
//! and folds the condition into the standard JSON-RPC `-32602` *Invalid
//! params*. [`PRE_RC_INVALID_PARAMS`] is retained **only** so bridges talking
//! to pre-RC peers can normalize incoming codes (see [`normalize_code`]);
//! new code must emit [`INVALID_PARAMS`].

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Invalid JSON was received by the server (JSON-RPC 2.0 §5.1).
pub const PARSE_ERROR: i64 = -32700;
/// The JSON sent is not a valid Request object.
pub const INVALID_REQUEST: i64 = -32600;
/// The method does not exist / is not available.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Invalid method parameter(s). In the 2026-07-28 RC this also covers the
/// condition pre-RC peers reported as `-32002` (see module docs).
pub const INVALID_PARAMS: i64 = -32602;
/// Internal JSON-RPC error.
pub const INTERNAL_ERROR: i64 = -32603;

/// Lower bound (inclusive) of the JSON-RPC server-defined error range.
pub const SERVER_ERROR_MIN: i64 = -32099;
/// Upper bound (inclusive) of the JSON-RPC server-defined error range.
pub const SERVER_ERROR_MAX: i64 = -32000;

/// The retired pre-RC "invalid params" code. Do not emit; kept for
/// normalizing traffic from pre-RC peers (see module docs, dated 2026-06-11).
pub const PRE_RC_INVALID_PARAMS: i64 = -32002;

/// True if `code` falls in the JSON-RPC implementation-defined server range.
pub fn is_server_error(code: i64) -> bool {
    (SERVER_ERROR_MIN..=SERVER_ERROR_MAX).contains(&code)
}

/// Map error codes from pre-RC peers onto their 2026-07-28 RC equivalents.
///
/// Currently the only remapping is `-32002` → `-32602`; every other code is
/// returned unchanged.
pub fn normalize_code(code: i64) -> i64 {
    if code == PRE_RC_INVALID_PARAMS {
        INVALID_PARAMS
    } else {
        code
    }
}

/// The JSON-RPC `error` object carried by an error response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    /// Numeric error code (see the constants in this module).
    pub code: i64,
    /// Short human-readable summary of the error.
    pub message: String,
    /// Optional structured detail, passed through verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorObject {
    /// Construct an error with `code` + `message` and no `data`.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Attach structured `data` to the error.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Convenience constructor for `-32602` *Invalid params*.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    /// Convenience constructor for `-32601` *Method not found*.
    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("Method not found: {method}"))
    }
}

impl fmt::Display for ErrorObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for ErrorObject {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_code_values_are_pinned() {
        assert_eq!(PARSE_ERROR, -32700);
        assert_eq!(INVALID_REQUEST, -32600);
        assert_eq!(METHOD_NOT_FOUND, -32601);
        assert_eq!(INVALID_PARAMS, -32602);
        assert_eq!(INTERNAL_ERROR, -32603);
        assert_eq!(PRE_RC_INVALID_PARAMS, -32002);
    }

    #[test]
    fn pre_rc_invalid_params_normalizes_to_32602() {
        assert_eq!(normalize_code(PRE_RC_INVALID_PARAMS), INVALID_PARAMS);
        // Everything else is left alone.
        assert_eq!(normalize_code(INVALID_PARAMS), INVALID_PARAMS);
        assert_eq!(normalize_code(PARSE_ERROR), PARSE_ERROR);
        assert_eq!(normalize_code(-32001), -32001);
    }

    #[test]
    fn server_error_range_is_inclusive() {
        assert!(is_server_error(-32000));
        assert!(is_server_error(-32099));
        assert!(is_server_error(PRE_RC_INVALID_PARAMS));
        assert!(!is_server_error(-32100));
        assert!(!is_server_error(INVALID_PARAMS));
    }

    #[test]
    fn error_object_omits_absent_data() {
        let err = ErrorObject::invalid_params("missing required field 'name'");
        let value = serde_json::to_value(&err).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"code": -32602, "message": "missing required field 'name'"})
        );

        let with_data = err.with_data(serde_json::json!({"field": "name"}));
        let value = serde_json::to_value(&with_data).unwrap();
        assert_eq!(value["data"], serde_json::json!({"field": "name"}));
    }
}

//! JSON-RPC 2.0 message envelope (MCP 2026-07-28 RC profile).
//!
//! The RC keeps plain JSON-RPC 2.0 requests, notifications and responses.
//! Batch arrays are **not** modelled: JSON-RPC batching was removed from MCP
//! in the 2025-06-18 revision and remains absent in the 2026-07-28 RC.
//!
//! Typed per-method params/results live in the sibling modules
//! ([`crate::discover`], [`crate::tools`], [`crate::tasks`]); the envelope
//! carries them as [`serde_json::Value`] and converts at the edge via
//! [`Request::params_as`] / [`Request::with_params`].

use std::fmt;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use crate::error::ErrorObject;
use crate::tag::string_tag;

/// The JSON-RPC protocol version string.
pub const VERSION: &str = "2.0";

string_tag! {
    /// The required `"jsonrpc": "2.0"` envelope field. Zero-sized; refuses
    /// any other value on deserialize.
    pub struct Version = "2.0";
}

/// A JSON-RPC request/response id: string or number (`Null` appears only on
/// error responses to unparseable requests, per JSON-RPC 2.0 §5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// Integer id. JSON-RPC discourages fractional ids; this crate rejects
    /// them outright.
    Number(i64),
    /// String id.
    String(String),
    /// `null` id — only valid on a [`Response`] when the request id could
    /// not be determined (e.g. a parse error).
    Null,
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{n}"),
            RequestId::String(s) => write!(f, "{s}"),
            RequestId::Null => write!(f, "null"),
        }
    }
}

impl From<i64> for RequestId {
    fn from(n: i64) -> Self {
        RequestId::Number(n)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        RequestId::String(s.to_owned())
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        RequestId::String(s)
    }
}

/// A JSON-RPC request (has an `id`, expects a response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// Caller-chosen id echoed back on the response.
    pub id: RequestId,
    /// Method name, e.g. `"tools/call"`.
    pub method: String,
    /// Raw params; convert with [`Request::params_as`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request {
    /// Build a request with no params.
    pub fn new(id: impl Into<RequestId>, method: impl Into<String>) -> Self {
        Self {
            jsonrpc: Version,
            id: id.into(),
            method: method.into(),
            params: None,
        }
    }

    /// Build a request from typed params (serialized to `Value`).
    pub fn with_params<P: Serialize>(
        id: impl Into<RequestId>,
        method: impl Into<String>,
        params: &P,
    ) -> serde_json::Result<Self> {
        Ok(Self {
            jsonrpc: Version,
            id: id.into(),
            method: method.into(),
            params: Some(serde_json::to_value(params)?),
        })
    }

    /// Deserialize the params into a typed shape. Absent `params` is treated
    /// as `{}`, so all-optional param structs parse from a bare request.
    pub fn params_as<P: DeserializeOwned>(&self) -> serde_json::Result<P> {
        match &self.params {
            Some(v) => serde_json::from_value(v.clone()),
            None => serde_json::from_value(Value::Object(serde_json::Map::new())),
        }
    }
}

/// A JSON-RPC notification (no `id`, no response).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// Method name, e.g. `"notifications/tools/list_changed"`.
    pub method: String,
    /// Raw params.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Notification {
    /// Build a notification with no params.
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            jsonrpc: Version,
            method: method.into(),
            params: None,
        }
    }
}

/// A JSON-RPC response: exactly one of `result` / `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Always `"2.0"`.
    pub jsonrpc: Version,
    /// Echo of the request id ([`RequestId::Null`] when the request could
    /// not be parsed).
    pub id: RequestId,
    /// `result` or `error` (flattened onto the envelope).
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

/// The mutually-exclusive `result` / `error` half of a [`Response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsePayload {
    /// Successful response: a `result` value.
    Success {
        /// Raw result; per-method typed results live in sibling modules.
        result: Value,
    },
    /// Error response: an `error` object.
    Failure {
        /// The JSON-RPC error object.
        error: ErrorObject,
    },
}

impl Response {
    /// Build a success response.
    pub fn success(id: impl Into<RequestId>, result: Value) -> Self {
        Self {
            jsonrpc: Version,
            id: id.into(),
            payload: ResponsePayload::Success { result },
        }
    }

    /// Build an error response.
    pub fn failure(id: impl Into<RequestId>, error: ErrorObject) -> Self {
        Self {
            jsonrpc: Version,
            id: id.into(),
            payload: ResponsePayload::Failure { error },
        }
    }

    /// The `result` value, if this is a success response.
    pub fn result(&self) -> Option<&Value> {
        match &self.payload {
            ResponsePayload::Success { result } => Some(result),
            ResponsePayload::Failure { .. } => None,
        }
    }

    /// The `error` object, if this is an error response.
    pub fn error(&self) -> Option<&ErrorObject> {
        match &self.payload {
            ResponsePayload::Success { .. } => None,
            ResponsePayload::Failure { error } => Some(error),
        }
    }
}

/// Any incoming JSON-RPC message, classified by shape: `method` + `id` →
/// request; `method` without `id` → notification; otherwise a response.
///
/// Parsing is lenient about extraneous fields (a malformed hybrid like
/// `{"id":1,"method":"x","result":...}` classifies as a request and the
/// unknown `result` key is ignored), matching common JSON-RPC practice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    /// A request expecting a response.
    Request(Request),
    /// A fire-and-forget notification.
    Notification(Notification),
    /// A response to an earlier request.
    Response(Response),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error;

    #[test]
    fn request_round_trips_with_numeric_and_string_ids() {
        for (raw, want_id) in [
            (
                r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}}"#,
                RequestId::Number(7),
            ),
            (
                r#"{"jsonrpc":"2.0","id":"req-1","method":"tools/list","params":{}}"#,
                RequestId::String("req-1".into()),
            ),
        ] {
            let req: Request = serde_json::from_str(raw).unwrap();
            assert_eq!(req.id, want_id);
            assert_eq!(req.method, "tools/list");
            let back = serde_json::to_value(&req).unwrap();
            assert_eq!(back, serde_json::from_str::<Value>(raw).unwrap());
        }
    }

    #[test]
    fn wrong_jsonrpc_version_is_rejected() {
        let raw = r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#;
        assert!(serde_json::from_str::<Request>(raw).is_err());
    }

    #[test]
    fn fractional_id_is_rejected() {
        let raw = r#"{"jsonrpc":"2.0","id":1.5,"method":"x"}"#;
        assert!(serde_json::from_str::<Request>(raw).is_err());
    }

    #[test]
    fn params_as_treats_absent_params_as_empty_object() {
        #[derive(Deserialize, Default, PartialEq, Debug)]
        struct Empty {
            cursor: Option<String>,
        }
        let req: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"x"}"#).unwrap();
        assert_eq!(req.params_as::<Empty>().unwrap(), Empty::default());
    }

    #[test]
    fn response_success_xor_error() {
        let ok: Response =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":3,"result":{"answer":42}}"#).unwrap();
        assert_eq!(ok.result().unwrap()["answer"], 42);
        assert!(ok.error().is_none());

        let err: Response = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"Invalid params"}}"#,
        )
        .unwrap();
        assert!(err.result().is_none());
        assert_eq!(err.error().unwrap().code, error::INVALID_PARAMS);
    }

    #[test]
    fn parse_error_response_carries_null_id() {
        let resp = Response::failure(
            RequestId::Null,
            ErrorObject::new(error::PARSE_ERROR, "Parse error"),
        );
        let value = serde_json::to_value(&resp).unwrap();
        assert_eq!(value["id"], Value::Null);

        let back: Response = serde_json::from_value(value).unwrap();
        assert_eq!(back.id, RequestId::Null);
    }

    #[test]
    fn message_classifies_request_notification_response() {
        let req: Message =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        assert!(matches!(req, Message::Request(_)));

        let notif: Message = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#,
        )
        .unwrap();
        assert!(matches!(notif, Message::Notification(_)));

        let resp: Message =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).unwrap();
        assert!(matches!(resp, Message::Response(_)));
    }
}

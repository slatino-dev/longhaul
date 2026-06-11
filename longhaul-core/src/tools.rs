//! Tools: `tools/list` and `tools/call` (MCP 2026-07-28 RC).
//!
//! Tool `inputSchema` / `outputSchema` are full **JSON Schema 2020-12**
//! documents. This crate does *not* model the schema vocabulary — schemas
//! are carried as [`serde_json::Value`] and passed through byte-faithfully.
//! The only structural check offered is [`validate_schema`], which bounds
//! nesting depth so adversarial schemas cannot blow the stack of whatever
//! walks them next (validators, form renderers, …).
//!
//! `tools/list` results carry optional response-level cache metadata
//! (`ttlMs`, `cacheScope`) so HTTP-layer caches can reuse listings without
//! understanding MCP semantics.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::meta::Meta;
use crate::tag::string_tag;
use crate::tasks::{InputRequiredResult, InputRequiredTag, TaskHandleResult, TaskTag};

/// Method name: enumerate available tools.
pub const METHOD_LIST: &str = "tools/list";
/// Method name: invoke a tool.
pub const METHOD_CALL: &str = "tools/call";

/// Default nesting-depth bound for [`validate_schema`]. Deep enough for any
/// real-world 2020-12 schema, shallow enough that a depth-bomb is rejected
/// long before it threatens the stack.
pub const DEFAULT_MAX_SCHEMA_DEPTH: usize = 64;

/// A tool definition as returned by `tools/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Unique tool name (the `name` of a `tools/call`).
    pub name: String,
    /// Optional display title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What the tool does, for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema 2020-12 for the call arguments — passed through untyped.
    pub input_schema: Value,
    /// Optional JSON Schema 2020-12 for `structuredContent` results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Tool metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    /// Unmodelled fields (annotations, icons, …), preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Tool {
    /// Run [`validate_schema`] over `inputSchema` and (if present)
    /// `outputSchema`.
    pub fn validate_schemas(&self, max_depth: usize) -> Result<(), SchemaError> {
        validate_schema(&self.input_schema, max_depth)?;
        if let Some(out) = &self.output_schema {
            validate_schema(out, max_depth)?;
        }
        Ok(())
    }
}

/// Why a carried schema was rejected by [`validate_schema`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// JSON Schema 2020-12 documents are objects or booleans; anything else
    /// is not a schema.
    NotASchema {
        /// JSON type that was found instead (e.g. `"string"`).
        found: &'static str,
    },
    /// The document nests deeper than the permitted bound.
    TooDeep {
        /// The bound that was exceeded.
        max_depth: usize,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::NotASchema { found } => write!(
                f,
                "not a JSON Schema 2020-12 document: expected object or boolean, found {found}"
            ),
            SchemaError::TooDeep { max_depth } => {
                write!(f, "schema nests deeper than the bound of {max_depth}")
            }
        }
    }
}

impl std::error::Error for SchemaError {}

/// Structurally validate a carried JSON Schema 2020-12 document without
/// modelling it: the top level must be an object or boolean (per the
/// 2020-12 core spec), and nesting must stay within `max_depth` levels.
///
/// Depth is counted in container levels: a scalar or empty container at the
/// top level has depth 1. The walk checks the bound **before** descending,
/// so its own recursion is capped at `max_depth` frames — it is safe to run
/// against untrusted depth-bomb input.
pub fn validate_schema(schema: &Value, max_depth: usize) -> Result<(), SchemaError> {
    match schema {
        Value::Object(_) | Value::Bool(_) => check_depth(schema, 1, max_depth),
        Value::Null => Err(SchemaError::NotASchema { found: "null" }),
        Value::Number(_) => Err(SchemaError::NotASchema { found: "number" }),
        Value::String(_) => Err(SchemaError::NotASchema { found: "string" }),
        Value::Array(_) => Err(SchemaError::NotASchema { found: "array" }),
    }
}

fn check_depth(value: &Value, depth: usize, max_depth: usize) -> Result<(), SchemaError> {
    if depth > max_depth {
        return Err(SchemaError::TooDeep { max_depth });
    }
    match value {
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| check_depth(item, depth + 1, max_depth)),
        Value::Object(map) => map
            .values()
            .try_for_each(|item| check_depth(item, depth + 1, max_depth)),
        _ => Ok(()),
    }
}

/// Params for `tools/list`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListToolsParams {
    /// Opaque pagination cursor from a previous result's `nextCursor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Request metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Result of `tools/list`, with the RC's response-level cache metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    /// The tool definitions for this page.
    pub tools: Vec<Tool>,
    /// Cursor for the next page, when more tools exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// How long (milliseconds) this result may be cached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    /// Cache partition for this result (e.g. `"session"`, `"global"`),
    /// carried opaquely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_scope: Option<String>,
    /// Result metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Params for `tools/call` — both the initial call and the
/// input-required retry (see [`CallToolParams::retry`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolParams {
    /// Name of the tool to invoke.
    pub name: String,
    /// Arguments, validated by the tool's `inputSchema`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Map<String, Value>>,
    /// Retry only: answers to a prior [`InputRequiredResult`], keyed like
    /// its `inputRequests`.
    #[serde(rename = "inputResponses", skip_serializing_if = "Option::is_none")]
    pub input_responses: Option<Map<String, Value>>,
    /// Retry only: the `requestState` token from the
    /// [`InputRequiredResult`], echoed back **verbatim**.
    #[serde(rename = "requestState", skip_serializing_if = "Option::is_none")]
    pub request_state: Option<String>,
    /// Request metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl CallToolParams {
    /// An initial call to `name` with no arguments.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: None,
            input_responses: None,
            request_state: None,
            meta: None,
        }
    }

    /// The retry shape after an [`InputRequiredResult`]: supply
    /// `inputResponses` and echo the opaque `requestState` unmodified.
    pub fn retry(
        name: impl Into<String>,
        input_responses: Map<String, Value>,
        request_state: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            arguments: None,
            input_responses: Some(input_responses),
            request_state: Some(request_state.into()),
            meta: None,
        }
    }
}

string_tag! {
    /// The `"type": "text"` discriminator of [`TextContent`].
    pub struct TextTag = "text";
}

/// A text content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    /// Always `"text"`.
    #[serde(rename = "type")]
    pub kind: TextTag,
    /// The text payload.
    pub text: String,
    /// Content metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// One element of a [`CallToolResult`]'s `content`.
///
/// Only text blocks are typed in this crate version; image / audio /
/// embedded-resource blocks round-trip untouched through
/// [`ContentBlock::Other`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBlock {
    /// A `{"type": "text", ...}` block.
    Text(TextContent),
    /// Any other block kind, preserved verbatim.
    Other(Value),
}

impl ContentBlock {
    /// Build a text block.
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text(TextContent {
            kind: TextTag,
            text: text.into(),
            meta: None,
        })
    }

    /// The text payload, if this is a text block.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text(t) => Some(&t.text),
            ContentBlock::Other(_) => None,
        }
    }
}

/// The direct (non-task, non-input-required) result of `tools/call`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    /// Content blocks produced by the tool.
    pub content: Vec<ContentBlock>,
    /// Structured payload conforming to the tool's `outputSchema`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// True when the tool itself failed (distinct from protocol errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Result metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Everything a `tools/call` can resolve to under the 2026-07-28 RC,
/// classified by the `resultType` discriminator:
///
/// * no `resultType` → a direct [`CallToolResult`];
/// * `"task"` → a [`TaskHandleResult`] (Tasks extension);
/// * `"inputRequired"` → an [`InputRequiredResult`].
///
/// An unrecognized `resultType` is a deserialization **error**, not a
/// silent fallback — future RC result kinds must be handled deliberately.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolCallOutcome {
    /// Direct content result.
    Content(CallToolResult),
    /// The call was promoted to a task.
    Task(TaskHandleResult),
    /// The call paused for client input.
    InputRequired(InputRequiredResult),
}

impl<'de> Deserialize<'de> for ToolCallOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let value = Value::deserialize(deserializer)?;
        match value.get("resultType").and_then(Value::as_str) {
            None => serde_json::from_value(value)
                .map(ToolCallOutcome::Content)
                .map_err(D::Error::custom),
            Some(tag) if tag == TaskTag::VALUE => serde_json::from_value(value)
                .map(ToolCallOutcome::Task)
                .map_err(D::Error::custom),
            Some(tag) if tag == InputRequiredTag::VALUE => serde_json::from_value(value)
                .map(ToolCallOutcome::InputRequired)
                .map_err(D::Error::custom),
            Some(other) => Err(D::Error::custom(format!(
                "unknown tools/call resultType: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_value(levels: usize) -> Value {
        let mut v = Value::Bool(true);
        for _ in 0..levels.saturating_sub(1) {
            let mut map = Map::new();
            map.insert("items".to_owned(), v);
            v = Value::Object(map);
        }
        v
    }

    #[test]
    fn schema_depth_bound_is_exact() {
        // Exactly at the bound: fine.
        assert_eq!(validate_schema(&nested_value(8), 8), Ok(()));
        // One past the bound: rejected.
        assert_eq!(
            validate_schema(&nested_value(9), 8),
            Err(SchemaError::TooDeep { max_depth: 8 })
        );
    }

    #[test]
    fn depth_bomb_is_rejected_without_stack_growth() {
        // 200k levels — far beyond any stack budget if the walk descended
        // before checking the bound.
        let bomb = nested_value(200_000);
        assert_eq!(
            validate_schema(&bomb, DEFAULT_MAX_SCHEMA_DEPTH),
            Err(SchemaError::TooDeep {
                max_depth: DEFAULT_MAX_SCHEMA_DEPTH
            })
        );
        // Dropping a 200k-deep Value would itself recurse; defuse it level
        // by level instead.
        let mut current = bomb;
        while let Value::Object(ref mut map) = current {
            let inner = map.remove("items").unwrap_or(Value::Null);
            current = inner;
        }
    }

    #[test]
    fn boolean_schemas_are_valid_2020_12() {
        assert_eq!(validate_schema(&Value::Bool(true), 4), Ok(()));
        assert_eq!(validate_schema(&Value::Bool(false), 4), Ok(()));
    }

    #[test]
    fn non_object_non_bool_is_not_a_schema() {
        assert_eq!(
            validate_schema(&Value::String("x".into()), 4),
            Err(SchemaError::NotASchema { found: "string" })
        );
        assert_eq!(
            validate_schema(&serde_json::json!([1, 2]), 4),
            Err(SchemaError::NotASchema { found: "array" })
        );
        assert_eq!(
            validate_schema(&Value::Null, 4),
            Err(SchemaError::NotASchema { found: "null" })
        );
    }

    #[test]
    fn content_blocks_type_text_and_pass_through_the_rest() {
        let raw = r#"[
            {"type": "text", "text": "indexed 128 files"},
            {"type": "image", "data": "aGk=", "mimeType": "image/png"}
        ]"#;
        let blocks: Vec<ContentBlock> = serde_json::from_str(raw).unwrap();
        assert_eq!(blocks[0].as_text(), Some("indexed 128 files"));
        assert!(matches!(blocks[1], ContentBlock::Other(_)));

        let back = serde_json::to_value(&blocks).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn outcome_dispatches_on_result_type() {
        let content: ToolCallOutcome =
            serde_json::from_str(r#"{"content":[{"type":"text","text":"ok"}]}"#).unwrap();
        assert!(matches!(content, ToolCallOutcome::Content(_)));

        let task: ToolCallOutcome = serde_json::from_str(
            r#"{"resultType":"task","task":{"id":"task-1","status":"working"}}"#,
        )
        .unwrap();
        assert!(matches!(task, ToolCallOutcome::Task(_)));

        let input: ToolCallOutcome = serde_json::from_str(
            r#"{"resultType":"inputRequired","inputRequests":{},"requestState":"s1"}"#,
        )
        .unwrap();
        assert!(matches!(input, ToolCallOutcome::InputRequired(_)));
    }

    #[test]
    fn unknown_result_type_is_an_error_not_a_fallback() {
        let err = serde_json::from_str::<ToolCallOutcome>(r#"{"resultType":"stream"}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("stream"), "unhelpful error: {err}");
    }

    #[test]
    fn retry_params_echo_request_state() {
        let mut responses = Map::new();
        responses.insert("confirmOverwrite".into(), serde_json::json!(true));
        let retry = CallToolParams::retry("index_directory", responses, "opaque-token");
        let value = serde_json::to_value(&retry).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "name": "index_directory",
                "inputResponses": {"confirmOverwrite": true},
                "requestState": "opaque-token"
            })
        );
    }
}

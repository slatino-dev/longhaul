//! Round-trip tests for the key MCP 2026-07-28 RC message shapes.
//!
//! Every fixture below is **hand-written JSON** mirroring the RC examples —
//! not generated from the very structs under test — so these tests pin the
//! wire format, not the implementation's opinion of it. Each test asserts
//! (a) the typed parse sees the right fields and (b) re-serialization is
//! byte-equivalent at the `Value` level (no fields invented, renamed or
//! dropped).

use longhaul_core::discover::{DiscoverParams, DiscoverResult, METHOD_DISCOVER};
use longhaul_core::error::{self, ErrorObject};
use longhaul_core::http::PROTOCOL_VERSION;
use longhaul_core::jsonrpc::{Request, RequestId, Response};
use longhaul_core::meta::KEY_CLIENT_INFO;
use longhaul_core::tasks::{
    CancelTaskParams, GetTaskParams, Task, TaskStatus, UpdateTaskParams, METHOD_CANCEL, METHOD_GET,
    METHOD_UPDATE,
};
use longhaul_core::tools::{
    CallToolParams, ListToolsResult, ToolCallOutcome, DEFAULT_MAX_SCHEMA_DEPTH, METHOD_CALL,
};
use serde_json::Value;

/// Parse `raw` as `T`, then assert `T` re-serializes to exactly the same
/// JSON value.
fn round_trip<T>(raw: &str) -> T
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let typed: T = serde_json::from_str(raw).expect("fixture must parse");
    let back = serde_json::to_value(&typed).expect("must re-serialize");
    let original: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(back, original, "round trip must be lossless");
    typed
}

#[test]
fn tools_call_request_with_client_info_and_trace_context() {
    let raw = r#"{
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "index_directory",
            "arguments": {"path": "./docs", "recursive": true},
            "_meta": {
                "io.modelcontextprotocol/clientInfo": {
                    "name": "longhaul-conformance",
                    "version": "0.1.0"
                },
                "traceparent": "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
                "tracestate": "vendor=opaque",
                "baggage": "userId=alice,serverRegion=eu"
            }
        }
    }"#;

    let request: Request = round_trip(raw);
    assert_eq!(request.id, RequestId::Number(7));
    assert_eq!(request.method, METHOD_CALL);

    let params: CallToolParams = request.params_as().unwrap();
    assert_eq!(params.name, "index_directory");
    assert_eq!(params.arguments.as_ref().unwrap()["recursive"], true);

    let meta = params.meta.as_ref().unwrap();
    let client = meta.client_info.as_ref().unwrap();
    assert_eq!(client.name, "longhaul-conformance");
    assert_eq!(client.version, "0.1.0");
    assert_eq!(
        meta.traceparent.as_deref(),
        Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
    );
    assert_eq!(meta.tracestate.as_deref(), Some("vendor=opaque"));
    assert_eq!(
        meta.baggage.as_deref(),
        Some("userId=alice,serverRegion=eu")
    );

    // The reserved key really is the reverse-DNS one, on the wire.
    let original: Value = serde_json::from_str(raw).unwrap();
    assert!(original["params"]["_meta"]
        .as_object()
        .unwrap()
        .contains_key(KEY_CLIENT_INFO));

    // Typed params re-serialize to exactly the fixture's params object.
    assert_eq!(serde_json::to_value(&params).unwrap(), original["params"]);
}

#[test]
fn tools_list_result_with_cache_metadata_and_2020_12_schemas() {
    let raw = r#"{
        "tools": [
            {
                "name": "echo",
                "title": "Echo",
                "description": "Echo a message back.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "message": {"type": "string", "minLength": 1}
                    },
                    "required": ["message"],
                    "additionalProperties": false
                },
                "outputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {"echo": {"type": "string"}},
                    "required": ["echo"]
                }
            }
        ],
        "nextCursor": "page-2",
        "ttlMs": 60000,
        "cacheScope": "session"
    }"#;

    let result: ListToolsResult = round_trip(raw);
    assert_eq!(result.ttl_ms, Some(60_000u64));
    assert_eq!(result.cache_scope.as_deref(), Some("session"));
    assert_eq!(result.next_cursor.as_deref(), Some("page-2"));

    let tool = &result.tools[0];
    assert_eq!(tool.name, "echo");
    // Schemas pass through untyped...
    assert_eq!(
        tool.input_schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(tool.input_schema["properties"]["message"]["minLength"], 1);
    // ...and pass the depth-bounded structural check.
    tool.validate_schemas(DEFAULT_MAX_SCHEMA_DEPTH).unwrap();
}

#[test]
fn server_discover_request_and_result() {
    let request_raw = r#"{
        "jsonrpc": "2.0",
        "id": "disc-1",
        "method": "server/discover",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/clientInfo": {"name": "inspector", "version": "2.3.0"}
            }
        }
    }"#;
    let request: Request = round_trip(request_raw);
    assert_eq!(request.method, METHOD_DISCOVER);
    let params: DiscoverParams = request.params_as().unwrap();
    assert_eq!(params.meta.unwrap().client_info.unwrap().name, "inspector");

    let result_raw = r#"{
        "protocolVersion": "2026-07-28",
        "serverInfo": {"name": "longhaul-indexer", "version": "0.1.0"},
        "capabilities": {
            "tools": {"listChanged": true},
            "tasks": {}
        },
        "instructions": "Index and search local directories."
    }"#;
    let result: DiscoverResult = round_trip(result_raw);
    assert_eq!(result.protocol_version, PROTOCOL_VERSION);
    assert!(result.capabilities.tasks.is_some());
    assert_eq!(result.capabilities.tools.unwrap().list_changed, Some(true));
}

#[test]
fn tools_call_returning_a_task_handle() {
    let raw = r#"{
        "resultType": "task",
        "task": {"id": "task-42", "status": "working"}
    }"#;
    let outcome: ToolCallOutcome = round_trip(raw);
    match outcome {
        ToolCallOutcome::Task(handle) => {
            assert_eq!(handle.task.id, "task-42");
            assert_eq!(handle.task.status, TaskStatus::Working);
        }
        other => panic!("expected a task handle, got {other:?}"),
    }
}

#[test]
fn input_required_outcome_and_the_retry_call() {
    let outcome_raw = r#"{
        "resultType": "inputRequired",
        "inputRequests": {
            "confirmOverwrite": {
                "type": "object",
                "properties": {"confirm": {"type": "boolean"}},
                "required": ["confirm"]
            }
        },
        "requestState": "eyJzdGVwIjoiY29uZmlybSJ9"
    }"#;
    let outcome: ToolCallOutcome = round_trip(outcome_raw);
    let input = match outcome {
        ToolCallOutcome::InputRequired(input) => input,
        other => panic!("expected inputRequired, got {other:?}"),
    };
    assert!(input.input_requests.contains_key("confirmOverwrite"));
    assert_eq!(input.request_state, "eyJzdGVwIjoiY29uZmlybSJ9");

    // The retry: inputResponses keyed like inputRequests, requestState
    // echoed verbatim.
    let retry_raw = r#"{
        "name": "index_directory",
        "inputResponses": {"confirmOverwrite": {"confirm": true}},
        "requestState": "eyJzdGVwIjoiY29uZmlybSJ9"
    }"#;
    let retry: CallToolParams = round_trip(retry_raw);
    assert_eq!(
        retry.request_state.as_deref(),
        Some(input.request_state.as_str())
    );
    assert_eq!(
        retry.input_responses.unwrap()["confirmOverwrite"]["confirm"],
        true
    );
}

#[test]
fn tasks_get_update_cancel_round_trip() {
    // tasks/get
    let get: GetTaskParams = round_trip(r#"{"taskId": "task-42"}"#);
    assert_eq!(get.task_id, "task-42");
    let fetched: Task = round_trip(r#"{"id": "task-42", "status": "inputRequired"}"#);
    assert_eq!(fetched.status, TaskStatus::InputRequired);

    // tasks/update — and the requested transition is legal per the machine.
    let update: UpdateTaskParams = round_trip(r#"{"taskId": "task-42", "status": "working"}"#);
    assert!(fetched.status.can_transition_to(update.status));

    // tasks/cancel
    let cancel: CancelTaskParams = round_trip(r#"{"taskId": "task-42"}"#);
    assert_eq!(cancel.task_id, "task-42");
    let cancelled: Task = round_trip(r#"{"id": "task-42", "status": "cancelled"}"#);
    assert!(cancelled.status.is_terminal());

    // Method-name constants match the RC strings (tasks/list does not exist).
    assert_eq!(METHOD_GET, "tasks/get");
    assert_eq!(METHOD_UPDATE, "tasks/update");
    assert_eq!(METHOD_CANCEL, "tasks/cancel");
}

#[test]
fn invalid_params_error_response_uses_32602() {
    let raw = r#"{
        "jsonrpc": "2.0",
        "id": 3,
        "error": {
            "code": -32602,
            "message": "Invalid params: missing required field 'name'"
        }
    }"#;
    let response: Response = round_trip(raw);
    let err = response.error().unwrap();
    assert_eq!(err.code, error::INVALID_PARAMS);

    // A pre-RC peer sending the retired code normalizes to the RC code.
    let legacy = ErrorObject::new(error::PRE_RC_INVALID_PARAMS, "Invalid params");
    assert_eq!(error::normalize_code(legacy.code), error::INVALID_PARAMS);
}

#[test]
fn direct_content_outcome_round_trips() {
    let raw = r#"{
        "content": [{"type": "text", "text": "indexed 128 files"}],
        "isError": false
    }"#;
    let outcome: ToolCallOutcome = round_trip(raw);
    match outcome {
        ToolCallOutcome::Content(result) => {
            assert_eq!(result.content[0].as_text(), Some("indexed 128 files"));
            assert_eq!(result.is_error, Some(false));
        }
        other => panic!("expected content, got {other:?}"),
    }
}

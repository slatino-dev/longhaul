//! In-process conformance tests.
//!
//! Starts a real longhaul-server on a random loopback port, pre-seeds a task,
//! then runs each conformance scenario (schema validation + behavioural checks)
//! against it using the same logic as the CLI runner.

use std::sync::Arc;

use longhaul_core::{
    discover::{
        DiscoverResult, Implementation, ServerCapabilities, TasksCapability, ToolsCapability,
    },
    http::{HEADER_METHOD, HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION},
    jsonrpc::Response,
    tasks::{Task, TaskStatus, UpdateTaskParams},
};
use longhaul_server::{registry::Registry, MemoryStore, ServerState, TaskStore};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// In-process server fixture
// ---------------------------------------------------------------------------

/// Start an in-process server on a random port and return its base URL + the
/// underlying state so tests can pre-seed tasks.
async fn start_server() -> (String, Arc<ServerState>) {
    let store = Arc::new(MemoryStore::new());
    let state = Arc::new(ServerState {
        discover_result: DiscoverResult {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            server_info: Implementation::new("conformance-server", "0.1.0"),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: None }),
                tasks: Some(TasksCapability::default()),
                extra: Default::default(),
            },
            instructions: None,
            meta: None,
        },
        registry: Registry::default(),
        store: store as Arc<dyn TaskStore>,
    });

    // Bind on port 0 → OS assigns a free port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://127.0.0.1:{}", addr.port());

    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        let app = longhaul_server::router::build(state_clone);
        axum::serve(listener, app).await.unwrap();
    });

    // Yield so the server task has a chance to start.
    tokio::task::yield_now().await;

    (url, state)
}

// ---------------------------------------------------------------------------
// HTTP helper (mirrors what the CLI runner does)
// ---------------------------------------------------------------------------

async fn post_mcp(base_url: &str, method: &str, params: Value) -> Response {
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp = client
        .post(format!("{base_url}/mcp"))
        .header("content-type", "application/json")
        .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION)
        .header(HEADER_METHOD, method)
        .json(&body)
        .send()
        .await
        .expect("HTTP request failed");
    let bytes = resp.bytes().await.unwrap();
    serde_json::from_slice::<Response>(&bytes).expect("invalid JSON-RPC response")
}

fn result_of(resp: &Response) -> &Value {
    resp.result().unwrap_or_else(|| {
        panic!(
            "expected success, got error: {:?}",
            resp.error().map(|e| &e.message)
        )
    })
}

// ---------------------------------------------------------------------------
// Schema fixture validator (inline; same fixture set as CLI runner)
// ---------------------------------------------------------------------------

fn schema_validate(schema_src: &str, value: &Value) -> Result<(), String> {
    let schema: Value = serde_json::from_str(schema_src).expect("fixture schema is invalid JSON");
    let validator = jsonschema::validator_for(&schema).expect("cannot compile fixture schema");
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

const RESPONSE_SCHEMA: &str = include_str!("../fixtures/jsonrpc_response.schema.json");
const DISCOVER_SCHEMA: &str = include_str!("../fixtures/discover_result.schema.json");
const TOOLS_LIST_SCHEMA: &str = include_str!("../fixtures/tools_list_result.schema.json");
const TASK_SCHEMA: &str = include_str!("../fixtures/task.schema.json");

// ---------------------------------------------------------------------------
// Discovery suite
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discovery_response_validates_schema() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "server/discover", json!({})).await;
    let raw = serde_json::to_value(&resp).unwrap();
    schema_validate(RESPONSE_SCHEMA, &raw).unwrap();
}

#[tokio::test]
async fn discovery_result_validates_schema() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "server/discover", json!({})).await;
    let r = result_of(&resp);
    schema_validate(DISCOVER_SCHEMA, r).unwrap();
}

#[tokio::test]
async fn discovery_protocol_version_is_pinned() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "server/discover", json!({})).await;
    let r = result_of(&resp);
    assert_eq!(
        r["protocolVersion"], PROTOCOL_VERSION,
        "protocolVersion wire value must match the RC constant"
    );
}

#[tokio::test]
async fn discovery_server_info_fields_present() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "server/discover", json!({})).await;
    let r = result_of(&resp);
    assert!(
        r["serverInfo"]["name"].is_string(),
        "serverInfo.name missing"
    );
    assert!(
        r["serverInfo"]["version"].is_string(),
        "serverInfo.version missing"
    );
}

// ---------------------------------------------------------------------------
// Tools suite
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_list_result_validates_schema() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "tools/list", json!({})).await;
    let r = result_of(&resp);
    schema_validate(TOOLS_LIST_SCHEMA, r).unwrap();
}

#[tokio::test]
async fn tools_list_returns_array() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "tools/list", json!({})).await;
    let r = result_of(&resp);
    assert!(r["tools"].is_array(), "tools must be an array");
}

#[tokio::test]
async fn tools_list_cache_metadata_types() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "tools/list", json!({})).await;
    let r = result_of(&resp);
    if let Some(ttl) = r.get("ttlMs") {
        assert!(ttl.is_number(), "ttlMs must be a number, got {ttl}");
    }
    if let Some(scope) = r.get("cacheScope") {
        assert!(
            scope.is_string(),
            "cacheScope must be a string, got {scope}"
        );
    }
}

// ---------------------------------------------------------------------------
// Tasks suite
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tasks_get_existing_validates_schema() {
    let (url, state) = start_server().await;

    // Pre-seed a task.
    state.store.insert(Task::new("t-schema")).unwrap();

    let resp = post_mcp(&url, "tasks/get", json!({"taskId": "t-schema"})).await;
    let r = result_of(&resp);
    schema_validate(TASK_SCHEMA, r).unwrap();
}

#[tokio::test]
async fn tasks_full_lifecycle_via_http() {
    let (url, state) = start_server().await;
    state.store.insert(Task::new("t-lifecycle")).unwrap();

    // get → working
    let resp = post_mcp(&url, "tasks/get", json!({"taskId": "t-lifecycle"})).await;
    assert_eq!(result_of(&resp)["status"], "working");

    // update → inputRequired
    let resp = post_mcp(
        &url,
        "tasks/update",
        json!({"taskId": "t-lifecycle", "status": "inputRequired"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "inputRequired");
    schema_validate(TASK_SCHEMA, result_of(&resp)).unwrap();

    // update → working (resume)
    let resp = post_mcp(
        &url,
        "tasks/update",
        json!({"taskId": "t-lifecycle", "status": "working"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "working");

    // update → completed
    let resp = post_mcp(
        &url,
        "tasks/update",
        json!({"taskId": "t-lifecycle", "status": "completed"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "completed");

    // get → completed
    let resp = post_mcp(&url, "tasks/get", json!({"taskId": "t-lifecycle"})).await;
    assert_eq!(result_of(&resp)["status"], "completed");
}

#[tokio::test]
async fn tasks_cancel_live_task() {
    let (url, state) = start_server().await;
    state.store.insert(Task::new("t-cancel")).unwrap();

    let resp = post_mcp(&url, "tasks/cancel", json!({"taskId": "t-cancel"})).await;
    assert_eq!(result_of(&resp)["status"], "cancelled");

    // Idempotent cancel.
    let resp = post_mcp(&url, "tasks/cancel", json!({"taskId": "t-cancel"})).await;
    assert_eq!(result_of(&resp)["status"], "cancelled");
}

#[tokio::test]
async fn tasks_get_missing_returns_32602() {
    let (url, _state) = start_server().await;
    let resp = post_mcp(&url, "tasks/get", json!({"taskId": "no-such-task"})).await;
    let err = resp.error().expect("expected error response");
    assert_eq!(
        err.code,
        longhaul_core::error::INVALID_PARAMS,
        "missing task must return -32602"
    );
}

#[tokio::test]
async fn tasks_illegal_transition_returns_32602() {
    let (url, state) = start_server().await;
    state.store.insert(Task::new("t-illegal")).unwrap();
    // working → completed (legal)
    state
        .store
        .update(UpdateTaskParams {
            task_id: "t-illegal".to_owned(),
            status: TaskStatus::Completed,
            meta: None,
        })
        .unwrap();
    // completed → working (illegal)
    let resp = post_mcp(
        &url,
        "tasks/update",
        json!({"taskId": "t-illegal", "status": "working"}),
    )
    .await;
    let err = resp.error().expect("expected error for illegal transition");
    assert_eq!(
        err.code,
        longhaul_core::error::INVALID_PARAMS,
        "illegal transition must return -32602"
    );
}

#[tokio::test]
async fn tasks_list_removed_returns_32601() {
    // tasks/list was removed in the 2026-07-28 RC. Calling it must return
    // -32601 Method Not Found.
    let (url, _state) = start_server().await;

    // tasks/list won't match the Mcp-Method middleware (it passes it through
    // to dispatch, which returns -32601). We send without the Mcp-Method
    // header to avoid the early rejection path.
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tasks/list",
        "params": {},
    });
    let raw_resp = client
        .post(format!("{url}/mcp"))
        .header("content-type", "application/json")
        .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION)
        .json(&body)
        .send()
        .await
        .unwrap();
    let bytes = raw_resp.bytes().await.unwrap();
    let resp: Response = serde_json::from_slice(&bytes).unwrap();
    let err = resp.error().expect("tasks/list must return an error");
    assert_eq!(
        err.code,
        longhaul_core::error::METHOD_NOT_FOUND,
        "tasks/list must return -32601"
    );
}

// ---------------------------------------------------------------------------
// Wire-level schema fixture tests (offline — no server needed)
// ---------------------------------------------------------------------------

#[test]
fn fixture_schema_task_validates_known_good() {
    let task = json!({"id": "task-1", "status": "working"});
    schema_validate(TASK_SCHEMA, &task).unwrap();
}

#[test]
fn fixture_schema_task_rejects_unknown_status() {
    let bad = json!({"id": "task-1", "status": "pending"});
    assert!(
        schema_validate(TASK_SCHEMA, &bad).is_err(),
        "unknown status must fail schema validation"
    );
}

#[test]
fn fixture_schema_task_rejects_missing_id() {
    let bad = json!({"status": "working"});
    assert!(
        schema_validate(TASK_SCHEMA, &bad).is_err(),
        "missing id must fail schema validation"
    );
}

#[test]
fn fixture_schema_jsonrpc_response_validates_success() {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {"tools": []}
    });
    schema_validate(RESPONSE_SCHEMA, &resp).unwrap();
}

#[test]
fn fixture_schema_jsonrpc_response_validates_error() {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": {"code": -32700, "message": "Parse error"}
    });
    schema_validate(RESPONSE_SCHEMA, &resp).unwrap();
}

#[test]
fn fixture_schema_discover_result_validates() {
    let result = json!({
        "protocolVersion": "2026-07-28",
        "serverInfo": {"name": "test", "version": "0.1.0"},
        "capabilities": {}
    });
    schema_validate(DISCOVER_SCHEMA, &result).unwrap();
}

#[test]
fn fixture_schema_discover_result_rejects_wrong_version() {
    let result = json!({
        "protocolVersion": "2025-01-01",
        "serverInfo": {"name": "test", "version": "0.1.0"},
        "capabilities": {}
    });
    assert!(schema_validate(DISCOVER_SCHEMA, &result).is_err());
}

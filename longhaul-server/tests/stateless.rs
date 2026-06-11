//! Statelessness integration test.
//!
//! Verifies the key design guarantee: a single client's task lifecycle can be
//! round-robined across **two independent server instances** that share only a
//! [`SqliteStore`] (pointing at the same temp database file). Every request is
//! routed to an alternating instance; the task must still complete correctly.
//!
//! This test also covers:
//! * `tasks/cancel` race: cancel arrives while the task is in a non-terminal
//!   state, should succeed idempotently even when retried.
//! * Schema: the JSON-RPC envelope of every response is checked for `id` and
//!   `result` presence.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{body::Body, extract::Request, http::StatusCode};
use longhaul_core::{
    discover::{
        DiscoverResult, Implementation, ServerCapabilities, TasksCapability, ToolsCapability,
    },
    http::{HEADER_METHOD, HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION},
    jsonrpc::Response,
    tasks::{TaskStatus, UpdateTaskParams},
    tools::ListToolsResult,
};
use longhaul_server::{
    registry::Registry, router, store::SqliteStore, MemoryStore, ServerState, TaskStore,
};
use serde_json::{json, Value};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn discover_result() -> DiscoverResult {
    DiscoverResult {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        server_info: Implementation::new("test-server", "0.1.0"),
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability { list_changed: None }),
            tasks: Some(TasksCapability::default()),
            extra: Default::default(),
        },
        instructions: None,
        meta: None,
    }
}

/// Send a JSON-RPC POST to a router and return the parsed response.
async fn post_mcp<R: tower::Service<Request<Body>, Response = axum::response::Response> + Send>(
    app: R,
    method: &str,
    params: Value,
) -> Response
where
    R::Future: Send,
    R::Error: std::fmt::Debug,
{
    let body_json = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION)
        .header(HEADER_METHOD, method)
        .body(Body::from(serde_json::to_vec(&body_json).unwrap()))
        .unwrap();

    let raw = app.oneshot(req).await.unwrap();
    assert_eq!(raw.status(), StatusCode::OK, "HTTP status must be 200");

    let bytes = axum::body::to_bytes(raw.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice::<Response>(&bytes).expect("response must be valid JSON-RPC")
}

fn result_of(resp: &Response) -> &Value {
    resp.result().expect("expected success response")
}

// ---------------------------------------------------------------------------
// Single-server smoke test (MemoryStore)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn smoke_tools_list() {
    let state = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: Arc::new(MemoryStore::new()),
    });
    let app = router::build(Arc::clone(&state));

    let resp = post_mcp(app, "tools/list", json!({})).await;
    let r = result_of(&resp);
    let list: ListToolsResult = serde_json::from_value(r.clone()).unwrap();
    assert!(list.tools.is_empty());
    assert_eq!(list.ttl_ms, Some(60_000));
}

#[tokio::test]
async fn smoke_server_discover() {
    let state = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: Arc::new(MemoryStore::new()),
    });
    let app = router::build(Arc::clone(&state));

    let resp = post_mcp(app, "server/discover", json!({})).await;
    let r = result_of(&resp);
    assert_eq!(r["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(r["serverInfo"]["name"], "test-server");
}

#[tokio::test]
async fn method_not_found_returns_error() {
    let state = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: Arc::new(MemoryStore::new()),
    });
    // Use the router directly. The Mcp-Method header matches the body method
    // so the middleware passes it through; the dispatch layer returns -32601.
    let body_json = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "no/such/method",
        "params": {},
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION)
        .header(HEADER_METHOD, "no/such/method")
        .body(Body::from(serde_json::to_vec(&body_json).unwrap()))
        .unwrap();
    let raw_resp = router::build(state).oneshot(req).await.unwrap();
    assert_eq!(raw_resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(raw_resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let resp: Response = serde_json::from_slice(&bytes).unwrap();
    let err = resp.error().expect("expected error response");
    assert_eq!(err.code, longhaul_core::error::METHOD_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Task lifecycle (MemoryStore)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_lifecycle_memory_store() {
    let store = Arc::new(MemoryStore::new());
    // Pre-insert a task in Working state.
    store
        .insert(longhaul_core::tasks::Task::new("task-1"))
        .unwrap();

    let state = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: store as Arc<dyn TaskStore>,
    });
    let app = router::build(Arc::clone(&state));

    // tasks/get
    let resp = post_mcp(app, "tasks/get", json!({"taskId": "task-1"})).await;
    let r = result_of(&resp);
    assert_eq!(r["status"], "working");

    // tasks/update: working → inputRequired
    let app2 = router::build(Arc::clone(&state));
    let resp = post_mcp(
        app2,
        "tasks/update",
        json!({"taskId": "task-1", "status": "inputRequired"}),
    )
    .await;
    let r = result_of(&resp);
    assert_eq!(r["status"], "inputRequired");

    // tasks/update: inputRequired → working
    let app3 = router::build(Arc::clone(&state));
    let resp = post_mcp(
        app3,
        "tasks/update",
        json!({"taskId": "task-1", "status": "working"}),
    )
    .await;
    let r = result_of(&resp);
    assert_eq!(r["status"], "working");

    // tasks/cancel
    let app4 = router::build(Arc::clone(&state));
    let resp = post_mcp(app4, "tasks/cancel", json!({"taskId": "task-1"})).await;
    let r = result_of(&resp);
    assert_eq!(r["status"], "cancelled");

    // tasks/cancel again — idempotent
    let app5 = router::build(Arc::clone(&state));
    let resp = post_mcp(app5, "tasks/cancel", json!({"taskId": "task-1"})).await;
    let r = result_of(&resp);
    assert_eq!(r["status"], "cancelled");
}

// ---------------------------------------------------------------------------
// KEY TEST: Statelessness across two server instances sharing SqliteStore
// ---------------------------------------------------------------------------
//
// Architecture:
//
//   client ──▶ Dispatcher (round-robin)
//                 ├─ instance A (SqliteStore → shared.db)
//                 └─ instance B (SqliteStore → shared.db)
//
// The dispatcher alternates requests between A and B. The task lifecycle
// (insert → get → update → get → cancel → get) must succeed even though
// no single instance serves two consecutive requests.

#[tokio::test]
async fn stateless_task_lifecycle_across_two_sqlite_instances() {
    // Create a temp file for the shared database.
    let dir = tempdir();
    let db_path = format!("{}/shared.db", dir);

    // Two independent SqliteStore instances on the same file.
    let store_a = Arc::new(SqliteStore::open(&db_path).unwrap());
    let store_b = Arc::new(SqliteStore::open(&db_path).unwrap());

    let state_a = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: store_a as Arc<dyn TaskStore>,
    });
    let state_b = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: store_b as Arc<dyn TaskStore>,
    });

    // Pre-insert the task through instance A.
    {
        let task = longhaul_core::tasks::Task::new("shared-task");
        state_a.store.insert(task).unwrap();
    }

    // Round-robin dispatcher: alternates between building routers for A and B.
    let counter = Arc::new(AtomicUsize::new(0));

    macro_rules! next_router {
        () => {{
            let idx = counter.fetch_add(1, Ordering::SeqCst);
            if idx % 2 == 0 {
                router::build(Arc::clone(&state_a))
            } else {
                router::build(Arc::clone(&state_b))
            }
        }};
    }

    // Step 1 (→ A): get → working
    let resp = post_mcp(
        next_router!(),
        "tasks/get",
        json!({"taskId": "shared-task"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "working", "step 1");

    // Step 2 (→ B): update → inputRequired
    let resp = post_mcp(
        next_router!(),
        "tasks/update",
        json!({"taskId": "shared-task", "status": "inputRequired"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "inputRequired", "step 2");

    // Step 3 (→ A): get → inputRequired (persisted by B)
    let resp = post_mcp(
        next_router!(),
        "tasks/get",
        json!({"taskId": "shared-task"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "inputRequired", "step 3");

    // Step 4 (→ B): update → working (resume)
    let resp = post_mcp(
        next_router!(),
        "tasks/update",
        json!({"taskId": "shared-task", "status": "working"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "working", "step 4");

    // Step 5 (→ A): update → completed
    let resp = post_mcp(
        next_router!(),
        "tasks/update",
        json!({"taskId": "shared-task", "status": "completed"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "completed", "step 5");

    // Step 6 (→ B): get → completed (written by A)
    let resp = post_mcp(
        next_router!(),
        "tasks/get",
        json!({"taskId": "shared-task"}),
    )
    .await;
    assert_eq!(result_of(&resp)["status"], "completed", "step 6");

    // Step 7 (→ A): cancel on completed — idempotent
    let resp = post_mcp(
        next_router!(),
        "tasks/cancel",
        json!({"taskId": "shared-task"}),
    )
    .await;
    assert_eq!(
        result_of(&resp)["status"],
        "completed",
        "step 7 (idempotent cancel)"
    );

    // Clean up (Windows: drop stores before removing the file).
    drop(state_a);
    drop(state_b);
    // Best-effort cleanup; ignore errors on Windows file locking.
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{db_path}-wal"));
    let _ = std::fs::remove_file(format!("{db_path}-shm"));
}

// ---------------------------------------------------------------------------
// Cancellation race test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_race_idempotent() {
    let store = Arc::new(MemoryStore::new());
    store
        .insert(longhaul_core::tasks::Task::new("race-task"))
        .unwrap();

    let state = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: store as Arc<dyn TaskStore>,
    });

    // Cancel twice — both should report cancelled (not an error).
    for i in 0..2 {
        let app = router::build(Arc::clone(&state));
        let resp = post_mcp(app, "tasks/cancel", json!({"taskId": "race-task"})).await;
        let r = result_of(&resp);
        assert_eq!(r["status"], "cancelled", "cancel attempt {i}");
    }
}

// ---------------------------------------------------------------------------
// Invalid transition returns -32602
// ---------------------------------------------------------------------------

#[tokio::test]
async fn illegal_transition_returns_invalid_params() {
    let store = Arc::new(MemoryStore::new());
    store.insert(longhaul_core::tasks::Task::new("t")).unwrap();
    // working → completed
    store
        .update(UpdateTaskParams {
            task_id: "t".to_owned(),
            status: TaskStatus::Completed,
            meta: None,
        })
        .unwrap();

    let state = Arc::new(ServerState {
        discover_result: discover_result(),
        registry: Registry::default(),
        store: store as Arc<dyn TaskStore>,
    });
    let app = router::build(Arc::clone(&state));

    // completed → working: illegal
    let resp = post_mcp(
        app,
        "tasks/update",
        json!({"taskId": "t", "status": "working"}),
    )
    .await;
    let err = resp.error().expect("expected error for illegal transition");
    assert_eq!(err.code, longhaul_core::error::INVALID_PARAMS);
}

// ---------------------------------------------------------------------------
// Temp directory helper (no tempfile dep — just use env::temp_dir)
// ---------------------------------------------------------------------------

fn tempdir() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir().join(format!("longhaul-test-{ts}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

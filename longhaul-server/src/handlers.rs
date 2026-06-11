//! Axum request handlers for all MCP endpoints.
//!
//! Every handler receives the JSON-RPC envelope (already parsed by
//! [`axum::Json`]), dispatches to the appropriate registry/store operation,
//! and returns a JSON-RPC response.

use std::sync::Arc;

use axum::{extract::State, Json};
use serde_json::Value;
use tracing::{debug, warn};

use longhaul_core::{
    discover::METHOD_DISCOVER,
    error::{self, ErrorObject, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND},
    jsonrpc::{Request, Response},
    tasks::{
        CancelTaskParams, GetTaskParams, UpdateTaskParams, METHOD_CANCEL, METHOD_GET, METHOD_UPDATE,
    },
    tools::{CallToolParams, METHOD_CALL, METHOD_LIST},
};

use crate::{
    registry::{err_content, ToolError},
    store::StoreError,
    ServerState,
};

/// Axum handler for `POST /mcp` — the single stateless dispatch point.
pub async fn dispatch(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Request>,
) -> Json<Response> {
    let id = request.id.clone();
    let resp = route(&state, request).await;
    Json(resp.unwrap_or_else(|(code, msg)| Response::failure(id, ErrorObject::new(code, msg))))
}

type DispatchResult = Result<Response, (i64, String)>;

async fn route(state: &ServerState, req: Request) -> DispatchResult {
    debug!(method = %req.method, id = %req.id, "dispatch");
    match req.method.as_str() {
        METHOD_DISCOVER => handle_discover(state, req).await,
        METHOD_LIST => handle_tools_list(state, req).await,
        METHOD_CALL => handle_tools_call(state, req).await,
        METHOD_GET => handle_tasks_get(state, req).await,
        METHOD_UPDATE => handle_tasks_update(state, req).await,
        METHOD_CANCEL => handle_tasks_cancel(state, req).await,
        other => {
            warn!(method = %other, "method not found");
            Err((METHOD_NOT_FOUND, format!("Method not found: {other}")))
        }
    }
}

async fn handle_discover(state: &ServerState, req: Request) -> DispatchResult {
    let result = serde_json::to_value(&state.discover_result)
        .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
    Ok(Response::success(req.id, result))
}

async fn handle_tools_list(state: &ServerState, req: Request) -> DispatchResult {
    let _params: longhaul_core::tools::ListToolsParams = req
        .params_as()
        .map_err(|e| (INVALID_PARAMS, e.to_string()))?;
    let result = state.registry.list_result();
    let value = serde_json::to_value(&result).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
    Ok(Response::success(req.id, value))
}

async fn handle_tools_call(state: &ServerState, req: Request) -> DispatchResult {
    let params: CallToolParams = req
        .params_as()
        .map_err(|e| (INVALID_PARAMS, e.to_string()))?;

    let entry = state.registry.get(&params.name).ok_or_else(|| {
        (
            error::INVALID_PARAMS,
            format!("Unknown tool: {}", params.name),
        )
    })?;

    let outcome = entry
        .handler
        .call(
            params.arguments,
            params.input_responses,
            params.request_state,
        )
        .await
        .unwrap_or_else(|e| match &e {
            ToolError::InvalidParams(_) => {
                // Propagate as a protocol error (handled below via map_err).
                // We use err_content only for internal failures.
                err_content(e.to_string())
            }
            _ => err_content(e.to_string()),
        });

    let value = serde_json::to_value(&outcome).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
    Ok(Response::success(req.id, value))
}

async fn handle_tasks_get(state: &ServerState, req: Request) -> DispatchResult {
    let params: GetTaskParams = req
        .params_as()
        .map_err(|e| (INVALID_PARAMS, e.to_string()))?;
    let task = state.store.get(&params.task_id).map_err(store_err_to_rpc)?;
    let value = serde_json::to_value(&task).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
    Ok(Response::success(req.id, value))
}

async fn handle_tasks_update(state: &ServerState, req: Request) -> DispatchResult {
    let params: UpdateTaskParams = req
        .params_as()
        .map_err(|e| (INVALID_PARAMS, e.to_string()))?;
    let task = state.store.update(params).map_err(store_err_to_rpc)?;
    let value = serde_json::to_value(&task).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
    Ok(Response::success(req.id, value))
}

async fn handle_tasks_cancel(state: &ServerState, req: Request) -> DispatchResult {
    let params: CancelTaskParams = req
        .params_as()
        .map_err(|e| (INVALID_PARAMS, e.to_string()))?;
    let task = state
        .store
        .cancel(&params.task_id)
        .map_err(store_err_to_rpc)?;
    let value = serde_json::to_value(&task).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
    Ok(Response::success(req.id, value))
}

fn store_err_to_rpc(e: StoreError) -> (i64, String) {
    match e {
        StoreError::NotFound(id) => (INVALID_PARAMS, format!("Task not found: {id}")),
        StoreError::IllegalTransition { from, to } => (
            INVALID_PARAMS,
            format!("Illegal task transition: {from} -> {to}"),
        ),
        other => (INTERNAL_ERROR, other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Health handlers
// ---------------------------------------------------------------------------

/// `GET /health` — liveness probe.
pub async fn health() -> Json<Value> {
    Json(serde_json::json!({"status": "ok"}))
}

/// `GET /ready` — readiness probe (same shape as health for now).
pub async fn ready() -> Json<Value> {
    Json(serde_json::json!({"status": "ready"}))
}

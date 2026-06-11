//! Tool registry — the table of [`ToolEntry`]s the server exposes.
//!
//! The registry is **append-only at construction time** and then becomes an
//! immutable, shareable snapshot. Tool handlers are `Arc<dyn ToolHandler>` so
//! they can be shared freely across tokio tasks.

use std::{collections::HashMap, sync::Arc};

use serde_json::{Map, Value};

use longhaul_core::{
    tasks::Task,
    tools::{CallToolResult, ListToolsResult, Tool, ToolCallOutcome},
};

use crate::store::StoreError;

/// The async return type of a tool call.
///
/// A tool can:
/// * Complete immediately → [`ToolCallOutcome::Content`]
/// * Spawn a long-running task → [`ToolCallOutcome::Task`]
/// * Pause for client input → [`ToolCallOutcome::InputRequired`]
pub type CallResult = Result<ToolCallOutcome, ToolError>;

/// Errors that a tool handler can return. They become JSON-RPC `-32602` or
/// `-32603` responses depending on the variant.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Bad arguments from the client (→ `-32602`).
    #[error("invalid params: {0}")]
    InvalidParams(String),
    /// Internal failure in the tool (→ `isError: true` content result,
    /// *not* a protocol-level error).
    #[error("tool error: {0}")]
    ToolFailed(String),
    /// Store failure while managing a spawned task.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl ToolError {
    /// Helper: bad-argument error.
    pub fn invalid(msg: impl Into<String>) -> Self {
        ToolError::InvalidParams(msg.into())
    }
}

/// A single executable tool registration.
pub struct ToolEntry {
    /// The protocol-visible tool definition (name, schemas, …).
    pub definition: Tool,
    /// The handler called when the tool is invoked.
    pub handler: Arc<dyn ToolHandler>,
}

/// Trait for tool implementations.
///
/// Implement this for each tool and register it with [`Registry::register`].
#[async_trait::async_trait]
pub trait ToolHandler: Send + Sync + 'static {
    /// Invoke the tool with the given arguments. May return a direct result,
    /// a task handle, or an input-required result.
    async fn call(
        &self,
        arguments: Option<Map<String, Value>>,
        input_responses: Option<Map<String, Value>>,
        request_state: Option<String>,
    ) -> CallResult;
}

// ---------------------------------------------------------------------------
// async_trait re-export so callers can use it without a direct dep
// ---------------------------------------------------------------------------

pub use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// The immutable, clone-cheap tool registry shared across request handlers.
#[derive(Clone, Default)]
pub struct Registry {
    tools: Arc<HashMap<String, Arc<ToolEntry>>>,
}

impl Registry {
    /// Build a registry from a list of entries.
    pub fn from_entries(entries: Vec<ToolEntry>) -> Self {
        let mut map = HashMap::new();
        for entry in entries {
            map.insert(entry.definition.name.clone(), Arc::new(entry));
        }
        Self {
            tools: Arc::new(map),
        }
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<ToolEntry>> {
        self.tools.get(name).cloned()
    }

    /// All tools, sorted by name (stable ordering for `tools/list`).
    pub fn all_sorted(&self) -> Vec<Arc<ToolEntry>> {
        let mut entries: Vec<_> = self.tools.values().cloned().collect();
        entries.sort_by(|a, b| a.definition.name.cmp(&b.definition.name));
        entries
    }

    /// Build the `tools/list` result (no pagination — small registries).
    ///
    /// Cache metadata (`ttlMs`, `cacheScope`) can be set by the caller
    /// after constructing the result.
    pub fn list_result(&self) -> ListToolsResult {
        ListToolsResult {
            tools: self
                .all_sorted()
                .into_iter()
                .map(|e| e.definition.clone())
                .collect(),
            next_cursor: None,
            ttl_ms: Some(60_000), // callers may override
            cache_scope: Some("session".to_owned()),
            meta: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Utility: build a simple immediate CallToolResult
// ---------------------------------------------------------------------------

/// Build an immediate text success result.
pub fn ok_text(text: impl Into<String>) -> ToolCallOutcome {
    ToolCallOutcome::Content(CallToolResult {
        content: vec![longhaul_core::tools::ContentBlock::text(text)],
        structured_content: None,
        is_error: None,
        meta: None,
    })
}

/// Build an immediate error content result (not a protocol error).
pub fn err_content(message: impl Into<String>) -> ToolCallOutcome {
    ToolCallOutcome::Content(CallToolResult {
        content: vec![longhaul_core::tools::ContentBlock::text(message)],
        structured_content: None,
        is_error: Some(true),
        meta: None,
    })
}

/// Build a task handle result.
pub fn task_handle(task: Task) -> ToolCallOutcome {
    use longhaul_core::tasks::TaskHandleResult;
    ToolCallOutcome::Task(TaskHandleResult::new(task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use longhaul_core::tools::Tool;
    use serde_json::json;

    struct Echo;

    #[async_trait]
    impl ToolHandler for Echo {
        async fn call(
            &self,
            args: Option<Map<String, Value>>,
            _ir: Option<Map<String, Value>>,
            _rs: Option<String>,
        ) -> CallResult {
            let msg = args
                .as_ref()
                .and_then(|a| a.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            Ok(ok_text(msg))
        }
    }

    fn make_tool(name: &str) -> ToolEntry {
        ToolEntry {
            definition: Tool {
                name: name.to_owned(),
                title: None,
                description: Some(format!("{name} tool")),
                input_schema: json!({"type": "object"}),
                output_schema: None,
                meta: None,
                extra: Default::default(),
            },
            handler: Arc::new(Echo),
        }
    }

    #[test]
    fn registry_sorts_tools_by_name() {
        let reg = Registry::from_entries(vec![
            make_tool("zebra"),
            make_tool("alpha"),
            make_tool("middle"),
        ]);
        let sorted = reg.all_sorted();
        assert_eq!(sorted[0].definition.name, "alpha");
        assert_eq!(sorted[1].definition.name, "middle");
        assert_eq!(sorted[2].definition.name, "zebra");
    }

    #[test]
    fn registry_list_result_includes_all_tools() {
        let reg = Registry::from_entries(vec![make_tool("a"), make_tool("b")]);
        let result = reg.list_result();
        assert_eq!(result.tools.len(), 2);
        assert_eq!(result.ttl_ms, Some(60_000));
    }

    #[test]
    fn registry_get_returns_none_for_unknown() {
        let reg = Registry::from_entries(vec![make_tool("known")]);
        assert!(reg.get("unknown").is_none());
        assert!(reg.get("known").is_some());
    }
}

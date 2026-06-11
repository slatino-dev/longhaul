//! `indexer` — MCP server example: long-running directory indexing via Tasks.
//!
//! ## What it demonstrates
//!
//! * A **long-running tool** (`index_directory`) that spawns a background
//!   tokio task and immediately returns a [`TaskHandleResult`] so the client
//!   can track progress via `tasks/get` / `tasks/cancel`.
//! * **Cancellation support**: a lightweight monitor task polls the store every
//!   250 ms; when `tasks/cancel` sets the store status to `cancelled`, the
//!   monitor fires the watch channel so `walk_and_index` aborts at the next
//!   directory boundary and the worker records the final `cancelled` status.
//! * **InputRequired round-trip**: when the supplied path matches multiple
//!   sub-directories the tool pauses and asks the client which one to use. The
//!   client retries with `inputResponses` + the echoed `requestState` token.
//! * **SqliteStore** backing, so the stateless-server guarantee holds: two
//!   instances of this binary sharing the same db file can service the same
//!   task lifecycle interchangeably.
//!
//! ## CLI
//!
//! ```text
//! indexer --dir <PATH> [--bind <ADDR>] [--db <PATH>]
//! ```
//!
//! * `--dir`  — root directory to make indexable (required).
//! * `--bind` — bind address (default `127.0.0.1:3000`).
//! * `--db`   — SQLite database path (default `indexer.db`).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use clap::Parser;
use serde_json::{json, Map, Value};
use tokio::sync::watch;
use tracing::{info, warn};
use uuid::Uuid;

use longhaul_core::{
    discover::{
        DiscoverResult, Implementation, ServerCapabilities, TasksCapability, ToolsCapability,
    },
    http::PROTOCOL_VERSION,
    tasks::{InputRequiredResult, Task, TaskHandleResult, TaskStatus},
    tools::{Tool, ToolCallOutcome},
};
use longhaul_server::{
    registry::{CallResult, ToolEntry, ToolError, ToolHandler},
    serve, MemoryStore, Registry, ServerState, SqliteStore, TaskStore,
};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "indexer",
    version,
    about = "MCP 2026-07-28 RC example: long-running directory indexer"
)]
struct Cli {
    /// Root directory to make indexable.
    #[arg(short, long)]
    dir: PathBuf,

    /// Bind address for the HTTP server.
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    bind: String,

    /// SQLite database path for task persistence.
    #[arg(long, default_value = "indexer.db")]
    db: String,
}

// ---------------------------------------------------------------------------
// Indexer result type
// ---------------------------------------------------------------------------

/// The result of indexing: word/symbol counts per file extension.
#[derive(Debug, Default)]
struct IndexResult {
    /// Total files visited.
    files: usize,
    /// Total bytes read.
    bytes: u64,
    /// Word count per file extension (e.g. `.rs` → word_count).
    words_by_ext: HashMap<String, u64>,
}

// ---------------------------------------------------------------------------
// Background indexing worker
// ---------------------------------------------------------------------------

/// Walk `dir` recursively, counting words per file extension.
/// Polls `cancel_rx` between directories to honour cancellation.
fn walk_and_index(dir: &Path, cancel_rx: &watch::Receiver<bool>) -> IndexResult {
    let mut result = IndexResult::default();
    walk_dir(dir, &mut result, cancel_rx);
    result
}

fn walk_dir(dir: &Path, acc: &mut IndexResult, cancel_rx: &watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        if *cancel_rx.borrow() {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, acc, cancel_rx);
        } else if path.is_file() {
            acc.files += 1;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if let Ok(text) = std::fs::read_to_string(&path) {
                let words = text.split_whitespace().count() as u64;
                let file_size = text.len() as u64;
                acc.bytes += file_size;
                *acc.words_by_ext.entry(ext).or_default() += words;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// IndexDirectoryTool — the ToolHandler implementation
// ---------------------------------------------------------------------------

/// Shared state between the tool handler and background workers.
struct IndexerState {
    store: Arc<dyn TaskStore>,
    /// Cancel senders keyed by task id. Workers hold the receiver.
    cancel_txs: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

/// The `index_directory` tool handler.
struct IndexDirectoryTool {
    state: Arc<IndexerState>,
}

/// The `requestState` token we embed during InputRequired. It carries the
/// chosen candidates so the retry knows what the server intended.
#[derive(serde::Serialize, serde::Deserialize)]
struct RequestStateToken {
    candidates: Vec<PathBuf>,
}

#[async_trait]
impl ToolHandler for IndexDirectoryTool {
    async fn call(
        &self,
        arguments: Option<Map<String, Value>>,
        input_responses: Option<Map<String, Value>>,
        request_state: Option<String>,
    ) -> CallResult {
        // --- Retry path (client supplied inputResponses) ---
        if let (Some(ir), Some(rs)) = (input_responses, request_state) {
            return self.handle_retry(ir, rs).await;
        }

        // --- Initial call path ---
        let path_str = arguments
            .as_ref()
            .and_then(|a| a.get("path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid("Missing required argument: path"))?;

        let root = PathBuf::from(path_str);
        if !root.exists() {
            return Err(ToolError::invalid(format!(
                "Path does not exist: {path_str}"
            )));
        }

        // If the path is ambiguous (matches multiple sub-dirs of the same
        // name) pause for client input. Here we simulate ambiguity when the
        // supplied path is a directory that has more than one immediate
        // sub-directory with "src" in the name.
        if root.is_dir() {
            let candidates: Vec<PathBuf> = root
                .read_dir()
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && p.file_name()
                            .is_some_and(|n| n.to_string_lossy().contains("src"))
                })
                .collect();

            if candidates.len() > 1 {
                // Pause for client input: which sub-directory to index?
                let token = RequestStateToken {
                    candidates: candidates.clone(),
                };
                let token_str = serde_json::to_string(&token).unwrap();

                let mut input_requests = Map::new();
                let choice_schema = json!({
                    "type": "string",
                    "description": "Full path of the sub-directory to index",
                    "enum": candidates.iter().map(|p| p.to_string_lossy().as_ref().to_owned()).collect::<Vec<_>>()
                });
                input_requests.insert("selectedPath".to_owned(), choice_schema);

                return Ok(ToolCallOutcome::InputRequired(InputRequiredResult {
                    result_type: longhaul_core::tasks::InputRequiredTag,
                    input_requests,
                    request_state: token_str,
                    meta: None,
                }));
            }

            // If exactly one "src" candidate exists, use it automatically.
            if candidates.len() == 1 {
                return self
                    .spawn_index_task(candidates.into_iter().next().unwrap())
                    .await;
            }
        }

        // No ambiguity: index the supplied path directly.
        self.spawn_index_task(root).await
    }
}

impl IndexDirectoryTool {
    /// Handle the InputRequired retry: client has picked a sub-directory.
    async fn handle_retry(
        &self,
        input_responses: Map<String, Value>,
        request_state: String,
    ) -> CallResult {
        let token: RequestStateToken = serde_json::from_str(&request_state)
            .map_err(|_| ToolError::invalid("Invalid requestState token"))?;

        let chosen = input_responses
            .get("selectedPath")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid("Missing inputResponse: selectedPath"))?;

        let chosen_path = PathBuf::from(chosen);
        if !token.candidates.contains(&chosen_path) {
            return Err(ToolError::invalid(format!(
                "selectedPath {chosen:?} is not one of the offered candidates"
            )));
        }

        self.spawn_index_task(chosen_path).await
    }

    /// Spawn the background indexing worker and return a task handle.
    async fn spawn_index_task(&self, dir: PathBuf) -> CallResult {
        let task_id = Uuid::new_v4().to_string();
        let task = Task::new(&task_id);
        self.state.store.insert(task.clone())?;

        let (cancel_tx, cancel_rx) = watch::channel(false);
        {
            let mut txs = self.state.cancel_txs.lock().unwrap();
            txs.insert(task_id.clone(), cancel_tx.clone());
        }

        // Monitor: polls the store every 250 ms. When tasks/cancel has set the
        // store status to Cancelled this fires the watch channel so the
        // walk_and_index loop aborts at the next directory boundary.
        let monitor_store = Arc::clone(&self.state.store);
        let monitor_task_id = task_id.clone();
        let monitor_tx = cancel_tx;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                match monitor_store.get(&monitor_task_id) {
                    Ok(t) if t.status == TaskStatus::Cancelled => {
                        let _ = monitor_tx.send(true);
                        break;
                    }
                    Ok(t) if t.status.is_terminal() => break, // completed/failed
                    Err(_) => break,                          // task gone
                    _ => {}
                }
            }
        });

        let store = Arc::clone(&self.state.store);
        let cancel_txs = Arc::clone(&self.state.cancel_txs);
        let dir_clone = dir.clone();
        let task_id_clone = task_id.clone();

        tokio::spawn(async move {
            info!(task_id = %task_id_clone, dir = %dir_clone.display(), "index worker started");

            let result =
                tokio::task::spawn_blocking(move || walk_and_index(&dir_clone, &cancel_rx)).await;

            // Clean up the cancel sender.
            {
                let mut txs = cancel_txs.lock().unwrap();
                txs.remove(&task_id_clone);
            }

            match result {
                Ok(idx) => {
                    // Check if we were cancelled during the walk.
                    let was_cancelled = store
                        .get(&task_id_clone)
                        .map(|t| t.status == TaskStatus::Cancelled)
                        .unwrap_or(false);

                    if !was_cancelled {
                        let summary = format!(
                            "Indexed {} files ({} bytes). Words by extension: {}",
                            idx.files,
                            idx.bytes,
                            idx.words_by_ext
                                .iter()
                                .map(|(ext, c)| format!(".{ext}={c}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        info!(task_id = %task_id_clone, %summary, "index complete");

                        // Transition to completed. Ignore errors (e.g. race-cancelled).
                        if let Err(e) = store.update(longhaul_core::tasks::UpdateTaskParams {
                            task_id: task_id_clone.clone(),
                            status: TaskStatus::Completed,
                            meta: None,
                        }) {
                            warn!(task_id = %task_id_clone, err = %e, "could not set completed");
                        }
                    }
                }
                Err(e) => {
                    warn!(task_id = %task_id_clone, err = %e, "index worker panicked");
                    let _ = store.update(longhaul_core::tasks::UpdateTaskParams {
                        task_id: task_id_clone,
                        status: TaskStatus::Failed,
                        meta: None,
                    });
                }
            }
        });

        Ok(ToolCallOutcome::Task(TaskHandleResult::new(task)))
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("indexer=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    if !cli.dir.exists() {
        eprintln!("error: directory does not exist: {}", cli.dir.display());
        std::process::exit(1);
    }

    let store: Arc<dyn TaskStore> = match SqliteStore::open(&cli.db) {
        Ok(s) => {
            info!(db = %cli.db, "opened SqliteStore");
            Arc::new(s)
        }
        Err(e) => {
            warn!(err = %e, "SqliteStore failed, falling back to MemoryStore");
            Arc::new(MemoryStore::new())
        }
    };

    let indexer_state = Arc::new(IndexerState {
        store: Arc::clone(&store),
        cancel_txs: Arc::new(Mutex::new(HashMap::new())),
    });

    let tool = IndexDirectoryTool {
        state: Arc::clone(&indexer_state),
    };

    let tool_def = Tool {
        name: "index_directory".to_owned(),
        title: Some("Index Directory".to_owned()),
        description: Some(
            "Walk a directory tree, count words per file extension, and report statistics. \
             Returns a task handle immediately; poll tasks/get for progress. \
             Supports cancellation via tasks/cancel."
                .to_owned(),
        ),
        input_schema: json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the directory to index."
                }
            },
            "additionalProperties": false
        }),
        output_schema: None,
        meta: None,
        extra: Default::default(),
    };

    let registry = Registry::from_entries(vec![ToolEntry {
        definition: tool_def,
        handler: Arc::new(tool),
    }]);

    let discover_result = DiscoverResult {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        server_info: Implementation::new("longhaul-indexer", env!("CARGO_PKG_VERSION")),
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability { list_changed: None }),
            tasks: Some(TasksCapability::default()),
            extra: Default::default(),
        },
        instructions: Some(
            "Use index_directory to index a directory tree. The tool returns immediately \
             with a task handle; poll tasks/get to check progress. If the path is \
             ambiguous (multiple src-like sub-directories), the tool will pause with \
             inputRequired and ask you to choose."
                .to_owned(),
        ),
        meta: None,
    };

    let state = Arc::new(ServerState {
        discover_result,
        registry,
        store,
    });

    let listener = tokio::net::TcpListener::bind(&cli.bind)
        .await
        .unwrap_or_else(|e| {
            eprintln!("error: cannot bind to {}: {e}", cli.bind);
            std::process::exit(1);
        });

    info!(bind = %cli.bind, dir = %cli.dir.display(), "indexer server ready");
    serve(listener, state).await;
}

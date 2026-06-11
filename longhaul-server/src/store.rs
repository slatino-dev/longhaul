//! [`TaskStore`] trait and its two implementations: [`MemoryStore`]
//! (in-process) and [`SqliteStore`] (rusqlite-backed, shareable across
//! server instances that mount the same database file).
//!
//! The statelessness guarantee tested in the integration suite is the
//! distinguishing feature: any instance with access to the shared
//! [`SqliteStore`] can service *any* task-lifecycle request, so a simple
//! round-robin dispatcher can spread a single client's task sequence across
//! two (or more) server instances and the task still completes correctly.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection};
use thiserror::Error;

use longhaul_core::tasks::{CancelOutcome, CancelTaskResult, Task, TaskStatus, UpdateTaskParams};

/// Errors that can occur in store operations.
#[derive(Debug, Error)]
pub enum StoreError {
    /// No task with the given id exists.
    #[error("task not found: {0}")]
    NotFound(String),
    /// The requested status transition violates the task state machine.
    #[error("illegal task transition: {from} -> {to}")]
    IllegalTransition { from: TaskStatus, to: TaskStatus },
    /// An underlying SQLite error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Lock poisoned (MemoryStore only).
    #[error("internal lock poisoned")]
    LockPoisoned,
}

/// Operations common to every task backing store.
pub trait TaskStore: Send + Sync + 'static {
    /// Return the task, or [`StoreError::NotFound`].
    fn get(&self, task_id: &str) -> Result<Task, StoreError>;

    /// Insert a brand-new task. Replaces any previous entry with the same id
    /// (insert-or-replace semantics, used by tool handlers that generate ids).
    fn insert(&self, task: Task) -> Result<(), StoreError>;

    /// Apply [`UpdateTaskParams`], returning the post-transition task.
    /// Returns [`StoreError::IllegalTransition`] when the requested status
    /// change violates [`TaskStatus::can_transition_to`].
    fn update(&self, params: UpdateTaskParams) -> Result<Task, StoreError>;

    /// Cancel the task (idempotent). Returns the task in its final state.
    fn cancel(&self, task_id: &str) -> Result<CancelTaskResult, StoreError>;
}

// ---------------------------------------------------------------------------
// MemoryStore — in-process Mutex<HashMap>
// ---------------------------------------------------------------------------

/// In-process task store backed by a `Mutex<HashMap>`. Suitable for single-
/// instance servers or tests that don't need cross-process sharing.
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    inner: Arc<Mutex<HashMap<String, Task>>>,
}

impl MemoryStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TaskStore for MemoryStore {
    fn get(&self, task_id: &str) -> Result<Task, StoreError> {
        let guard = self.inner.lock().map_err(|_| StoreError::LockPoisoned)?;
        guard
            .get(task_id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(task_id.to_owned()))
    }

    fn insert(&self, task: Task) -> Result<(), StoreError> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::LockPoisoned)?;
        guard.insert(task.id.clone(), task);
        Ok(())
    }

    fn update(&self, params: UpdateTaskParams) -> Result<Task, StoreError> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::LockPoisoned)?;
        let task = guard
            .get_mut(&params.task_id)
            .ok_or_else(|| StoreError::NotFound(params.task_id.clone()))?;
        if !task.status.can_transition_to(params.status) {
            return Err(StoreError::IllegalTransition {
                from: task.status,
                to: params.status,
            });
        }
        task.status = params.status;
        Ok(task.clone())
    }

    fn cancel(&self, task_id: &str) -> Result<CancelTaskResult, StoreError> {
        let mut guard = self.inner.lock().map_err(|_| StoreError::LockPoisoned)?;
        let task = guard
            .get_mut(task_id)
            .ok_or_else(|| StoreError::NotFound(task_id.to_owned()))?;
        match task.cancel() {
            CancelOutcome::Cancelled | CancelOutcome::AlreadyTerminal(_) => Ok(task.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// SqliteStore — rusqlite-backed, safe across threads (Mutex<Connection>)
// ---------------------------------------------------------------------------

/// Schema version embedded in the database. Bump if the table layout changes.
const SCHEMA_VERSION: i64 = 1;

/// SQLite-backed task store. Multiple server instances that open the **same**
/// database file (WAL mode) share task state, enabling the statelessness
/// test: round-robining a client's task lifecycle across two instances that
/// both hold a `SqliteStore` pointing at the same file must succeed.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) a SQLite database at `path` and ensure the schema is
    /// up to date. Pass `":memory:"` for an in-memory database.
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        // WAL mode for concurrent readers with a single writer.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run schema migrations. Idempotent — safe to call on every open.
    fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);",
        )?;
        let current: i64 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        if current < SCHEMA_VERSION {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tasks (
                    id      TEXT PRIMARY KEY NOT NULL,
                    status  TEXT NOT NULL
                );
                DELETE FROM schema_version;
                INSERT INTO schema_version VALUES (1);",
            )?;
        }
        Ok(())
    }

    fn read_task(conn: &Connection, task_id: &str) -> Result<Task, StoreError> {
        conn.query_row(
            "SELECT id, status FROM tasks WHERE id = ?1",
            params![task_id],
            |row| {
                let id: String = row.get(0)?;
                let status_str: String = row.get(1)?;
                Ok((id, status_str))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound(task_id.to_owned()),
            other => StoreError::Sqlite(other),
        })
        .and_then(|(id, status_str)| {
            let status = parse_status(&status_str).ok_or_else(|| {
                StoreError::Sqlite(rusqlite::Error::InvalidColumnName(status_str))
            })?;
            Ok(Task::new_with_status(id, status))
        })
    }
}

impl TaskStore for SqliteStore {
    fn get(&self, task_id: &str) -> Result<Task, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::LockPoisoned)?;
        Self::read_task(&conn, task_id)
    }

    fn insert(&self, task: Task) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::LockPoisoned)?;
        conn.execute(
            "INSERT OR REPLACE INTO tasks (id, status) VALUES (?1, ?2)",
            params![task.id, task.status.as_str()],
        )?;
        Ok(())
    }

    fn update(&self, params: UpdateTaskParams) -> Result<Task, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::LockPoisoned)?;
        let task = Self::read_task(&conn, &params.task_id)?;
        if !task.status.can_transition_to(params.status) {
            return Err(StoreError::IllegalTransition {
                from: task.status,
                to: params.status,
            });
        }
        conn.execute(
            "UPDATE tasks SET status = ?1 WHERE id = ?2",
            params![params.status.as_str(), params.task_id],
        )?;
        Ok(Task::new_with_status(params.task_id, params.status))
    }

    fn cancel(&self, task_id: &str) -> Result<CancelTaskResult, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::LockPoisoned)?;
        let mut task = Self::read_task(&conn, task_id)?;
        match task.cancel() {
            CancelOutcome::Cancelled => {
                conn.execute(
                    "UPDATE tasks SET status = 'cancelled' WHERE id = ?1",
                    params![task_id],
                )?;
            }
            CancelOutcome::AlreadyTerminal(_) => {
                // Nothing to update; idempotent.
            }
        }
        Ok(task)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a status string. Returns `None` for unrecognised values (storage
/// corruption). Kept private; callers handle the error.
fn parse_status(s: &str) -> Option<TaskStatus> {
    match s {
        "working" => Some(TaskStatus::Working),
        "inputRequired" => Some(TaskStatus::InputRequired),
        "completed" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "cancelled" => Some(TaskStatus::Cancelled),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Task construction helper (not on the public core type)
// ---------------------------------------------------------------------------

/// Extension trait so the server can build a `Task` from stored fields
/// without needing to expose mutable internals.
pub(crate) trait TaskExt {
    fn new_with_status(id: String, status: TaskStatus) -> Task;
}

impl TaskExt for Task {
    fn new_with_status(id: String, status: TaskStatus) -> Task {
        let mut t = Task::new(id);
        // Task::new() starts at Working; set the actual stored status.
        // Use the raw field — we control both sides of the store boundary.
        t.status = status;
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use longhaul_core::tasks::{TaskStatus, UpdateTaskParams};

    fn update(task_id: &str, status: TaskStatus) -> UpdateTaskParams {
        UpdateTaskParams {
            task_id: task_id.to_owned(),
            status,
            meta: None,
        }
    }

    fn lifecycle_test(store: &dyn TaskStore) {
        let task = Task::new("t1");
        store.insert(task).unwrap();

        // Get returns the initial Working state.
        let t = store.get("t1").unwrap();
        assert_eq!(t.status, TaskStatus::Working);

        // working → inputRequired
        let t = store
            .update(update("t1", TaskStatus::InputRequired))
            .unwrap();
        assert_eq!(t.status, TaskStatus::InputRequired);

        // inputRequired → working (resume)
        let t = store.update(update("t1", TaskStatus::Working)).unwrap();
        assert_eq!(t.status, TaskStatus::Working);

        // working → completed
        let t = store.update(update("t1", TaskStatus::Completed)).unwrap();
        assert_eq!(t.status, TaskStatus::Completed);
    }

    fn illegal_transition_test(store: &dyn TaskStore) {
        let task = Task::new("t2");
        store.insert(task).unwrap();
        store.update(update("t2", TaskStatus::Completed)).unwrap(); // working → completed
        let err = store.update(update("t2", TaskStatus::Working)).unwrap_err();
        assert!(
            matches!(err, StoreError::IllegalTransition { .. }),
            "expected IllegalTransition, got {err:?}"
        );
    }

    fn cancel_test(store: &dyn TaskStore) {
        let task = Task::new("t3");
        store.insert(task).unwrap();
        let cancelled = store.cancel("t3").unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        // Idempotent cancel.
        let again = store.cancel("t3").unwrap();
        assert_eq!(again.status, TaskStatus::Cancelled);
    }

    fn not_found_test(store: &dyn TaskStore) {
        assert!(matches!(store.get("no-such"), Err(StoreError::NotFound(_))));
        assert!(matches!(
            store.cancel("no-such"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn memory_store_lifecycle() {
        let store = MemoryStore::new();
        lifecycle_test(&store);
    }

    #[test]
    fn memory_store_illegal_transition() {
        let store = MemoryStore::new();
        illegal_transition_test(&store);
    }

    #[test]
    fn memory_store_cancel() {
        let store = MemoryStore::new();
        cancel_test(&store);
    }

    #[test]
    fn memory_store_not_found() {
        let store = MemoryStore::new();
        not_found_test(&store);
    }

    #[test]
    fn sqlite_store_lifecycle() {
        let store = SqliteStore::open(":memory:").unwrap();
        lifecycle_test(&store);
    }

    #[test]
    fn sqlite_store_illegal_transition() {
        let store = SqliteStore::open(":memory:").unwrap();
        illegal_transition_test(&store);
    }

    #[test]
    fn sqlite_store_cancel() {
        let store = SqliteStore::open(":memory:").unwrap();
        cancel_test(&store);
    }

    #[test]
    fn sqlite_store_not_found() {
        let store = SqliteStore::open(":memory:").unwrap();
        not_found_test(&store);
    }

    #[test]
    fn sqlite_store_insert_or_replace() {
        let store = SqliteStore::open(":memory:").unwrap();
        let task = Task::new("dup");
        store.insert(task).unwrap();
        store.update(update("dup", TaskStatus::Completed)).unwrap();
        // Re-insert resets.
        store.insert(Task::new("dup")).unwrap();
        let t = store.get("dup").unwrap();
        assert_eq!(t.status, TaskStatus::Working);
    }
}

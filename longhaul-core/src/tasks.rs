//! The Tasks extension (MCP 2026-07-28 RC).
//!
//! A `tools/call` may, instead of a direct content result, return a **task
//! handle** ([`TaskHandleResult`]) for long-running work, or pause for
//! client input ([`InputRequiredResult`]). Tasks are then driven through
//! [`METHOD_GET`], [`METHOD_UPDATE`] and [`METHOD_CANCEL`].
//!
//! `tasks/list` is deliberately **absent**: it was removed in the 2026-07-28
//! RC (enumeration of another party's tasks leaked cross-session state and
//! had no sound pagination story). Do not re-add it.
//!
//! ## Status-name note (recorded 2026-06-11)
//!
//! The RC prose names the lifecycle states Working / Input Required /
//! Completed / Failed / Cancelled without fixing a canonical JSON casing in
//! the examples we pinned. This crate serializes them **camelCase** —
//! `"working"`, `"inputRequired"`, `"completed"`, `"failed"`, `"cancelled"`
//! — matching the casing the RC *does* fix for the `"inputRequired"`
//! `resultType` discriminator, and keeping the RC's British spelling
//! `"cancelled"`. If the final release picks different casing, this enum is
//! the single place to change.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::meta::Meta;
use crate::tag::string_tag;

/// Method name: fetch the current state of a task.
pub const METHOD_GET: &str = "tasks/get";
/// Method name: request a status transition on a task.
pub const METHOD_UPDATE: &str = "tasks/update";
/// Method name: cancel a task (idempotent — see [`Task::cancel`]).
pub const METHOD_CANCEL: &str = "tasks/cancel";

/// Lifecycle status of a task. See the module docs for the wire-casing note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    /// The task is executing.
    Working,
    /// The task is paused waiting for client input (see
    /// [`InputRequiredResult`]).
    InputRequired,
    /// Terminal: finished successfully.
    Completed,
    /// Terminal: finished with an error.
    Failed,
    /// Terminal: stopped by a cancel request.
    Cancelled,
}

impl TaskStatus {
    /// Every status, in declaration order (handy for exhaustive tests).
    pub const ALL: [TaskStatus; 5] = [
        TaskStatus::Working,
        TaskStatus::InputRequired,
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Cancelled,
    ];

    /// The exact wire string for this status.
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "inputRequired",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    /// True for the absorbing states ([`Completed`](Self::Completed),
    /// [`Failed`](Self::Failed), [`Cancelled`](Self::Cancelled)).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }

    /// The task lifecycle state machine.
    ///
    /// ```text
    ///                +--------------- working <---------------+
    ///                |                /  |  \                 |
    ///                v               v   v   v                |
    ///          inputRequired --> failed cancelled completed   |
    ///                |    \______________________ ____________|
    ///                +--> cancelled               (resume)
    /// ```
    ///
    /// Legal transitions:
    ///
    /// * `working → inputRequired | completed | failed | cancelled`
    /// * `inputRequired → working | failed | cancelled`
    ///
    /// Everything else is illegal. In particular (interpretive choices,
    /// recorded 2026-06-11, since the RC does not spell these out):
    ///
    /// * Terminal states are absorbing — nothing leaves `completed`,
    ///   `failed` or `cancelled`.
    /// * A same-state "transition" is **not** a transition
    ///   (`can_transition_to(s, s) == false`); status refreshes are reads,
    ///   not updates.
    /// * `inputRequired` cannot jump straight to `completed`: supplying
    ///   input resumes the task (`→ working`), and completion is then the
    ///   executor's decision.
    pub fn can_transition_to(self, next: TaskStatus) -> bool {
        use TaskStatus::*;
        matches!(
            (self, next),
            (Working, InputRequired)
                | (Working, Completed)
                | (Working, Failed)
                | (Working, Cancelled)
                | (InputRequired, Working)
                | (InputRequired, Failed)
                | (InputRequired, Cancelled)
        )
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a status change violates the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    /// Status the task was in.
    pub from: TaskStatus,
    /// Status that was requested.
    pub to: TaskStatus,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "illegal task transition: {} -> {}",
            self.from.as_str(),
            self.to.as_str()
        )
    }
}

impl std::error::Error for InvalidTransition {}

/// Outcome of [`Task::cancel`]. Both variants are successes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The task was live and is now `cancelled`.
    Cancelled,
    /// The task had already reached the given terminal state; it is left
    /// unchanged (idempotent success — see [`Task::cancel`]).
    AlreadyTerminal(TaskStatus),
}

/// The task object: the RC fixes `id` + `status`; any further fields the
/// final release adds round-trip via `extra`, and `_meta` is available as
/// everywhere else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Server-assigned opaque task identifier.
    pub id: String,
    /// Current lifecycle status.
    pub status: TaskStatus,
    /// Task metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    /// Unmodelled fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Task {
    /// Create a new task in the initial [`TaskStatus::Working`] state.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: TaskStatus::Working,
            meta: None,
            extra: Map::new(),
        }
    }

    /// Move the task to `next`, enforcing the state machine.
    pub fn transition_to(&mut self, next: TaskStatus) -> Result<(), InvalidTransition> {
        if self.status.can_transition_to(next) {
            self.status = next;
            Ok(())
        } else {
            Err(InvalidTransition {
                from: self.status,
                to: next,
            })
        }
    }

    /// Cancel the task. **Never fails.**
    ///
    /// Live tasks (`working`, `inputRequired`) move to `cancelled`. Tasks
    /// already in a terminal state are left untouched and the call reports
    /// [`CancelOutcome::AlreadyTerminal`].
    ///
    /// ## Interpretive note (recorded 2026-06-11)
    ///
    /// The RC does not say what `tasks/cancel` against an already-completed
    /// task should do. We treat it as **idempotent success** (like an HTTP
    /// `DELETE` of a gone resource): a cancel that races completion is an
    /// expected interleaving, not a client bug, so it must not surface an
    /// error — the caller gets the task's actual terminal state back. If
    /// the final release mandates an error instead, change it here and in
    /// the server's `tasks/cancel` handler.
    pub fn cancel(&mut self) -> CancelOutcome {
        if self.status.is_terminal() {
            CancelOutcome::AlreadyTerminal(self.status)
        } else {
            self.status = TaskStatus::Cancelled;
            CancelOutcome::Cancelled
        }
    }
}

string_tag! {
    /// The `"resultType": "task"` discriminator of [`TaskHandleResult`].
    pub struct TaskTag = "task";
}

/// A `tools/call` result handing back a task instead of content.
///
/// ## Interpretive note (recorded 2026-06-11)
///
/// The RC normatively fixes the `resultType` discriminator only for
/// `"inputRequired"`. For the task-handle result we adopt the symmetric
/// `"resultType": "task"` with the task object under `"task"`; the
/// discriminator is what lets [`crate::tools::ToolCallOutcome`] classify
/// outcomes without guessing from field presence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskHandleResult {
    /// Always `"task"`.
    #[serde(rename = "resultType")]
    pub result_type: TaskTag,
    /// The newly created (or still-running) task.
    pub task: Task,
    /// Result metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl TaskHandleResult {
    /// Wrap a task as a `tools/call` result.
    pub fn new(task: Task) -> Self {
        Self {
            result_type: TaskTag,
            task,
            meta: None,
        }
    }
}

string_tag! {
    /// The `"resultType": "inputRequired"` discriminator of
    /// [`InputRequiredResult`].
    pub struct InputRequiredTag = "inputRequired";
}

/// A `tools/call` result pausing for client input.
///
/// `inputRequests` maps an input key to a descriptor of what is needed
/// (typically a JSON Schema 2020-12 fragment, passed through untyped).
/// `requestState` is an **opaque** server token the client must echo back
/// unmodified on the retry (see [`crate::tools::CallToolParams::retry`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputRequiredResult {
    /// Always `"inputRequired"`.
    #[serde(rename = "resultType")]
    pub result_type: InputRequiredTag,
    /// What the server needs, keyed by input name.
    #[serde(rename = "inputRequests")]
    pub input_requests: Map<String, Value>,
    /// Opaque resume token; echo it back verbatim.
    #[serde(rename = "requestState")]
    pub request_state: String,
    /// Result metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Params for `tasks/get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetTaskParams {
    /// Id of the task to fetch.
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// Request metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Result of `tasks/get`: the current task object.
pub type GetTaskResult = Task;

/// Params for `tasks/update`: request a status transition. Servers must
/// validate the transition with [`TaskStatus::can_transition_to`] and reply
/// with `-32602` ([`crate::error::INVALID_PARAMS`]) when it is illegal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateTaskParams {
    /// Id of the task to update.
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// The status to move the task to.
    pub status: TaskStatus,
    /// Request metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Result of `tasks/update`: the task after the transition.
pub type UpdateTaskResult = Task;

/// Params for `tasks/cancel`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelTaskParams {
    /// Id of the task to cancel.
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// Request metadata.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

/// Result of `tasks/cancel`: the task afterwards — `cancelled` if it was
/// live, its unchanged terminal state otherwise (idempotent cancel).
pub type CancelTaskResult = Task;

#[cfg(test)]
mod tests {
    use super::*;
    use TaskStatus::*;

    /// The complete legal-transition relation. Paired with the exhaustive
    /// matrix test below, this *is* the state machine's spec.
    const LEGAL: &[(TaskStatus, TaskStatus)] = &[
        (Working, InputRequired),
        (Working, Completed),
        (Working, Failed),
        (Working, Cancelled),
        (InputRequired, Working),
        (InputRequired, Failed),
        (InputRequired, Cancelled),
    ];

    #[test]
    fn transition_matrix_is_exactly_the_legal_set() {
        for from in TaskStatus::ALL {
            for to in TaskStatus::ALL {
                assert_eq!(
                    from.can_transition_to(to),
                    LEGAL.contains(&(from, to)),
                    "{from} -> {to}"
                );
            }
        }
    }

    #[test]
    fn terminal_states_are_absorbing() {
        for from in [Completed, Failed, Cancelled] {
            assert!(from.is_terminal());
            for to in TaskStatus::ALL {
                assert!(!from.can_transition_to(to), "{from} must not reach {to}");
            }
        }
        assert!(!Working.is_terminal());
        assert!(!InputRequired.is_terminal());
    }

    #[test]
    fn same_state_is_not_a_transition() {
        for s in TaskStatus::ALL {
            assert!(!s.can_transition_to(s), "{s} -> {s} must be illegal");
        }
    }

    #[test]
    fn transition_to_mutates_on_success_and_preserves_on_failure() {
        let mut task = Task::new("task-1");
        task.transition_to(InputRequired).unwrap();
        assert_eq!(task.status, InputRequired);
        task.transition_to(Working).unwrap();
        task.transition_to(Completed).unwrap();
        assert_eq!(task.status, Completed);

        // Illegal: out of a terminal state. Status must be untouched.
        let err = task.transition_to(Working).unwrap_err();
        assert_eq!(
            err,
            InvalidTransition {
                from: Completed,
                to: Working
            }
        );
        assert_eq!(task.status, Completed);

        // Illegal: inputRequired cannot jump straight to completed.
        let mut paused = Task::new("task-2");
        paused.transition_to(InputRequired).unwrap();
        assert!(paused.transition_to(Completed).is_err());
        assert_eq!(paused.status, InputRequired);
    }

    #[test]
    fn cancel_is_idempotent_success_on_terminal_tasks() {
        // Live task: really cancels.
        let mut task = Task::new("t");
        assert_eq!(task.cancel(), CancelOutcome::Cancelled);
        assert_eq!(task.status, Cancelled);

        // Cancel again: success, unchanged.
        assert_eq!(task.cancel(), CancelOutcome::AlreadyTerminal(Cancelled));
        assert_eq!(task.status, Cancelled);

        // Cancel-on-completed: success, stays completed (2026-06-11 note).
        let mut done = Task::new("t2");
        done.transition_to(Completed).unwrap();
        assert_eq!(done.cancel(), CancelOutcome::AlreadyTerminal(Completed));
        assert_eq!(done.status, Completed);

        // Paused tasks are cancellable.
        let mut paused = Task::new("t3");
        paused.transition_to(InputRequired).unwrap();
        assert_eq!(paused.cancel(), CancelOutcome::Cancelled);
        assert_eq!(paused.status, Cancelled);
    }

    #[test]
    fn status_wire_strings_are_pinned() {
        for (status, wire) in [
            (Working, "\"working\""),
            (InputRequired, "\"inputRequired\""),
            (Completed, "\"completed\""),
            (Failed, "\"failed\""),
            (Cancelled, "\"cancelled\""),
        ] {
            assert_eq!(serde_json::to_string(&status).unwrap(), wire);
            assert_eq!(
                serde_json::from_str::<TaskStatus>(wire).unwrap(),
                status,
                "{wire}"
            );
        }
        // Pre-camelCase / foreign spellings must not parse.
        assert!(serde_json::from_str::<TaskStatus>("\"input_required\"").is_err());
        assert!(serde_json::from_str::<TaskStatus>("\"canceled\"").is_err());
    }

    #[test]
    fn task_round_trips_and_preserves_unknown_fields() {
        let raw = r#"{"id":"task-42","status":"inputRequired","pollIntervalMs":500}"#;
        let task: Task = serde_json::from_str(raw).unwrap();
        assert_eq!(task.id, "task-42");
        assert_eq!(task.status, InputRequired);
        assert_eq!(task.extra["pollIntervalMs"], 500);

        let back = serde_json::to_value(&task).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn task_handle_result_requires_the_task_tag() {
        let raw = r#"{"resultType":"task","task":{"id":"task-42","status":"working"}}"#;
        let handle: TaskHandleResult = serde_json::from_str(raw).unwrap();
        assert_eq!(handle.task.id, "task-42");
        assert_eq!(handle.task.status, Working);

        let wrong = r#"{"resultType":"job","task":{"id":"task-42","status":"working"}}"#;
        assert!(serde_json::from_str::<TaskHandleResult>(wrong).is_err());
    }
}

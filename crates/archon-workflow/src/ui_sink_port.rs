//! The port through which a live workflow run reports progress to a user
//! interface.
//!
//! Execution decides *what* a run tells the user; the host decides *where* that
//! lands. Before this port the two were the same decision: stage execution named
//! `archon_tui::event_channel::TuiEventSender` directly, which made "run a
//! workflow" and "have a terminal UI attached" the same capability. The CLI path
//! already contradicted that — `run_live_cli_action` builds a channel and spawns
//! a task whose only job is to drain it, because there is no TUI to receive
//! anything.
//!
//! So the direction is inverted, the same way [`crate::llm_client_port`] inverts
//! the LLM. This crate declares the three things a run needs to say
//! ([`WorkflowUiEvent`]) and the sink that accepts them ([`WorkflowUiSink`]);
//! the host supplies an implementation. The bin crate's `TuiEventSender` is one,
//! behind an adapter, and nothing here names it.
//!
//! [`WorkflowUiSink::emit`] is the backpressured send, not the try-send: every
//! production call site this port replaced used `send_async`, which waits for
//! capacity. Losing a completion report because a bounded queue was momentarily
//! full is not a failure mode this runtime accepts, which is why the bin crate
//! carries an architecture test forbidding `let _ = ...emit(...)`.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

/// Lifecycle position of one agent invocation, as shown to the user.
///
/// Deliberately narrower than the host's own status enum: these three are what
/// workflow execution can distinguish. Statuses that describe the host's
/// scheduling of an agent rather than the workflow's view of it (queued,
/// backgrounded, waiting on a tool) are the host's to report, and a run has no
/// way to know them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowActivityStatus {
    Running,
    Complete,
    Failed,
}

/// One agent's progress within a run.
///
/// `id` is stable across the updates describing a single agent, so a host that
/// renders a live list can replace a row rather than append to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowActivityUpdate {
    pub id: String,
    pub name: String,
    pub status: WorkflowActivityStatus,
    pub detail: Option<String>,
    pub run_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Everything a live run tells the user directly.
///
/// Not to be confused with [`crate::events::WorkflowEvent`], which is the
/// durable run log written to disk and replayed on resume. This enum is
/// transient presentation: dropping one loses a line of output, dropping the
/// other loses run state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowUiEvent {
    /// Assistant-visible text, appended to the run's output stream.
    Text(String),
    /// A run-level failure, surfaced distinctly from ordinary text.
    Error(String),
    /// Progress for one agent invocation.
    Activity(WorkflowActivityUpdate),
}

/// The sink refused an event.
///
/// Carries the host's own message rather than a structured cause: the only
/// thing every call site does with it is embed it in a
/// [`WorkflowError::NotificationDelivery`](crate::error::WorkflowError::NotificationDelivery)
/// message or a `tracing` field, and a classification nothing branches on would
/// be a classification nothing keeps accurate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowUiDeliveryError(String);

impl WorkflowUiDeliveryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for WorkflowUiDeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorkflowUiDeliveryError {}

pub type WorkflowUiResult = Result<(), WorkflowUiDeliveryError>;

/// Where a live run's user-visible output goes.
///
/// Futures are `Send`: stage execution emits from inside spawned tasks, so
/// anything narrower would not be usable there.
#[async_trait]
pub trait WorkflowUiSink: Send + Sync {
    /// Deliver one event, waiting for capacity if the sink is bounded.
    ///
    /// An `Err` means the event did not reach the user and will not be
    /// retried. Call sites that must not proceed without the user having seen
    /// the event — the run-start banner, agent activity a stage's result is
    /// reported against — return the error; ones that merely annotate a run log
    /// it and continue.
    async fn emit(&self, event: WorkflowUiEvent) -> WorkflowUiResult;
}

/// The shared handle execution actually threads around.
///
/// A run hands the same sink to every stage, fanout branch and spawned driver,
/// so the clonable form is the one that appears in signatures.
pub type SharedWorkflowUiSink = Arc<dyn WorkflowUiSink>;

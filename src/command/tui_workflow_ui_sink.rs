//! Host side of `archon_workflow::ui_sink_port`.
//!
//! `archon-workflow` must not name the TUI (see the port's module doc), so this
//! file is where the workflow layer's UI port meets
//! `archon_tui::event_channel::TuiEventSender`.
//!
//! Deliberately not named `workflow_*`, for the same reason
//! [`crate::command::pipeline_workflow_llm`] is not: every
//! `src/command/workflow*.rs` file is destined for `crates/archon-workflow`, and
//! none of them may name `archon_tui`. Keeping the adapter outside that prefix
//! makes the invariant a one-line grep rather than a convention.

use std::sync::Arc;

use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use archon_workflow::ui_sink_port::{
    SharedWorkflowUiSink, WorkflowActivityStatus, WorkflowActivityUpdate, WorkflowUiDeliveryError,
    WorkflowUiEvent, WorkflowUiResult, WorkflowUiSink,
};
use async_trait::async_trait;

/// Presents the bounded TUI event channel through the workflow UI port.
pub(crate) struct TuiWorkflowUiSink {
    inner: TuiEventSender,
}

impl TuiWorkflowUiSink {
    pub(crate) fn new(inner: TuiEventSender) -> Self {
        Self { inner }
    }

    /// The port as an owned trait object, which is how every caller wants it.
    pub(crate) fn arc(inner: TuiEventSender) -> SharedWorkflowUiSink {
        Arc::new(Self::new(inner))
    }
}

#[async_trait]
impl WorkflowUiSink for TuiWorkflowUiSink {
    async fn emit(&self, event: WorkflowUiEvent) -> WorkflowUiResult {
        // `send_async`, not `send`: the port contract is a backpressured send,
        // and dropping a run's output because a bounded queue was briefly full
        // is not a failure mode this runtime accepts.
        self.inner
            .send_async(tui_event(event))
            .await
            // The message is preserved verbatim rather than re-classified: call
            // sites embed it in `WorkflowError::NotificationDelivery` text that
            // tests assert on.
            .map_err(|error| WorkflowUiDeliveryError::new(error.to_string()))
    }
}

/// A real bounded channel behind the port, plus its receiver.
///
/// Workflow tests assert on the `TuiEvent`s a run produces and on what happens
/// when the channel is closed or full, so they need the genuine channel rather
/// than a recording double — a double would test the port, not the delivery
/// behaviour the port was built to preserve.
#[cfg(test)]
pub(crate) fn bounded_workflow_ui_sink(
    capacity: usize,
) -> (
    SharedWorkflowUiSink,
    archon_tui::event_channel::TuiEventReceiver,
) {
    let (tx, rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(capacity);
    (TuiWorkflowUiSink::arc(tx), rx)
}

/// The same, at the channel's own default capacity.
#[cfg(test)]
pub(crate) fn default_workflow_ui_sink() -> (
    SharedWorkflowUiSink,
    archon_tui::event_channel::TuiEventReceiver,
) {
    bounded_workflow_ui_sink(archon_tui::event_channel::TUI_EVENT_CHANNEL_CAPACITY)
}

/// The port, the raw sender behind it, and the receiver.
///
/// One test fills the channel to capacity to prove the transient-retry path
/// *waits* for room rather than dropping a status. Filling needs the try-send
/// [`WorkflowUiSink`] deliberately does not expose, so that test reaches the
/// channel directly — through [`try_fill_one`], so it still never names a
/// `TuiEvent`.
#[cfg(test)]
pub(crate) fn default_workflow_ui_sink_parts() -> (
    SharedWorkflowUiSink,
    TuiEventSender,
    archon_tui::event_channel::TuiEventReceiver,
) {
    let (tx, rx) = archon_tui::event_channel::bounded_tui_event_channel();
    (TuiWorkflowUiSink::arc(tx.clone()), tx, rx)
}

/// Enqueue one payload-free event without waiting. `false` once the queue is
/// full.
#[cfg(test)]
pub(crate) fn try_fill_one(sender: &TuiEventSender) -> bool {
    sender.send(TuiEvent::GenerationStarted).is_ok()
}

fn tui_event(event: WorkflowUiEvent) -> TuiEvent {
    match event {
        WorkflowUiEvent::Text(text) => TuiEvent::TextDelta(text),
        WorkflowUiEvent::Error(message) => TuiEvent::Error(message),
        WorkflowUiEvent::Activity(update) => TuiEvent::AgentActivity(activity_update(update)),
    }
}

fn activity_update(update: WorkflowActivityUpdate) -> AgentActivityUpdate {
    AgentActivityUpdate {
        id: update.id,
        name: update.name,
        // Workflow stages are always dispatched as subagents of the session
        // that launched the run; the port has no way to express anything else
        // and no caller wanted to.
        role: AgentActivityRole::Subagent,
        status: activity_status(update.status),
        current_tool: None,
        detail: update.detail,
        run_id: update.run_id,
        parent_id: None,
        artifact_id: None,
        provider: update.provider,
        model: update.model,
        cost_usd: None,
    }
}

fn activity_status(status: WorkflowActivityStatus) -> AgentActivityStatus {
    match status {
        WorkflowActivityStatus::Running => AgentActivityStatus::Running,
        WorkflowActivityStatus::Complete => AgentActivityStatus::Complete,
        WorkflowActivityStatus::Failed => AgentActivityStatus::Failed,
    }
}

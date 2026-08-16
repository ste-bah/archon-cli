use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use super::{
    ResilientWorkflowUiSink, WorkflowUiDeliveryError, WorkflowUiEvent, WorkflowUiResult,
    WorkflowUiSink,
};

struct RefusingSink {
    attempts: AtomicUsize,
}

#[async_trait]
impl WorkflowUiSink for RefusingSink {
    async fn emit(&self, _event: WorkflowUiEvent) -> WorkflowUiResult {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(WorkflowUiDeliveryError::new("channel closed"))
    }
}

struct AcceptingSink {
    delivered: AtomicUsize,
}

#[async_trait]
impl WorkflowUiSink for AcceptingSink {
    async fn emit(&self, _event: WorkflowUiEvent) -> WorkflowUiResult {
        self.delivered.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The overnight-halt failure mode: the receiver is gone, every emit refuses.
/// The run must keep going — delivery failure is the sink's problem, never the
/// branch's.
#[tokio::test]
async fn a_refusing_sink_never_fails_the_caller() {
    let inner = Arc::new(RefusingSink {
        attempts: AtomicUsize::new(0),
    });
    let sink = ResilientWorkflowUiSink::wrap(inner.clone());
    for _ in 0..3 {
        sink.emit(WorkflowUiEvent::Text("progress".to_string()))
            .await
            .expect("delivery failure is degraded, not propagated");
    }
    // Delivery is still attempted every time; only the refusal is swallowed.
    assert_eq!(inner.attempts.load(Ordering::SeqCst), 3);
}

/// A healthy sink is left exactly alone: every event reaches it.
#[tokio::test]
async fn a_healthy_sink_receives_every_event() {
    let inner = Arc::new(AcceptingSink {
        delivered: AtomicUsize::new(0),
    });
    let sink = ResilientWorkflowUiSink::wrap(inner.clone());
    for _ in 0..5 {
        sink.emit(WorkflowUiEvent::Text("progress".to_string()))
            .await
            .expect("healthy delivery succeeds");
    }
    assert_eq!(inner.delivered.load(Ordering::SeqCst), 5);
}

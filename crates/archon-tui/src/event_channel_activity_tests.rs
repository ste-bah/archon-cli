use super::*;
use crate::events::{
    AgentActivityRole as Role, AgentActivityStatus as Status, AgentActivityUpdate,
};
use serial_test::serial;

fn activity(id: &str, status: Status) -> TuiEvent {
    TuiEvent::AgentActivity(AgentActivityUpdate {
        id: id.into(),
        name: "worker".into(),
        role: Role::Subagent,
        status,
        current_tool: None,
        detail: None,
        run_id: None,
        parent_id: None,
        artifact_id: None,
        provider: None,
        model: None,
        cost_usd: None,
    })
}

fn terminal_on_full_queue(status: Status) {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    let mut rows = Vec::new();
    tx.send(activity("live", Status::Running)).unwrap();
    if let TuiEvent::AgentActivity(update) = rx.try_recv().unwrap() {
        crate::agent_activity::apply_update(&mut rows, update);
    }
    tx.send(TuiEvent::TextDelta("occupied".into())).unwrap();
    let _ = tx.send(activity("live", status)); // synchronous sink ignores the return value
    // Reconciliation must arrive while the unrelated queued event remains;
    // a continuously busy channel must not starve terminal state.
    let event = rx.try_recv().unwrap();
    if let TuiEvent::AgentActivitySnapshot(snapshot) = event {
        rows.clear();
        for update in snapshot {
            crate::agent_activity::apply_update(&mut rows, update);
        }
    }
    assert!(rows.is_empty(), "terminal event left a stale row: {rows:?}");
    assert!(rx.len() <= 1);
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
        tx.send(TuiEvent::GenerationStarted).unwrap();
        let producer = tx.clone();
        let pending = tokio::spawn(async move { producer.send_async(activity("live", Status::Running)).await });
        tokio::task::yield_now().await;
        tx.send(activity("live", status)).unwrap();
        assert!(matches!(rx.recv().await, Some(TuiEvent::AgentActivitySnapshot(rows)) if rows.is_empty()));
        assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
        pending.await.unwrap().unwrap();
        assert!(rx.try_recv().is_err(), "old backpressured activity resurrected the terminal row");
    });
}
#[test]
#[serial(tui_drain_metrics)]
fn gc_full_channel_completion_reconciles() {
    terminal_on_full_queue(Status::Complete);
}
#[test]
#[serial(tui_drain_metrics)]
fn gc_full_channel_failure_reconciles() {
    terminal_on_full_queue(Status::Failed);
}
#[test]
#[serial(tui_drain_metrics)]
fn gc_full_channel_cancellation_reconciles() {
    terminal_on_full_queue(Status::Cancelled);
}

fn separate_instances(terminal: Status) {
    use archon_observability::{
        AgentActivityEvent as Event, AgentActivityKind as Kind, AgentActivityStatus as State,
    };
    let make = |instance: &str, state| {
        AgentActivityUpdate::from(
            Event::new("session", Kind::AgentRunning, state, "activity")
                .with_subagent_id("shared")
                .with_agent_id(format!("call-instance:{instance}")),
        )
    };
    let mut rows = Vec::new();
    crate::agent_activity::apply_update(&mut rows, make("live-call", State::Running));
    crate::agent_activity::apply_update(&mut rows, make("refused-call", State::Queued));
    let mut end = make("refused-call", State::Failed);
    end.status = terminal;
    crate::agent_activity::apply_update(&mut rows, end);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].id.contains("live-call"));
}
#[test]
fn gc_duplicate_failed_call_preserves_live_row() {
    separate_instances(Status::Failed);
}
#[test]
fn gc_duplicate_cancelled_call_preserves_live_row() {
    separate_instances(Status::Cancelled);
}
#[test]
fn gc_duplicate_completed_call_preserves_live_row() {
    separate_instances(Status::Complete);
}

#[test]
#[serial(tui_drain_metrics)]
fn r2_running_sync_rejection_preserves_queued_events() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).unwrap();
    let sent = tx.send(activity("running", Status::Running));
    assert!(
        sent.is_err(),
        "nonterminal full-queue send claimed reconciliation delivery"
    );
    assert!(matches!(rx.try_recv(), Ok(TuiEvent::GenerationStarted)));
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn r2_queued_async_waits_for_capacity() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).unwrap();
    let send = tokio::spawn(async move { tx.send_async(activity("queued", Status::Queued)).await });
    tokio::task::yield_now().await;
    assert!(!send.is_finished(), "queued async update bypassed capacity");
    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    send.await.unwrap().unwrap();
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::AgentActivity(update)) if update.status == Status::Queued)
    );
}

#[tokio::test]
#[serial(tui_drain_metrics)]
async fn r2_running_atomic_async_waits_for_capacity() {
    let (tx, mut rx) = bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).unwrap();
    let send = tokio::spawn(async move {
        tx.send_atomic_async(activity("running", Status::Running))
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !send.is_finished(),
        "atomic async activity bypassed capacity"
    );
    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    send.await.unwrap().unwrap();
    assert!(
        matches!(rx.recv().await, Some(TuiEvent::AgentActivity(update)) if update.status == Status::Running)
    );
}

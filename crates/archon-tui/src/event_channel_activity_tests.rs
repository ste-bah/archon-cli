use super::*;
use crate::events::{
    AgentActivityRole as Role, AgentActivityStatus as Status, AgentActivityUpdate,
};

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
}
#[test]
fn gc_full_channel_completion_reconciles() {
    terminal_on_full_queue(Status::Complete);
}
#[test]
fn gc_full_channel_failure_reconciles() {
    terminal_on_full_queue(Status::Failed);
}
#[test]
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

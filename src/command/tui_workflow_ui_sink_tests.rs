use std::time::Duration;

use archon_tui::app::TuiEvent;
use archon_workflow::{WorkflowActivityStatus, WorkflowActivityUpdate, WorkflowUiEvent};

use super::tui_workflow_ui_sink::fixed_decomposition_workflow_ui_sink_parts;

fn activity(id: &str) -> WorkflowUiEvent {
    WorkflowUiEvent::Activity(WorkflowActivityUpdate {
        id: id.into(),
        name: "fixed decomposition acceptance".into(),
        status: WorkflowActivityStatus::Running,
        detail: Some("author attempt started".into()),
        run_id: Some("wf-fixed".into()),
        provider: None,
        model: None,
    })
}

#[tokio::test]
async fn fixed_progress_saturation_coalesces_without_stalling() {
    let (sink, fill, mut receiver) = fixed_decomposition_workflow_ui_sink_parts(1);
    fill.send(TuiEvent::GenerationStarted).unwrap();

    tokio::time::timeout(Duration::from_millis(50), async {
        sink.emit(activity("one")).await.unwrap();
        sink.emit(activity("two")).await.unwrap();
        sink.emit(activity("three")).await.unwrap();
    })
    .await
    .expect("fixed transient progress must not wait for TUI capacity");

    assert!(matches!(
        receiver.recv().await,
        Some(TuiEvent::GenerationStarted)
    ));
    sink.emit(activity("four")).await.unwrap();
    let marker = receiver.recv().await.expect("coalescing marker");
    assert!(
        matches!(marker, TuiEvent::TextDelta(ref text) if text.contains(".decompose.log")),
        "{marker:?}"
    );

    sink.emit(activity("five")).await.unwrap();
    assert!(matches!(
        receiver.recv().await,
        Some(TuiEvent::AgentActivity(update)) if update.id == "five"
    ));
}

#[tokio::test]
async fn fixed_coalescing_never_discards_the_current_terminal_event() {
    let (sink, fill, mut receiver) = fixed_decomposition_workflow_ui_sink_parts(2);
    fill.send(TuiEvent::GenerationStarted).unwrap();
    fill.send(TuiEvent::GenerationStarted).unwrap();
    sink.emit(activity("dropped-progress")).await.unwrap();

    assert!(matches!(
        receiver.recv().await,
        Some(TuiEvent::GenerationStarted)
    ));
    assert!(matches!(
        receiver.recv().await,
        Some(TuiEvent::GenerationStarted)
    ));
    sink.emit(WorkflowUiEvent::Error("terminal failure".into()))
        .await
        .unwrap();

    let first = tokio::time::timeout(Duration::from_millis(50), receiver.recv())
        .await
        .expect("first completion delivery")
        .expect("first event");
    let second = tokio::time::timeout(Duration::from_millis(50), receiver.recv())
        .await
        .expect("terminal event must not be discarded")
        .expect("second event");
    assert!(
        matches!(first, TuiEvent::Error(ref text) if text == "terminal failure")
            || matches!(second, TuiEvent::Error(ref text) if text == "terminal failure"),
        "first={first:?} second={second:?}"
    );
    assert!(
        matches!(first, TuiEvent::TextDelta(_)) || matches!(second, TuiEvent::TextDelta(_)),
        "first={first:?} second={second:?}"
    );
}

#[tokio::test]
async fn fixed_terminal_delivery_loss_never_aborts_persisted_execution() {
    let (sink, fill, receiver) = fixed_decomposition_workflow_ui_sink_parts(1);
    fill.send(TuiEvent::GenerationStarted).unwrap();
    tokio::time::timeout(Duration::from_millis(50), async {
        sink.emit(WorkflowUiEvent::Error("terminal while full".into()))
            .await
            .unwrap();
    })
    .await
    .expect("full presentation channel must not stall terminalization");

    drop(receiver);
    tokio::time::timeout(Duration::from_millis(50), async {
        sink.emit(WorkflowUiEvent::Text("terminal while closed".into()))
            .await
            .unwrap();
    })
    .await
    .expect("closed presentation channel must not abort persisted execution");
}

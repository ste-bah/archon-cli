//! Issue 322: every path after a call's activity row opens closes the row
//! with one terminal event, and the queued message states the real clock
//! state.

use std::sync::Arc;
use std::time::Duration;

use archon_observability::{
    AgentActivityEvent, AgentActivityKind as Kind, AgentActivityStatus as Status,
    InMemoryActivitySink,
};
use archon_tools::subagent_dispatch_clock::{DispatchClock, scope_session};
use tokio_util::sync::CancellationToken;

use super::tests::{queued_request, test_executor_with_sink};
use super::*;

fn executor(cap: usize) -> (Arc<AgentSubagentExecutor>, InMemoryActivitySink) {
    let sink = InMemoryActivitySink::new();
    let executor = test_executor_with_sink(cap, Some(Arc::new(sink.clone())));
    (executor, sink)
}

fn events_for(sink: &InMemoryActivitySink, id: &str) -> Vec<AgentActivityEvent> {
    sink.events()
        .into_iter()
        .filter(|event| event.subagent_id.as_deref() == Some(id))
        .collect()
}

fn statuses(sink: &InMemoryActivitySink, id: &str) -> Vec<Status> {
    events_for(sink, id).iter().map(|e| e.status).collect()
}

fn is_terminal(status: Status) -> bool {
    matches!(
        status,
        Status::Completed | Status::Failed | Status::Cancelled
    )
}

async fn until_queued(sink: &InMemoryActivitySink, id: &str) {
    for _ in 0..500 {
        if statuses(sink, id).contains(&Status::Queued) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("call {id} never reported that it is queued");
}

/// Run call `id` through the executor while every slot is held, return the
/// held permit and the call's task.
async fn queue_call(
    executor: &Arc<AgentSubagentExecutor>,
    sink: &InMemoryActivitySink,
    id: &str,
    request: SubagentRequest,
    cancel: &CancellationToken,
) -> (
    tokio::sync::OwnedSemaphorePermit,
    tokio::task::JoinHandle<Result<String, ExecutorError>>,
) {
    let held = executor
        .acquire_subagent_capacity(&CancellationToken::new())
        .await
        .unwrap();
    let task = {
        let executor = Arc::clone(executor);
        let cancel = cancel.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            executor
                .run_subagent_to_completion(id, request, ToolContext::default(), cancel)
                .await
        })
    };
    until_queued(sink, id).await;
    (held, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_cancelled_while_queued_closes_its_row_as_cancelled() {
    let (executor, sink) = executor(1);
    let cancel = CancellationToken::new();
    let (_held, task) = queue_call(&executor, &sink, "q-cancel", queued_request(), &cancel).await;
    cancel.cancel();
    let err = task
        .await
        .unwrap()
        .expect_err("a cancelled queued call errors");
    assert!(err.to_string().contains("subagent cancelled"), "{err}");
    let events = events_for(&sink, "q-cancel");
    assert_eq!(
        statuses(&sink, "q-cancel"),
        [Status::Queued, Status::Cancelled]
    );
    assert_eq!(events[1].kind, Kind::Cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_dropped_while_waiting_closes_its_row() {
    let (executor, sink) = executor(1);
    let cancel = CancellationToken::new();
    let (_held, task) = queue_call(&executor, &sink, "q-drop", queued_request(), &cancel).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        statuses(&sink, "q-drop"),
        [Status::Queued, Status::Cancelled]
    );
    assert!(!cancel.is_cancelled(), "the drop alone closed the row");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_whose_slots_close_fails_its_row() {
    let (executor, sink) = executor(1);
    let cancel = CancellationToken::new();
    let (_held, task) = queue_call(&executor, &sink, "q-closed", queued_request(), &cancel).await;
    executor.subagent_capacity.close();
    let err = task.await.unwrap().expect_err("a closed semaphore errors");
    assert!(err.to_string().contains("semaphore closed"), "{err}");
    assert_eq!(
        statuses(&sink, "q-closed"),
        [Status::Queued, Status::Failed]
    );
    assert!(
        events_for(&sink, "q-closed")[1]
            .message
            .contains("semaphore closed")
    );
}

/// The slot is taken (the row reads "running"), then the call is refused
/// before its run starts: the row must still close.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_refused_before_its_run_starts_closes_its_row() {
    let (executor, sink) = executor(2);
    let cancel = CancellationToken::new();
    let held_other = executor.acquire_subagent_capacity(&cancel).await.unwrap();
    let mut request = queued_request();
    request.subagent_type = Some("no-such-agent-type-322".into());
    let (held, task) = queue_call(&executor, &sink, "q-refused", request, &cancel).await;
    drop(held);
    let err = task
        .await
        .unwrap()
        .expect_err("an unknown agent type is refused");
    assert!(err.to_string().contains("Unknown subagent type"), "{err}");
    assert_eq!(
        statuses(&sink, "q-refused"),
        [Status::Queued, Status::Running, Status::Failed]
    );
    drop(held_other);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_whose_registration_fails_closes_its_row() {
    let (executor, sink) = executor(1);
    executor
        .subagent_manager
        .lock()
        .await
        .register_with_id("q-dup".into(), queued_request())
        .unwrap();
    let cancel = CancellationToken::new();
    let (held, task) = queue_call(&executor, &sink, "q-dup", queued_request(), &cancel).await;
    drop(held);
    let err = task
        .await
        .unwrap()
        .expect_err("a running id is not registered twice");
    assert!(err.to_string().contains("Failed to register"), "{err}");
    assert_eq!(
        statuses(&sink, "q-dup"),
        [Status::Queued, Status::Running, Status::Failed]
    );
}

/// The run's own terminal event closes the row: neither `settle` nor the
/// drop sends a second one, and a row that never opened sends none.
#[tokio::test]
async fn a_row_sends_exactly_one_terminal_event() {
    let (executor, sink) = executor(1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    {
        let mut row = executor.activity_row("ran");
        row.queued("worker", "m", "queued".into());
        row.started("worker", "m");
        row.finished("worker", "m", &Err("Subagent failed: boom".into()));
        row.settle(&Err(ExecutorError::Internal("boom".into())), &cancel);
    }
    {
        let mut row = executor.activity_row("never-opened");
        row.settle(&Err(ExecutorError::Internal("refused".into())), &cancel);
    }
    let terminal: Vec<_> = statuses(&sink, "ran")
        .into_iter()
        .filter(|s| is_terminal(*s))
        .collect();
    assert_eq!(terminal, [Status::Failed]);
    assert!(events_for(&sink, "never-opened").is_empty());
}

/// Wait for a slot as `id` inside `session` (an agent id and its clocks),
/// release the slot, and return the queued and acquired messages.
async fn slot_messages(session: Option<(&str, Vec<Arc<DispatchClock>>)>) -> (String, String) {
    let (executor, sink) = executor(1);
    let held = executor
        .acquire_subagent_capacity(&CancellationToken::new())
        .await
        .unwrap();
    let wait = {
        let executor = Arc::clone(&executor);
        async move {
            let mut row = executor.activity_row("slot");
            let cancel = CancellationToken::new();
            let request = queued_request();
            executor
                .acquire_subagent_slot("slot", &request, &cancel, &mut row)
                .await
                .map(drop)
        }
    };
    let task = match session {
        Some((agent_id, clocks)) => tokio::spawn(scope_session(agent_id.to_string(), clocks, wait)),
        None => tokio::spawn(wait),
    };
    until_queued(&sink, "slot").await;
    drop(held);
    task.await.unwrap().unwrap();
    let events = events_for(&sink, "slot");
    let message = |status| {
        events
            .iter()
            .find(|e| e.status == status)
            .map(|e| e.message.clone())
            .unwrap()
    };
    (message(Status::Queued), message(Status::Running))
}

const PAUSED: &str = "its dispatch clocks do not run until it starts";
const STARTS: &str = "its dispatch clocks start now";
const NO_CLOCK: &str = "no dispatch clock is installed for it, so none is stopped";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_with_clocks_says_they_are_stopped() {
    let (queued, acquired) = slot_messages(Some(("slot", vec![DispatchClock::new()]))).await;
    assert!(queued.ends_with(PAUSED), "{queued}");
    assert!(acquired.ends_with(STARTS), "{acquired}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_with_no_clock_session_does_not_claim_stopped_clocks() {
    let (queued, acquired) = slot_messages(None).await;
    assert!(!queued.contains(PAUSED), "{queued}");
    assert!(queued.ends_with(NO_CLOCK), "{queued}");
    assert!(!acquired.contains("dispatch clock"), "{acquired}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_whose_session_has_no_clocks_does_not_claim_stopped_clocks() {
    let (queued, acquired) = slot_messages(Some(("slot", Vec::new()))).await;
    assert!(queued.ends_with(NO_CLOCK), "{queued}");
    assert!(!acquired.contains("dispatch clock"), "{acquired}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_call_inside_another_agents_clock_session_has_no_clock_of_its_own() {
    let (queued, acquired) = slot_messages(Some(("other", vec![DispatchClock::new()]))).await;
    assert!(queued.ends_with(NO_CLOCK), "{queued}");
    assert!(!acquired.contains("dispatch clock"), "{acquired}");
}

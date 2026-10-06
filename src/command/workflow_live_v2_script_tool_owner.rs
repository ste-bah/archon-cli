//! Captured executor ownership at tool admission and every execution poll.
use archon_workflow::{WorkflowResult, WorkflowStore, control_pause::PauseOwner};
use std::{future::Future, task::Poll, time::Duration};

pub(super) async fn execute<T>(
    store: &WorkflowStore,
    run_id: &str,
    executor: u64,
    work: impl Future<Output = WorkflowResult<T>>,
) -> WorkflowResult<T> {
    let mut work = Box::pin(work);
    let mut watch = tokio::time::interval(Duration::from_secs(2));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Each synchronous poll (including registry admission and tool entry) is
    // serialized against pause/resume. Pending releases the lock immediately.
    // The timer also wakes tools which would otherwise remain pending forever.
    std::future::poll_fn(|cx| {
        // Register the next timer wake even when the immediate tick is ready.
        while watch.poll_tick(cx).is_ready() {}
        match store.with_run_lock(run_id, |locked| {
            PauseOwner::Executor(executor).require_pauser(&locked.load_state(run_id)?)?;
            Ok(work.as_mut().poll(cx))
        }) {
            Ok(Poll::Ready(result)) => Poll::Ready(result),
            Ok(Poll::Pending) => Poll::Pending,
            Err(refused) => Poll::Ready(Err(refused)),
        }
    })
    .await
}

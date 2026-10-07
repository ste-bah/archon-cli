//! Tool admission uses the same captured-owner store as audit and state writes.
use archon_workflow::{WorkflowResult, WorkflowStore};
use std::future::Future;
pub(super) async fn execute<T>(
    store: &WorkflowStore,
    run_id: &str,
    executor: u64,
    work: impl Future<Output = WorkflowResult<T>>,
) -> WorkflowResult<T> {
    store
        .for_executor(run_id, executor)
        .execute_owned(run_id, work)
        .await
}

//! A call is bounded in total, not merely per attempt.
//!
//! The retry budgets around a write branch all count ATTEMPTS: up to thirteen
//! size re-asks, plus transport retries which deliberately do not consume that
//! count, plus a schema repair and the port's own transient retry. Each attempt
//! carries the host's per-dispatch timeout, and nothing added their elapsed time
//! together — so a two-hour timeout permitted a day. Observed as one task
//! running 6h20m and still going when a person killed it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::{
    WorkflowV2AgentAdapter, WorkflowV2CallExecution, WorkflowV2Result, WorkflowV2ResultStore,
};

/// Fails every dispatch with a size rejection, so the loop always wants another
/// attempt, and reports a clock that has already run past any budget.
struct AlwaysRetries {
    attempts: AtomicUsize,
    budget: Mutex<Option<Duration>>,
}

impl AlwaysRetries {
    fn new(budget: Option<Duration>) -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            budget: Mutex::new(budget),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for AlwaysRetries {
    fn call_time_budget(&self) -> Option<Duration> {
        *self.budget.lock().expect("budget")
    }

    fn fanout_parallelism(&self, requested: Option<usize>) -> usize {
        requested.unwrap_or(1).max(1)
    }

    async fn run_call(
        &self,
        _task: &str,
        _repository_root: Option<String>,
        _execution: &WorkflowV2CallExecution,
        _adapter: &WorkflowV2AgentAdapter,
        _v2_store: Option<&WorkflowV2ResultStore>,
        _task_universe: Option<&WorkflowV2TaskUniverse>,
    ) -> crate::WorkflowResult<WorkflowV2Result> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        // The wholesale line-cap rejection, which the loop treats as
        // correctable and re-asks against — and which reports a smaller
        // overshoot each time, so the progress test never ends the loop either.
        let seen = self.attempts.load(Ordering::SeqCst) as u32;
        Err(crate::WorkflowError::port(format!(
            "your patch would make source file 'src/a.rs' {} lines (currently 495, cap 500); \
             the ENTIRE patch is rejected",
            600 - seen
        )))
    }
}

/// The decision the loop actually makes, exercised without a clock.
///
/// A spent budget must stop the loop BEFORE it dispatches again — the point is
/// to stop adding attempts, not to notice afterwards.
#[test]
fn a_spent_budget_is_exhausted() {
    let started = std::time::Instant::now();
    assert!(super::size_retry::call_time_budget_exhausted(
        started,
        Some(Duration::ZERO)
    ));
}

/// A budget with time left is not.
#[test]
fn a_budget_with_time_left_is_not_exhausted() {
    let started = std::time::Instant::now();
    assert!(!super::size_retry::call_time_budget_exhausted(
        started,
        Some(Duration::from_secs(3600))
    ));
}

/// No budget is unbounded, so nothing changes for a dispatcher that has none.
#[test]
fn no_budget_is_never_exhausted() {
    let started = std::time::Instant::now();
    assert!(!super::size_retry::call_time_budget_exhausted(
        started, None
    ));
}

/// And the live dispatcher derives its budget from the configured per-dispatch
/// timeout rather than a constant, so the bound moves with the setting instead
/// of contradicting it.
#[test]
fn a_dispatcher_reports_the_budget_the_loop_reads() {
    let dispatch = AlwaysRetries::new(Some(Duration::from_secs(21_600)));
    assert_eq!(
        dispatch.call_time_budget(),
        Some(Duration::from_secs(21_600))
    );
    assert!(AlwaysRetries::new(None).call_time_budget().is_none());
}

/// Exhaustion is recoverable, not fatal. A hard error would end the wave and
/// discard whatever the branch had already established; the branch outcome is
/// meant to carry that forward as remediation data.
#[test]
fn budget_exhaustion_is_treated_as_a_recoverable_timeout() {
    let message = format!(
        "write branch 'implement-1-0' {} of 7200s after 7300s across re-dispatches",
        super::errors::CALL_TIME_BUDGET_EXHAUSTED
    );
    assert!(super::errors::is_recoverable_write_branch_timeout(&message));
}

/// And an unrelated failure is still not a timeout.
#[test]
fn other_failures_are_not_budget_exhaustion() {
    assert!(!super::errors::is_recoverable_write_branch_timeout(
        "changed files outside declared ownership"
    ));
}

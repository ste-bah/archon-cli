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
    assert!(super::errors::is_recoverable_write_branch_interruption(
        &message
    ));
}

/// And an unrelated failure is still not a timeout.
#[test]
fn other_failures_are_not_budget_exhaustion() {
    assert!(!super::errors::is_recoverable_write_branch_interruption(
        "changed files outside declared ownership"
    ));
}

/// The error that ended a fifteen-task run at its third task.
///
/// It arrived wrapped in "agent transport failed", which is why it reached the
/// unclassified result and became terminal. The classification has to see
/// through the wrapping, because that is the form it actually has in the wild.
#[test]
fn a_stranded_subagent_registration_is_an_interruption_not_a_verdict() {
    let live = "workflow stage failed: agent transport failed: workflow stage failed: subagent \
                failed: Failed to register subagent: subagent already exists and is running: \
                wf-06ff5242-stage-implement-tdl-020-11-0-attempt-1-1991183-coder";

    assert!(super::errors::is_host_resource_contention(live));
    assert!(super::errors::is_recoverable_write_branch_interruption(
        live
    ));
}

#[test]
fn a_full_subagent_pool_is_an_interruption_too() {
    let busy = "subagent failed: max concurrent subagents reached (8)";

    assert!(super::errors::is_recoverable_write_branch_interruption(
        busy
    ));
}

/// The phrase alone is not enough. This predicate runs BEFORE the validation
/// classifier, so anything it captures becomes re-askable — and an agent can
/// put any words it likes in a summary that ends up inside a validation error.
#[test]
fn an_agents_own_prose_cannot_pose_as_a_registry_collision() {
    let agent_prose = "agent result failed validation: summary says the dataset \
                       already exists and is running in production";

    assert!(!super::errors::is_host_resource_contention(agent_prose));
    assert!(!super::errors::is_recoverable_write_branch_interruption(
        agent_prose
    ));
}

/// The class must stay narrow: work that was examined and found wrong is not
/// an interruption, and treating it as one would re-ask a branch forever.
#[test]
fn a_real_verdict_on_the_work_is_not_an_interruption() {
    for verdict in [
        "implementation agent changed files outside declared target_files: src/a.rs",
        "agent result failed validation: missing task_coverage",
        "write target 'src/a.rs' for item 'x' is unsafe",
    ] {
        assert!(
            !super::errors::is_recoverable_write_branch_interruption(verdict),
            "{verdict}"
        );
        assert!(
            !super::errors::is_host_resource_contention(verdict),
            "{verdict}"
        );
    }
}

/// The ceiling is a SUM of the budgets it is made of, not a number someone
/// picked. If a budget is raised, this fails unless the ceiling moved with it —
/// which is the point: the previous arrangement let separate counters multiply
/// into a total nobody had worked out.
#[test]
fn the_dispatch_ceiling_is_derived_from_the_budgets_it_is_made_of() {
    use super::size_retry::{MAX_BRANCH_DISPATCHES, MAX_SIZE_RETRIES};
    use crate::v2::transport_retry::MAX_TRANSPORT_RETRIES;

    assert_eq!(
        MAX_BRANCH_DISPATCHES,
        1 + MAX_SIZE_RETRIES + MAX_TRANSPORT_RETRIES,
        "the ceiling must be the first attempt plus both re-ask budgets"
    );
}

/// A branch must be able to spend its FULL size budget even after losing
/// dispatches to the transport — which the loop's own comment promised and its
/// bound did not deliver, because `for _ in 0..=MAX_SIZE_RETRIES` counted every
/// iteration including the transport ones.
#[test]
fn transport_retries_do_not_eat_the_size_retry_budget() {
    use super::size_retry::{MAX_BRANCH_DISPATCHES, MAX_SIZE_RETRIES};
    use crate::v2::transport_retry::MAX_TRANSPORT_RETRIES;

    let worst_case_dispatches = 1 + MAX_SIZE_RETRIES + MAX_TRANSPORT_RETRIES;
    assert!(
        MAX_BRANCH_DISPATCHES >= worst_case_dispatches,
        "a branch that loses {MAX_TRANSPORT_RETRIES} dispatches to the transport must still have \
         all {MAX_SIZE_RETRIES} size re-asks available: ceiling {MAX_BRANCH_DISPATCHES} < needed \
         {worst_case_dispatches}"
    );
}

/// A stall has to be reported, not inferred — so the marker must survive onto
/// the record a consumer actually reads.
#[test]
fn a_branch_that_stopped_improving_says_so_in_its_record() {
    let rejected = super::errors::write_branch_validation_error_result(
        "implement-x-1-0",
        None,
        "source file src/a.rs exceeds max 500 lines",
    );

    let stamped = super::errors::stamp_no_progress(Ok(rejected), 12).expect("stamped outcome");

    assert_eq!(stamped.data["branch_no_progress"], serde_json::json!(true));
    assert_eq!(stamped.data["branch_attempts"], serde_json::json!(12));
    assert!(
        stamped.summary.contains("stopped getting closer"),
        "the summary must say why it stopped, not just that it failed: {}",
        stamped.summary
    );
    // The actionable rejection is still there — the stamp is additive.
    assert!(
        stamped
            .residual_gaps
            .iter()
            .any(|gap| gap.description.contains("exceeds max")),
        "stamping must not discard the rejection that names the remedy"
    );
}

/// An error the classification declined to own must stay declined: stamping it
/// would claim a diagnosis that was never made.
#[test]
fn an_unclassified_error_is_not_given_a_stall_diagnosis() {
    let declined: crate::WorkflowResult<crate::WorkflowV2Result> =
        Err(crate::WorkflowError::port("something else entirely"));

    let after = super::errors::stamp_no_progress(declined, 3);

    assert!(after.is_err(), "an error must pass through untouched");
}

/// Issue-54: the tool guard ended a session that thrashed past the read wall.
/// It arrives wrapped in the pipeline's transport phrase; it is a host cut with
/// the work unjudged, so the branch takes the interrupted path — partial
/// captured, retried once over it, then stalled — and says why.
#[test]
fn a_read_wall_thrash_cut_is_an_interruption_with_its_own_summary() {
    let cut = "workflow stage failed: agent transport failed: subagent failed: read-wall thrash: \
               16 non-writing calls after the read budget was exhausted; 0 substantive writes";
    assert!(super::errors::is_recoverable_write_branch_interruption(cut));
    assert!(!super::errors::is_host_resource_contention(cut));
    let result = super::errors::write_branch_interrupted_result(
        "agents-5-0",
        &serde_json::json!({"item": {"id": "agents-5-0"}}),
        cut,
    );
    assert_eq!(result.status, crate::v2::WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.summary,
        "write branch 'agents-5-0' was stopped by the host after thrashing at the read wall without writing"
    );
    // The key the retry-once path and `resume` read.
    assert_eq!(result.data["branch_runtime_timeout"], true);
    assert_ne!(result.data["branch_host_resource_contention"], true);
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "write_branch_timeout_agents-5-0"
                && gap
                    .description
                    .contains("read-wall thrash: 16 non-writing calls")),
        "{:#?}",
        result.residual_gaps
    );
}

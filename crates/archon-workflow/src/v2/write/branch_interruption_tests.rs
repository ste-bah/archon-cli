//! Classification of write-branch interruptions. Branch progress tests cover
//! repeated dispatch behavior; these tests distinguish interruptions from
//! implementation verdicts.

/// And an unrelated failure is still not a timeout.
#[test]
fn other_failures_are_not_recoverable_interruptions() {
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

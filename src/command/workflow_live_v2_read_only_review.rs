//! REM-5: a review map re-runs the branches that ended without a verdict
//! before the map is done, for as long as re-running them makes progress.
//!
//! A review branch that failed (a timeout, a dropped transport, an output
//! the host rejected) leaves its task with no reviewer verdict; the host
//! records that as an `unreviewed` finding, which only the terminal rule
//! acted on -- by holding the run. The host already re-asks a failed review
//! branch once inside its own attempt (`retry`); this re-runs, as a further
//! pass of the SAME map call, every branch a pass left incomplete, and keeps
//! going while each pass completes at least one of them. A pass that
//! completes none is a plateau: the loop ends, and whatever is still
//! incomplete is recorded unreviewed exactly as before. No fixed count.
//!
//! Only a call whose review contract names the `map` stage loops; every
//! other read-only fan-out runs one pass, as it always has. Everything is
//! the same call: the re-run branches keep their ids and inputs, their
//! outcomes replace the incomplete ones, and the map's result, attachment
//! and roster are built from the final outcomes.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use super::review_tree::ReviewTreeTripwire;

use archon_workflow::v2::review_findings::incomplete_review_branch_ids;
use archon_workflow::v2::scheduler::{WorkflowV2FanoutItem, WorkflowV2FanoutReport};

/// Run `items` through `pass`; for a review map, re-run the incomplete
/// branches while each pass completes at least one. `before_rerun` runs
/// ahead of every further pass (the host's pause/cancel poll) and ends the
/// loop with its error.
pub(super) async fn run_review_passes<P, Fut, B>(
    items: Vec<WorkflowV2FanoutItem>,
    review_map: bool,
    mut pass: P,
    mut before_rerun: B,
) -> archon_workflow::WorkflowResult<WorkflowV2FanoutReport>
where
    P: FnMut(Vec<WorkflowV2FanoutItem>) -> Fut,
    Fut: Future<Output = archon_workflow::WorkflowResult<WorkflowV2FanoutReport>>,
    B: FnMut(&[String]) -> archon_workflow::WorkflowResult<()>,
{
    let mut report = pass(items.clone()).await?;
    if !review_map {
        return Ok(report);
    }
    let mut incomplete = incomplete_review_branch_ids(&report.outcomes);
    while !incomplete.is_empty() && !report.cancelled {
        before_rerun(&incomplete)?;
        let rerun: Vec<WorkflowV2FanoutItem> = items
            .iter()
            .filter(|item| incomplete.contains(&item.id))
            .cloned()
            .collect();
        if rerun.is_empty() {
            break;
        }
        let again = pass(rerun).await?;
        let still = incomplete_review_branch_ids(&again.outcomes);
        let progressed = incomplete.iter().any(|id| {
            again.outcomes.iter().any(|outcome| &outcome.item_id == id) && !still.contains(id)
        });
        report.peak_parallelism = report.peak_parallelism.max(again.peak_parallelism);
        report.cancelled |= again.cancelled;
        for outcome in again.outcomes {
            match report
                .outcomes
                .iter_mut()
                .find(|existing| existing.item_id == outcome.item_id)
            {
                Some(existing) => *existing = outcome,
                None => report.outcomes.push(outcome),
            }
        }
        if !progressed {
            break;
        }
        incomplete = still;
    }
    Ok(report)
}

/// Major 2: whether review branches get a shell, and the tripwire they then
/// run under. A shell only when the platform can put the OS write boundary
/// under it, a boundary scope is drawn for the checkout, and the tripwire
/// armed over every root the run's records name (`review_roots`); any one
/// missing, no shell. The tripwire spills under the run's own store.
pub(super) fn review_shell(
    v2_store: &archon_workflow::WorkflowV2ResultStore,
    root: Option<&str>,
    universe: Option<&archon_workflow::task_universe::WorkflowV2TaskUniverse>,
    call_id: &str,
) -> (bool, Option<Arc<ReviewTreeTripwire>>) {
    let context = archon_workflow::project_artifact_context_from_v2_root(v2_store.root());
    let scoped = super::super::workflow_live_v2_call_boundary::read_only_boundary(
        Some(v2_store),
        root,
        context.project_root.as_deref(),
        root,
    )
    .is_some();
    if root.is_none()
        || !shell_allowed(archon_tools::bash::shell_write_boundary_available(), scoped)
    {
        return (false, None);
    }
    let watch = super::review_roots::review_roots(
        v2_store.run_root(),
        context.project_root.as_deref().map(Path::new),
        &context.artifact_roots,
        root.map(Path::new),
        universe,
    );
    let spill = v2_store.root().join("review-tripwire").join(
        call_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>(),
    );
    let tripwire = ReviewTreeTripwire::arm(watch, &spill).map(Arc::new);
    (tripwire.is_some(), tripwire)
}

/// The OS boundary is available AND a boundary scope is in effect.
pub(super) fn shell_allowed(boundary_available: bool, scope_drawn: bool) -> bool {
    boundary_available && scope_drawn
}

#[cfg(test)]
#[path = "workflow_live_v2_read_only_review_tests.rs"]
mod tests;

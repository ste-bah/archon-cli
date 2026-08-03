//! The pre-run gate: a structural knob setting is admitted or refused *before*
//! the run, by the same lint analyses `archon workflow lint` reports with.
//!
//! # Why a gate and not a check afterwards
//!
//! A budget knob that is wrong costs wall-clock and is visible in the run's own
//! failure. A structural knob that is wrong changes what ran concurrently, and
//! by the time that shows up as a corrupted worktree or a spuriously failed
//! verification the run has already spent its money and the evidence for *why*
//! is gone. So the proposal is scored against the graph first, and a proposal
//! the graph cannot support never reaches the lifecycle.
//!
//! # Two graphs, because the knob makes two different claims
//!
//! Narrowing the implementation fan-out is a claim about the **declared task
//! graph** — how much of this PRD may honestly run at once. Doing it *at all*
//! is a claim about the **plan shape** — that the run still has the review
//! diamond the knob is allowed to operate inside. Both are checked, and either
//! one refuses.
//!
//! Refusal always means "hold the operator's configured cap". Never "pick
//! something in between": a width justified by a graph the lints could not
//! vouch for is a number with no argument behind it, and the configured cap at
//! least has an author.

use std::path::Path;

use archon_core::config::ShapeDecision;
use archon_topology::ir::{FanoutSpec, NodeRole, TaskGraph, TaskNode};
use archon_topology::{DiamondFinding, GraphOrigin};
use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostMethod};

/// Stage families that form the review diamond the knob operates inside.
///
/// Read out of the plan rather than restated, so a plan edit that moved
/// `adversarial-review` back to a terminal reduce would change the graph these
/// produce and be caught by [`TaskGraph::diamond_conformance`] — by the
/// analysis, not by a string comparison that someone could update to match.
const FANOUT_FAMILY: &str = "implementation-wave";
const VERIFY_FAMILY: &str = "verification-wave";
const REVIEW_FAMILY: &str = "adversarial-review";
const FOLD_FAMILY: &str = "cross-cutting-review";

/// Tier names the lifecycle dispatches these families under.
///
/// The IR scores verifier diversity on `agent`, and the plan declares tiers
/// rather than agent names — `verification-wave` runs at `coder` and
/// `adversarial-review` at `critic`, whose candidate list is a different set of
/// agents entirely. Using the tier is the honest lowering: it is what actually
/// distinguishes the two reviewers, and it is stable across agent registries in
/// a way a resolved agent name is not.
const VERIFY_TIER: &str = "coder";
const REVIEW_TIER: &str = "critic";

/// What the gate concluded, for the report. A refusal carries its own sentence
/// because the operator has to be able to act on it without re-running a lint
/// by hand.
pub(crate) enum GateOutcome {
    /// Both graphs support the proposal at this width.
    Admitted,
    /// The proposal is withdrawn. The string names the lint and what it found.
    Refused(String),
}

/// Score a proposed width against the plan shape and the declared task graph,
/// mutating `decision` in place.
///
/// Does nothing at all when the decision did not move: constraint 1 says a knob
/// with no evidence holds its default, and a lint that tightened an *unmoved*
/// default would be changing behaviour on the path where nothing was learned —
/// the one path that must stay byte-identical.
pub(crate) fn admit(
    decision: &mut ShapeDecision,
    plan_calls: &[WorkflowV2HostCall],
    tasks_root: Option<&Path>,
) -> GateOutcome {
    if !decision.source.moved() {
        return GateOutcome::Admitted;
    }
    if let Some(reason) = refuse_reason(plan_calls, tasks_root) {
        decision.refuse(reason.clone());
        return GateOutcome::Refused(reason);
    }
    if let Some(widest) = widest_declared_wave(tasks_root) {
        // A width above the widest wave the declared graph ever offers is
        // unreachable: the extra branches would never have anything to run.
        // Tightening to it makes the reported number the one the run will
        // actually use, which is the number the operator has to be able to
        // check against the wave layering.
        decision.tighten_to(widest);
    }
    GateOutcome::Admitted
}

/// The first reason to withdraw the proposal, or `None` when both graphs
/// support it.
fn refuse_reason(plan_calls: &[WorkflowV2HostCall], tasks_root: Option<&Path>) -> Option<String> {
    if let Some(reason) = plan_shape_refusal(plan_calls) {
        return Some(reason);
    }
    declared_graph_refusal(tasks_root)
}

/// The plan shape must still be the per-task review diamond.
///
/// Constraint: no knob setting may run inside a plan that has lost its earned
/// failure handling. The diamond is the part of that handling a *structural*
/// knob could plausibly interact with — everything else in the plan
/// (`blocked-*`, the repair loops, noop proofs, evidence reconciliation, the
/// final zero-gap audit) is unreachable from a fan-out width, and is checked
/// separately by [`required_failure_handling_is_intact`].
fn plan_shape_refusal(plan_calls: &[WorkflowV2HostCall]) -> Option<String> {
    if let Some(missing) = required_failure_handling_is_intact(plan_calls) {
        return Some(format!(
            "the run's plan is missing the '{missing}' stage family. Narrowing the fan-out \
             would change how work is distributed inside a plan that has lost part of its \
             earned failure handling, so the configured cap is kept and the plan is the \
             thing to fix"
        ));
    }
    let graph = review_diamond_graph(plan_calls)?;
    let report = graph
        .diamond_conformance()
        .map_err(|error| format!("{error}"))
        .ok()?;
    if let Some(finding) = report.findings.first() {
        return Some(format!(
            "diamond conformance on the plan's review shape reported {}: {}. A structural knob \
             must not act on a plan whose review diamond is already broken",
            finding_label(finding),
            finding.remedy()
        ));
    }
    // "No findings" and "nothing to check" are different answers, and the
    // analysis distinguishes them: `diversity` is empty exactly when no reducer
    // has any verification feeding it, which on this graph means the fan-out
    // reaches no fold at all. A plan whose implementation wave is never folded
    // has no diamond for the knob to be operating inside, and reading the empty
    // findings list as a clean bill of health is precisely the mistake the lint
    // suite's own report format goes out of its way to prevent.
    report.diversity.is_empty().then(|| {
        format!(
            "diamond conformance found no reduce stage with verification feeding it — the \
             plan's '{FANOUT_FAMILY}' is never folded, so there is no review diamond for a \
             structural knob to run inside. An empty findings list here means the lint had \
             nothing to look at, not that it looked and approved"
        )
    })
}

fn finding_label(finding: &DiamondFinding) -> &'static str {
    match finding {
        DiamondFinding::UnverifiedFanout { .. } => "an unverified fan-out",
        DiamondFinding::SoleVerifier { .. } => "a sole verifier",
        DiamondFinding::HomogeneousVerifiers { .. } => "homogeneous verifiers",
    }
}

/// The declared dependency graph must be one a width claim can be read off.
///
/// Two conditions, both computed by the shipped lints:
///
/// - It must **validate**. A cycle or an unknown dependency id means there are
///   no waves, so "how many tasks may run at once" has no answer. That case
///   belongs to `dependency-graph-repair` and `blocked-dependency-deadlock`; a
///   narrowed fan-out would mask a deadlock by never running two tasks together
///   and let the run limp instead of reporting.
/// - It must have **no unsupported edges**. An unsupported edge is a declared
///   dependency carrying no dataflow — the "fake edge" the lint is named for.
///   Wave layering computed over fake edges under-reports what may run
///   concurrently, so a width derived from it is a claim about a graph that is
///   not the one being executed.
fn declared_graph_refusal(tasks_root: Option<&Path>) -> Option<String> {
    let graph = declared_graph(tasks_root)?;
    if let Err(error) = graph.validate() {
        return Some(format!(
            "the declared dependency graph does not validate ({error}). Wave width has no \
             meaning on a graph with no waves, and dependency-graph-repair owns this case"
        ));
    }
    let unsupported = graph.unsupported_edges().ok()?;
    let first = unsupported.first()?;
    Some(format!(
        "{} unsupported (fake) edge(s) in the declared dependency graph, e.g. {} -> {}: {}. \
         A width computed from wave layering over fake edges is a claim about a graph that \
         is not the one being run",
        unsupported.len(),
        first.dependent,
        first.dependency,
        first.remedy()
    ))
}

/// The widest wave the declared graph offers, or `None` when there is no graph
/// to read. Fails open to "no ceiling" rather than inventing one.
fn widest_declared_wave(tasks_root: Option<&Path>) -> Option<u32> {
    let graph = declared_graph(tasks_root)?;
    let waves = graph.waves().ok()?;
    let widest = waves.iter().map(Vec::len).max()?;
    u32::try_from(widest).ok()
}

fn declared_graph(tasks_root: Option<&Path>) -> Option<TaskGraph> {
    let root = tasks_root?;
    crate::command::workflow_live::workflow_live_task_universe::task_graph_from_root(root).ok()
}

/// Stage families whose loss would mean the run can no longer fail honestly.
///
/// Not the full 60-odd plan: these are the families the issue names as earned
/// failure handling — the blocked terminals, the repair loops, noop-proof
/// verification, evidence reconciliation, dependency-deadlock detection, and
/// the final zero-gap audit. A knob that cannot remove them still has to prove
/// it did not, because "it cannot" is a property of today's implementation and
/// this list is what would notice tomorrow's.
const REQUIRED_FAILURE_HANDLING: &[&str] = &[
    "dependency-graph-repair",
    "dependency-graph-repair-deadlock",
    "blocked-dependency-deadlock",
    "noop-proof-verification",
    "noop-proof-reverification",
    "blocked-noop-proof-failed",
    "evidence-repair",
    "remediation-outcome-repair",
    "blocked-remediation-unresolved",
    "blocked-verification-failed",
    "blocked-loop-exhaustion",
    "final-evidence-reconciliation",
    "blocked-final-evidence-reconciliation",
    "final-zero-gap-audit",
    "final-acceptance-gate",
];

/// The first required family the plan no longer declares, or `None`.
pub(crate) fn required_failure_handling_is_intact(
    plan_calls: &[WorkflowV2HostCall],
) -> Option<&'static str> {
    REQUIRED_FAILURE_HANDLING
        .iter()
        .find(|family| !plan_calls.iter().any(|call| call.id == **family))
        .copied()
}

/// Lower the plan's review diamond into the topology IR.
///
/// Returns `None` when the plan does not declare all four families — a plan
/// that is not the decomposed-PRD plan is not one this gate has an opinion
/// about, and inventing nodes for the missing ones would produce a report about
/// a graph nobody wrote.
///
/// Roles come from the plan's own declared methods, which is the whole point:
/// flipping `adversarial-review` from `PARALLEL` back to `REDUCE` turns it from
/// a Verify node into a Reduce node, `frontier_verifiers` then finds one
/// verifier feeding the fold instead of two, and `diamond_conformance` reports
/// `SoleVerifier`. That shape is refused by the analysis rather than by a
/// hardcoded veto — which matters, because a hardcoded veto is a line someone
/// can delete without understanding it.
pub(crate) fn review_diamond_graph(plan_calls: &[WorkflowV2HostCall]) -> Option<TaskGraph> {
    let method = |family: &str| {
        plan_calls
            .iter()
            .find(|call| call.id == family)
            .map(|call| call.method)
    };
    let fanout_method = method(FANOUT_FAMILY)?;
    let verify_method = method(VERIFY_FAMILY)?;
    let review_method = method(REVIEW_FAMILY)?;
    let fold_method = method(FOLD_FAMILY)?;

    let mut nodes = vec![TaskNode {
        fanout: (fanout_method == WorkflowV2HostMethod::Fanout).then(|| FanoutSpec {
            source: None,
            // The declared shape, not this run's width: the lint scores whether
            // the diamond exists, and a per-run number would make the same plan
            // lint differently on two runs.
            max_parallelism: None,
        }),
        ..TaskNode::new(FANOUT_FAMILY, role_for(fanout_method))
    }];
    nodes.push(TaskNode {
        depends_on: vec![FANOUT_FAMILY.to_string()],
        agent: Some(VERIFY_TIER.to_string()),
        ..TaskNode::new(VERIFY_FAMILY, role_for(verify_method))
    });
    nodes.push(TaskNode {
        depends_on: vec![VERIFY_FAMILY.to_string()],
        agent: Some(REVIEW_TIER.to_string()),
        ..TaskNode::new(REVIEW_FAMILY, role_for(review_method))
    });
    nodes.push(TaskNode {
        depends_on: vec![REVIEW_FAMILY.to_string()],
        ..TaskNode::new(FOLD_FAMILY, role_for(fold_method))
    });

    Some(TaskGraph {
        nodes,
        ..TaskGraph::new(
            "generated-plan-review-diamond",
            GraphOrigin::Workflow {
                run_id: "generated-plan".to_string(),
            },
        )
    })
}

/// Host method to IR role.
///
/// `Parallel` is `Verify` because every `parallel` family in this plan is a
/// read-only inspection stage — verification, noop proof, review, artifact
/// existence. `Fanout` is `Work` because it is the only write-capable family.
/// Anything else that reaches this function folds branches or terminates, and
/// `Reduce` is the honest role for both.
fn role_for(method: WorkflowV2HostMethod) -> NodeRole {
    match method {
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Implementation => NodeRole::Work,
        WorkflowV2HostMethod::Parallel => NodeRole::Verify,
        _ => NodeRole::Reduce,
    }
}

#[cfg(test)]
#[path = "sona_workflow_shape_gate_tests.rs"]
mod tests;

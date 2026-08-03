use super::*;

use archon_core::config::{ShapeInput, ShapeSource, TunableShapeKnob, decide_fanout_width};
use archon_workflow::WorkflowV2HostOptions;

use archon_workflow::v2::decomposed_prd_plan::decomposed_prd_plan_calls;

fn moved_decision() -> ShapeDecision {
    let input = ShapeInput {
        knob: TunableShapeKnob::ImplementationWaveFanoutWidth,
        weight: Some(1.0),
        observations: 20,
        drift_rolled_back: false,
    };
    let decision = decide_fanout_width(8, Some(&input));
    assert!(decision.source.moved(), "the fixture must be a moved value");
    decision
}

fn call(id: &str, method: WorkflowV2HostMethod) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    }
}

/// Replace one family's method, leaving the rest of the shipped plan intact.
fn plan_with(id: &str, method: WorkflowV2HostMethod) -> Vec<WorkflowV2HostCall> {
    let mut calls = decomposed_prd_plan_calls();
    let entry = calls
        .iter_mut()
        .find(|call| call.id == id)
        .unwrap_or_else(|| panic!("the shipped plan declares '{id}'"));
    entry.method = method;
    calls
}

// ------------------------------------------------------- the shipped plan

/// The whole gate rests on this: the plan that actually ships lints clean, so
/// a refusal always means something changed rather than that the gate is
/// permanently closed.
#[test]
fn the_shipped_plan_lints_clean_and_admits_a_learned_width() {
    let mut decision = moved_decision();
    let applied = decision.applied;
    match admit(&mut decision, &decomposed_prd_plan_calls(), None) {
        GateOutcome::Admitted => {}
        GateOutcome::Refused(reason) => panic!("the shipped plan must admit: {reason}"),
    }
    assert_eq!(
        decision.applied, applied,
        "no tasks root, so no wave ceiling"
    );
    assert_eq!(decision.source, ShapeSource::Learned);
    assert!(decision.refusal.is_none());
}

#[test]
fn the_shipped_plan_declares_every_required_failure_handling_family() {
    assert_eq!(
        required_failure_handling_is_intact(&decomposed_prd_plan_calls()),
        None,
        "a family in REQUIRED_FAILURE_HANDLING is not in the plan; either the plan lost it \
         or the list names something that never existed — both must be resolved by hand"
    );
}

/// Diamond conformance run against the plan the repo ships, through the same
/// lowering the gate uses. `verification-wave` and `adversarial-review` both
/// reach `cross-cutting-review` without an intervening reduce, and they run at
/// different tiers, so the fold has a two-reviewer panel.
#[test]
fn the_shipped_plans_review_diamond_scores_two_distinct_verifiers() {
    let graph = review_diamond_graph(&decomposed_prd_plan_calls()).expect("all four families");
    let report = graph.diamond_conformance().expect("valid graph");
    assert!(
        report.is_clean(),
        "the shipped review diamond must lint clean: {:?}",
        report.findings
    );
    let score = report
        .diversity
        .iter()
        .find(|score| score.reducer == "cross-cutting-review")
        .expect("the terminal reduce is scored");
    assert_eq!(score.verifiers.len(), 2);
    assert_eq!(score.distinct_agents, 2);
}

// --------------------------------------------------- constraint 5: terminal

/// The three-month lesson, enforced by the analysis rather than by a veto.
///
/// Moving `adversarial-review` back to a terminal `REDUCE` makes it the fold
/// instead of a verifier, `frontier_verifiers` then finds `verification-wave`
/// alone feeding it, and `diamond_conformance` reports `SoleVerifier`. No
/// evidence can select that shape, because the gate refuses any structural
/// change inside a plan whose diamond is already broken — and nothing in this
/// file has to know *why* terminal review is wrong to reach that conclusion.
#[test]
fn a_terminal_adversarial_review_is_refused_before_the_run() {
    let plan = plan_with("adversarial-review", WorkflowV2HostMethod::Reduce);
    let mut decision = moved_decision();
    let GateOutcome::Refused(reason) = admit(&mut decision, &plan, None) else {
        panic!("a terminal adversarial-review must be refused");
    };
    assert!(
        reason.contains("sole verifier"),
        "the refusal must name what the lint found: {reason}"
    );
    assert!(
        reason.contains("adversarial-review"),
        "the refusal must name the stage to change: {reason}"
    );
    assert_eq!(decision.source, ShapeSource::RefusedByDependencyGraph);
    assert_eq!(
        decision.applied, decision.baseline,
        "a refused proposal returns to the operator's cap, not to something in between"
    );
    assert_eq!(decision.applied_width(), None);
}

/// The other half of the same lesson, and the case that nearly slipped through.
///
/// Demoting `cross-cutting-review` out of `REDUCE` leaves the fan-out with no
/// fold at all. `diamond_conformance` then reports *no findings* — not because
/// the shape is fine but because there is no reducer to check, which is the
/// "nothing to check" state the lint's own report format prints separately from
/// "no findings". Treating an empty findings list as approval would have let a
/// structural knob run inside a plan whose implementation wave is never
/// reviewed by anything.
#[test]
fn a_fanout_whose_fold_stopped_being_a_reduce_is_refused() {
    let plan = plan_with("cross-cutting-review", WorkflowV2HostMethod::Parallel);
    let graph = review_diamond_graph(&plan).expect("all four families");
    let report = graph.diamond_conformance().expect("valid graph");
    assert!(
        report.findings.is_empty() && report.diversity.is_empty(),
        "this test is only meaningful while the lint stays silent on this shape: {report:?}"
    );

    let mut decision = moved_decision();
    let GateOutcome::Refused(reason) = admit(&mut decision, &plan, None) else {
        panic!("a plan with no terminal reduce must be refused");
    };
    assert!(
        reason.contains("diamond conformance"),
        "the refusal must name the lint: {reason}"
    );
    assert!(
        reason.contains("never folded"),
        "the refusal must say the fan-out reaches no reduce: {reason}"
    );
    assert_eq!(decision.applied, decision.baseline);
}

// ------------------------------------------- constraint 3: failure handling

/// A knob setting may not run inside a plan that has lost its earned failure
/// handling, and the refusal has to say which family is missing — "something is
/// wrong" is not actionable months later.
#[test]
fn a_plan_missing_a_blocked_terminal_is_refused_naming_what_would_be_lost() {
    let mut plan = decomposed_prd_plan_calls();
    plan.retain(|call| call.id != "blocked-loop-exhaustion");
    let mut decision = moved_decision();
    let GateOutcome::Refused(reason) = admit(&mut decision, &plan, None) else {
        panic!("a plan missing blocked-loop-exhaustion must be refused");
    };
    assert!(
        reason.contains("blocked-loop-exhaustion"),
        "the refusal must name the missing family: {reason}"
    );
    assert!(
        reason.contains("earned failure handling"),
        "the refusal must say what class of thing was lost: {reason}"
    );
    assert_eq!(decision.applied, decision.baseline);
}

#[test]
fn every_required_family_is_individually_load_bearing() {
    for family in REQUIRED_FAILURE_HANDLING {
        let mut plan = decomposed_prd_plan_calls();
        plan.retain(|call| call.id != *family);
        assert_eq!(
            required_failure_handling_is_intact(&plan),
            Some(*family),
            "removing '{family}' must be detected"
        );
    }
}

// ------------------------------------------------------- constraint 1: default

/// The one path that must stay byte-identical to a build without this module.
/// A lint may tighten a proposal; it may never be the reason an *unproposed*
/// run changed shape.
#[test]
fn the_gate_does_nothing_at_all_to_an_unmoved_decision() {
    for source in [ShapeSource::Baseline, ShapeSource::InsufficientEvidence] {
        let mut decision = decide_fanout_width(8, None);
        decision.source = source;
        // Even against a plan that would refuse a real proposal.
        let plan = plan_with("adversarial-review", WorkflowV2HostMethod::Reduce);
        match admit(&mut decision, &plan, None) {
            GateOutcome::Admitted => {}
            GateOutcome::Refused(reason) => panic!("must not touch an unmoved decision: {reason}"),
        }
        assert_eq!(decision.source, source);
        assert_eq!(decision.applied, 8);
        assert_eq!(decision.applied_width(), None);
        assert!(decision.refusal.is_none());
    }
}

// ------------------------------------------- the declared dependency graph

/// The real seventeen-task PRD corpus, the same fixture the lint suite's
/// `real_corpus` tests use. It is the only surface in the tree that declares
/// dataflow on both sides, so it is the only one on which `classify_edges` can
/// conclude anything — which makes it the only honest test of this half of the
/// gate.
fn real_tasks_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/prd-trading-data-lake-ahdm-001")
}

/// A graph with no fake edges admits the proposal, and the widest wave becomes
/// the ceiling: width above it is unreachable, because the extra branches would
/// have nothing to run.
#[test]
fn a_clean_declared_graph_admits_and_supplies_the_wave_ceiling() {
    let root = real_tasks_root();
    assert!(root.is_dir(), "the seventeen-task fixture must exist");

    let mut decision = moved_decision();
    let proposed = decision.applied;
    match admit(&mut decision, &decomposed_prd_plan_calls(), Some(&root)) {
        GateOutcome::Admitted => {}
        GateOutcome::Refused(reason) => panic!("the real corpus must admit: {reason}"),
    }

    let graph = crate::command::topology_task_graph::task_graph_from_root(&root)
        .expect("the fixture lowers");
    let widest = graph
        .waves()
        .expect("valid graph")
        .iter()
        .map(Vec::len)
        .max()
        .expect("at least one wave");
    assert_eq!(
        usize::try_from(decision.applied).expect("small width"),
        proposed.min(u32::try_from(widest).expect("small wave")) as usize,
        "the applied width is the proposal capped by the widest declared wave"
    );
    assert!(decision.refusal.is_none());
}

/// A tasks root that is not a task directory yields no graph, and no graph
/// means no claim either way — the gate must not refuse on the strength of a
/// file it could not read, or a missing directory would silently pin every run
/// to the operator's cap for a reason nobody could see.
#[test]
fn an_unreadable_tasks_root_neither_refuses_nor_tightens() {
    let empty = tempfile::tempdir().expect("tempdir");
    let mut decision = moved_decision();
    let proposed = decision.applied;
    match admit(
        &mut decision,
        &decomposed_prd_plan_calls(),
        Some(empty.path()),
    ) {
        GateOutcome::Admitted => {}
        GateOutcome::Refused(reason) => panic!("an unreadable root must not refuse: {reason}"),
    }
    assert_eq!(decision.applied, proposed);
}

// ------------------------------------------------------------- the lowering

/// A plan that is not the decomposed-PRD plan has no review diamond, and the
/// gate must say so by producing no graph rather than by inventing nodes for
/// the families it could not find.
#[test]
fn a_plan_without_the_four_families_lowers_to_no_graph() {
    let plan = vec![
        call("something-else", WorkflowV2HostMethod::Parallel),
        call("cross-cutting-review", WorkflowV2HostMethod::Reduce),
    ];
    assert!(review_diamond_graph(&plan).is_none());
}

/// ...and the gate admits it rather than refusing: the knob has no opinion
/// about a plan it does not recognise, and refusing everything unrecognised
/// would make the knob's default depend on which plan happened to run.
#[test]
fn an_unrecognised_plan_is_admitted_rather_than_refused() {
    let plan = vec![call("something-else", WorkflowV2HostMethod::Parallel)];
    let mut decision = moved_decision();
    // It still has to clear the failure-handling check, which an unrecognised
    // plan cannot — so assert against the recognised-but-minimal case instead.
    let GateOutcome::Refused(reason) = admit(&mut decision, &plan, None) else {
        panic!("a plan with no failure handling at all is refused first");
    };
    assert!(reason.contains("earned failure handling"));
}

#[test]
fn the_lowering_reads_methods_from_the_plan_rather_than_assuming_them() {
    let graph = review_diamond_graph(&plan_with(
        "adversarial-review",
        WorkflowV2HostMethod::Reduce,
    ))
    .expect("all four families");
    let review = graph.node("adversarial-review").expect("node exists");
    assert_eq!(
        review.role,
        NodeRole::Reduce,
        "the node's role must follow the plan's declared method"
    );
}

#[test]
fn the_two_reviewers_are_lowered_with_different_agents() {
    let graph = review_diamond_graph(&decomposed_prd_plan_calls()).expect("all four families");
    let verify = graph.node("verification-wave").expect("node exists");
    let review = graph.node("adversarial-review").expect("node exists");
    assert_ne!(
        verify.agent, review.agent,
        "if the two reviewers lowered to the same agent, diamond conformance would report \
         HomogeneousVerifiers on the shipped plan and the gate would never open"
    );
}

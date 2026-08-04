//! Rendering the report as text.
//!
//! Two rules shape every line here.
//!
//! **Nothing below `Exercised` is printed as satisfied.** Not as a tick, not as
//! a percentage that rounds up, not as a "mapped" column. F1 was accepted
//! because a report said 93 requirements were mapped; this one says what each
//! one has actually earned, and prints the missing half beside it.
//!
//! **Every gap names the specific absent fact.** PRD §32 forbids "should work",
//! "later", "TBD" and "best effort" without stating what fails closed, so a gap
//! line here is always of the form *"anchor X: declared verifier Y passed but
//! the trace never read Z"* rather than a shrug.

use std::fmt::Write as _;

use archon_knowledge::traceability::report::AnchorVerdict;
use archon_knowledge::traceability::{
    AnchorFreshness, FalsificationOutcome, FalsificationPlan, ProofLevel, ReadScope,
    RequirementRow, Severity, TraceReport,
};

/// The whole report, as text.
pub(super) fn report(report: &TraceReport) -> String {
    let mut out = String::new();
    header(&mut out, report);
    coverage(&mut out, report);
    rows(&mut out, report);
    shared_anchors(&mut out, report);
    falsification(&mut out, report);
    out
}

fn header(out: &mut String, report: &TraceReport) {
    let _ = writeln!(out, "Requirement traceability");
    let _ = writeln!(out, "  PRD:   {}", report.prd_path);
    let _ = writeln!(out, "  Tasks: {}", report.task_dir);
    if !report.index_consulted {
        let _ = writeln!(
            out,
            "  Code index: NOT CONSULTED (--leann-db not given). Every requirement is \
             Unproven for want of an anchor, not for want of code."
        );
    }
    let counts = report.level_counts();
    let _ = writeln!(
        out,
        "\n  {}",
        counts
            .iter()
            .map(|(level, n)| format!("{level}: {n}"))
            .collect::<Vec<_>>()
            .join("   ")
    );
    if report.stale_anchors > 0 {
        let _ = writeln!(
            out,
            "  {} anchor(s) stale: their file changed since anchoring, so the recorded \
             line range is no longer trustworthy. Re-index out of band.",
            report.stale_anchors
        );
    }
    let _ = writeln!(out, "\n  {}\n", report.gate_verdict());
}

fn coverage(out: &mut String, report: &TraceReport) {
    let coverage = &report.coverage;
    let _ = writeln!(out, "Coverage (from the tasks' explicit `implements:`)");
    let _ = writeln!(
        out,
        "  {} requirements in the PRD, {} distinct IDs cited across tasks",
        coverage.requirements_total, coverage.citations_total
    );

    if coverage.phantom.is_empty() {
        let _ = writeln!(out, "  every cited ID exists in the PRD");
    } else {
        let _ = writeln!(
            out,
            "  {} phantom citation(s) — cited but absent from the PRD:",
            coverage.phantom.len()
        );
        for phantom in &coverage.phantom {
            let _ = writeln!(
                out,
                "    {} cites {} ({})",
                phantom.task_id, phantom.cited_id, phantom.source_path
            );
        }
    }

    if coverage.unclaimed.is_empty() {
        let _ = writeln!(out, "  every requirement is claimed by at least one task");
    } else {
        let _ = writeln!(
            out,
            "  {} unclaimed requirement(s) — a decomposition gap, reported not invented:",
            coverage.unclaimed.len()
        );
        for id in &coverage.unclaimed {
            let _ = writeln!(out, "    {id}");
        }
    }
    if !coverage.multiply_claimed.is_empty() {
        let _ = writeln!(
            out,
            "  {} requirement(s) claimed by more than one task: {}",
            coverage.multiply_claimed.len(),
            coverage.multiply_claimed.join(", ")
        );
    }
    let _ = writeln!(out);
}

fn rows(out: &mut String, report: &TraceReport) {
    let _ = writeln!(out, "Per requirement");
    for row in &report.rows {
        let _ = writeln!(
            out,
            "\n  {}  [{}]{}",
            row.requirement_id,
            row.level.as_str(),
            severity_note(row)
        );
        if row.claimed_by.is_empty() {
            let _ = writeln!(out, "    claimed by: (nothing)");
        } else {
            let _ = writeln!(out, "    claimed by: {}", row.claimed_by.join(", "));
        }
        for verdict in &row.anchors {
            anchor_line(out, verdict);
        }
        if row.level.satisfies_promotion_gate() {
            continue;
        }
        for reason in row.missing_reasons() {
            let _ = writeln!(out, "    missing: {reason}");
        }
    }
    let _ = writeln!(out);
}

fn severity_note(row: &RequirementRow) -> String {
    match (row.severity, &row.severity_evidence) {
        (Severity::Error, Some(phrase)) => {
            format!("  severity=error (matched `{phrase}`)")
        }
        (Severity::Error, None) => "  severity=error".to_string(),
        (Severity::Unclassified, _) => String::new(),
    }
}

fn anchor_line(out: &mut String, verdict: &AnchorVerdict) {
    let stale = match &verdict.freshness {
        AnchorFreshness::Fresh => String::new(),
        AnchorFreshness::Stale { .. } => "  STALE".to_string(),
        AnchorFreshness::FileMissing => "  FILE MISSING".to_string(),
    };
    let _ = writeln!(
        out,
        "    anchor: {}  ({}, scope {}){stale}",
        verdict.anchor.citation(),
        verdict.level.as_str(),
        verdict.anchor.path_scope,
    );
    if let Some(proof) = &verdict.proof {
        let scope = match &proof.read_scope {
            ReadScope::Node(node) => format!("read by node {node}"),
            // Named rather than smoothed over: a run-scoped read says the run
            // touched the file, not that this task did.
            ReadScope::Run => "read somewhere in the run (weaker than node-scoped)".to_string(),
        };
        let _ = writeln!(
            out,
            "      exercised by `{}` [{:?}] — {scope}",
            proof.command, proof.origin
        );
    }
}

fn shared_anchors(out: &mut String, report: &TraceReport) {
    if report.shared_anchors.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "Evidence reuse — one span cited by several requirements. This is the shape of \
         finding F1 (`repeated generic evidence for REQ-DL-001..004`). One function \
         satisfying two requirements is real; one span answering for many is not."
    );
    for shared in &report.shared_anchors {
        let _ = writeln!(
            out,
            "  {}  cited by {}: {}",
            shared.citation,
            shared.requirement_ids.len(),
            shared.requirement_ids.join(", ")
        );
    }
    let _ = writeln!(out);
}

/// The falsification section.
///
/// Every line that mentions an outcome is guarded on there being one. Without
/// `--falsify` no plan has an outcome, so this renders exactly what it rendered
/// before execution existed — the opt-in is not opt-in if the read-only output
/// moves when the feature ships.
fn falsification(out: &mut String, report: &TraceReport) {
    let plans: Vec<(&FalsificationPlan, Option<&FalsificationOutcome>)> = report
        .rows
        .iter()
        .flat_map(|row| row.anchors.iter())
        .filter_map(|verdict| {
            verdict
                .falsification
                .as_ref()
                .ok()
                .map(|plan| (plan, verdict.falsification_outcome.as_ref()))
        })
        .collect();

    let in_scope = report
        .rows
        .iter()
        .filter(|row| row.severity == Severity::Error)
        .count();

    let _ = writeln!(
        out,
        "Falsification scope: {in_scope} requirement(s) classified error-severity, \
         {} plan(s) generated.",
        plans.len()
    );
    if in_scope == 0 {
        let _ = writeln!(
            out,
            "  PRD §21 declares severity per validation check, not per requirement, so \
             severity is derived from a recorded phrase match and nothing matched. Out of \
             scope is the fail-closed direction: these requirements still have to reach \
             Exercised on evidence."
        );
    }
    for (plan, outcome) in plans {
        let _ = writeln!(
            out,
            "\n  {} — break {}:{}-{}, then `{}` must fail",
            plan.requirement_id, plan.file_path, plan.line_start, plan.line_end, plan.command
        );
        let _ = writeln!(out, "    criterion: {}", plan.pass_criterion());
        match outcome {
            None => {
                let _ = writeln!(
                    out,
                    "    NOT EXECUTED. A plan promotes nothing; the edge stays at {}.",
                    ProofLevel::Exercised.as_str()
                );
            }
            Some(outcome) => {
                let _ = writeln!(out, "    {}", outcome.describe());
            }
        }
    }
}

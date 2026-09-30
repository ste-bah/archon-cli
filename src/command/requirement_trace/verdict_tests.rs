//! Batch O: a refuted claim is a body finding of the task that made it.

use archon_knowledge::traceability::anchors::{Anchor, AnchorFreshness};
use archon_knowledge::traceability::report::{AnchorVerdict, RequirementRow};
use archon_knowledge::traceability::{
    CoverageReport, FalsificationOutcome, FalsificationPlan, MutationKind, ProofLevel, Severity,
    TraceReport,
};

use super::*;

fn verdict(outcome: Option<FalsificationOutcome>) -> AnchorVerdict {
    let plan = FalsificationPlan {
        requirement_id: "REQ-WS-001".into(),
        severity_evidence: "fail closed".into(),
        task_id: "TASK-WS-002".into(),
        file_path: "src/a.rs".into(),
        line_start: 2,
        line_end: 3,
        expected_file_hash: "h".into(),
        mutation: MutationKind::AbortAnchoredRange,
        command: "cargo test -p widgets store".into(),
    };
    AnchorVerdict {
        anchor: Anchor {
            requirement_id: plan.requirement_id.clone(),
            task_id: plan.task_id.clone(),
            file_path: plan.file_path.clone(),
            line_start: plan.line_start,
            line_end: plan.line_end,
            file_hash: plan.expected_file_hash.clone(),
            path_scope: "src/".into(),
            relevance_score: 0.9,
        },
        freshness: AnchorFreshness::Fresh,
        level: ProofLevel::Exercised,
        proof: None,
        missing: None,
        falsification: Ok(plan),
        falsification_outcome: outcome,
    }
}

fn report(anchors: Vec<AnchorVerdict>) -> TraceReport {
    TraceReport {
        prd_path: "/p/PRD.md".into(),
        task_dir: "/p/tasks".into(),
        coverage: CoverageReport {
            requirements_total: 1,
            citations_total: 1,
            ..CoverageReport::default()
        },
        rows: vec![RequirementRow {
            requirement_id: "REQ-WS-001".into(),
            prd_line: 1,
            severity: Severity::Error,
            severity_evidence: None,
            claimed_by: vec!["TASK-WS-002".into()],
            anchors,
            anchor_gap: None,
            level: ProofLevel::Exercised,
        }],
        shared_anchors: Vec::new(),
        stale_anchors: 0,
        index_consulted: true,
    }
}

#[test]
fn a_refuted_claim_goes_back_to_the_body_of_the_task_that_made_it() {
    let refuted = verdict(Some(FalsificationOutcome::EdgeIsDecoration {
        mutated_exit: Some(0),
    }));
    let findings = policy_findings(&report(vec![refuted]), Vec::new());
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert_eq!(finding.subject, "TASK-WS-002");
    assert_eq!(
        finding.source_path,
        PathBuf::from("/p/tasks/TASK-WS-002.md")
    );
    assert!(
        finding.text.starts_with("task TASK-WS-002:")
            && finding.text.contains("REQ-WS-001")
            && finding.text.contains("src/a.rs:2-3")
            && finding.text.contains("cargo test -p widgets store"),
        "{}",
        finding.text
    );
}

#[test]
fn a_shown_dependency_or_an_unrun_plan_raises_nothing() {
    for outcome in [
        None,
        Some(FalsificationOutcome::DependencyShown {
            mutated_exit: Some(101),
        }),
    ] {
        assert!(policy_findings(&report(vec![verdict(outcome)]), Vec::new()).is_empty());
    }
}

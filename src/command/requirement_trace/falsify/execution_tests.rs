use super::*;

#[test]
fn a_verifier_that_breaks_with_the_anchored_lines_promotes_the_edge_to_falsifiable() {
    use archon_knowledge::traceability::anchors::{Anchor, AnchorFreshness};
    use archon_knowledge::traceability::report::{AnchorVerdict, RequirementRow};
    use archon_knowledge::traceability::{CoverageReport, ProofLevel, Severity, TraceReport};

    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let before = std::fs::read(&path).expect("read");
    let plan = plan_for(dir.path(), "git diff --quiet -- src/a.rs");

    let verdict = AnchorVerdict {
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
        falsification_outcome: None,
    };
    let mut report = TraceReport {
        prd_path: "PRD.md".into(),
        task_dir: "tasks".into(),
        coverage: CoverageReport::default(),
        rows: vec![RequirementRow {
            requirement_id: "REQ-DL-100".into(),
            prd_line: 1,
            severity: Severity::Error,
            severity_evidence: Some("fail closed".into()),
            claimed_by: vec!["TASK-A".into()],
            anchors: vec![verdict],
            anchor_gap: None,
            level: ProofLevel::Exercised,
        }],
        shared_anchors: Vec::new(),
        stale_anchors: 0,
        index_consulted: true,
    };

    execute_plans(dir.path(), &mut report);

    assert_eq!(
        report.rows[0].anchors[0].falsification_outcome,
        Some(FalsificationOutcome::DependencyShown {
            mutated_exit: Some(1)
        })
    );
    assert_eq!(report.rows[0].anchors[0].level, ProofLevel::Falsifiable);
    // The row level is recomputed from its anchors, not left where it started.
    assert_eq!(report.rows[0].level, ProofLevel::Falsifiable);
    assert!(report.rows[0].satisfied());
    // And the tree is exactly as it was found.
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert!(!guard::backup_path(&path).exists());
}

/// A verifier that fails on the unmodified tree is refused before the mutation,
/// so a pre-existing breakage can never be mistaken for a kill.
#[test]
fn a_baseline_that_does_not_pass_is_refused_before_the_mutation() {
    let (dir, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
    let before = std::fs::read(&path).expect("read");
    let plan = plan_for(dir.path(), "git rev-parse --verify no-such-ref");

    let refused = attempt(dir.path(), &plan).expect_err("refused");
    assert!(
        matches!(refused, RefusedToRun::BaselineDidNotPass { .. }),
        "{refused:?}"
    );
    assert!(refused.describe().contains("already failing"));
    assert_eq!(std::fs::read(&path).expect("read"), before);
    assert!(!guard::backup_path(&path).exists());
}

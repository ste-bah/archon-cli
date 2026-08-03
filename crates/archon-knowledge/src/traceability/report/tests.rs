use super::*;
use crate::traceability::falsification::NotPlannable;

fn anchor(req: &str, path: &str, start: usize) -> Anchor {
    Anchor {
        requirement_id: req.into(),
        task_id: "TASK-A".into(),
        file_path: path.into(),
        line_start: start,
        line_end: start + 5,
        file_hash: "aaaabbbbccccdddd".into(),
        path_scope: path.into(),
        relevance_score: 0.5,
    }
}

fn verdict(req: &str, path: &str, start: usize, level: ProofLevel) -> AnchorVerdict {
    AnchorVerdict {
        anchor: anchor(req, path, start),
        freshness: AnchorFreshness::Fresh,
        level,
        proof: None,
        missing: (level < ProofLevel::Exercised).then_some(MissingForPromotion::NoTrace),
        falsification: Err(NotPlannable::OutOfSeverityScope),
        falsification_outcome: None,
    }
}

fn row(req: &str, anchors: Vec<AnchorVerdict>) -> RequirementRow {
    let level = strongest_level(&anchors);
    RequirementRow {
        requirement_id: req.into(),
        prd_line: 1,
        severity: Severity::Unclassified,
        severity_evidence: None,
        claimed_by: vec!["TASK-A".into()],
        anchors,
        anchor_gap: None,
        level,
    }
}

fn report(rows: Vec<RequirementRow>) -> TraceReport {
    let shared_anchors = find_shared_anchors(&rows);
    TraceReport {
        prd_path: "PRD.md".into(),
        task_dir: "tests/fixtures".into(),
        coverage: CoverageReport::default(),
        rows,
        shared_anchors,
        stale_anchors: 0,
        index_consulted: true,
    }
}

#[test]
fn a_stale_anchor_contributes_nothing_to_the_level() {
    let mut stale = verdict("REQ-DL-001", "src/a.rs", 10, ProofLevel::Exercised);
    stale.freshness = AnchorFreshness::Stale {
        recorded: "aaaabbbbccccdddd".into(),
        current: "1111222233334444".into(),
    };
    let row = row("REQ-DL-001", vec![stale]);
    assert_eq!(row.level, ProofLevel::Unproven);
    assert!(!row.satisfied());
    let reasons = row.missing_reasons();
    assert_eq!(reasons.len(), 1);
    assert!(reasons[0].contains("is stale"), "{}", reasons[0]);
    assert!(reasons[0].contains("aaaabbbbcccc"), "{}", reasons[0]);
}

#[test]
fn a_missing_file_is_named_as_such() {
    let mut gone = verdict("REQ-DL-001", "src/a.rs", 10, ProofLevel::Exercised);
    gone.freshness = AnchorFreshness::FileMissing;
    let row = row("REQ-DL-001", vec![gone]);
    assert_eq!(row.level, ProofLevel::Unproven);
    assert!(row.missing_reasons()[0].contains("file that is gone"));
}

#[test]
fn the_strongest_fresh_anchor_sets_the_level() {
    let row = row(
        "REQ-DL-001",
        vec![
            verdict("REQ-DL-001", "src/a.rs", 10, ProofLevel::Candidate),
            verdict("REQ-DL-001", "src/b.rs", 20, ProofLevel::Exercised),
        ],
    );
    assert_eq!(row.level, ProofLevel::Exercised);
    assert!(row.satisfied());
}

#[test]
fn an_unclaimed_requirement_reads_as_a_decomposition_gap() {
    let mut row = row("REQ-DL-009", Vec::new());
    row.claimed_by.clear();
    assert_eq!(row.level, ProofLevel::Unproven);
    let reasons = row.missing_reasons();
    assert!(reasons[0].contains("decomposition gap"), "{}", reasons[0]);
}

#[test]
fn a_task_with_no_declared_paths_says_so_and_names_f1() {
    let mut row = row("REQ-DL-001", Vec::new());
    row.anchor_gap = Some(AnchorGap::NoDeclaredPaths {
        task_id: "TASK-TDL-130".into(),
    });
    let reasons = row.missing_reasons();
    assert!(reasons[0].contains("TASK-TDL-130"), "{}", reasons[0]);
    assert!(reasons[0].contains("F1"), "{}", reasons[0]);
}

/// The F1 shape, computed: one span standing in for four requirements.
#[test]
fn evidence_reused_across_requirements_is_detected_as_a_shared_anchor() {
    let rows: Vec<RequirementRow> = (1..=4)
        .map(|n| {
            let id = format!("REQ-DL-00{n}");
            row(
                &id,
                vec![verdict(&id, "src/generic.rs", 1, ProofLevel::Candidate)],
            )
        })
        .collect();
    let report = report(rows);
    assert_eq!(report.shared_anchors.len(), 1);
    assert_eq!(report.shared_anchors[0].citation, "src/generic.rs:1-6");
    assert_eq!(
        report.shared_anchors[0].requirement_ids,
        ["REQ-DL-001", "REQ-DL-002", "REQ-DL-003", "REQ-DL-004"]
    );
}

#[test]
fn distinct_anchors_are_not_flagged() {
    let rows: Vec<RequirementRow> = (1..=3)
        .map(|n| {
            let id = format!("REQ-DL-00{n}");
            row(
                &id,
                vec![verdict(&id, "src/a.rs", n * 100, ProofLevel::Candidate)],
            )
        })
        .collect();
    assert!(report(rows).shared_anchors.is_empty());
}

#[test]
fn the_verdict_is_neither_a_pass_nor_a_failure() {
    let report = report(vec![
        row(
            "REQ-DL-001",
            vec![verdict("REQ-DL-001", "src/a.rs", 1, ProofLevel::Exercised)],
        ),
        row(
            "REQ-DL-002",
            vec![verdict("REQ-DL-002", "src/b.rs", 1, ProofLevel::Candidate)],
        ),
        row("REQ-DL-003", Vec::new()),
    ]);
    let verdict = report.gate_verdict();
    assert!(
        verdict.starts_with("1/3 requirements satisfied"),
        "{verdict}"
    );
    assert!(verdict.contains("declared residual gaps"), "{verdict}");
    assert_eq!(report.satisfied().len(), 1);
    assert_eq!(report.residual_gaps().len(), 2);

    let counts = report.level_counts();
    assert_eq!(counts["Exercised"], 1);
    assert_eq!(counts["Candidate"], 1);
    assert_eq!(counts["Unproven"], 1);
    assert_eq!(counts["Falsifiable"], 0);
}

#[test]
fn ninety_three_candidates_do_not_add_up_to_one_satisfied_requirement() {
    let rows: Vec<RequirementRow> = (1..=93)
        .map(|n| {
            let id = format!("REQ-DL-{n:03}");
            row(
                &id,
                vec![verdict(
                    &id,
                    &format!("src/f{n}.rs"),
                    1,
                    ProofLevel::Candidate,
                )],
            )
        })
        .collect();
    let report = report(rows);
    assert_eq!(report.satisfied().len(), 0);
    assert_eq!(report.residual_gaps().len(), 93);
    assert!(report.gate_verdict().starts_with("0/93"));
}

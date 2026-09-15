//! The prior view is bounded by declared paths, never by history length.
use super::*;
use crate::repository_audit::{
    AuditRecord, AuditReport, RequiredAction, Verdict,
    correction::Correction,
    ledger::{AuditLedger, Obligation, Reassessment, Waiver},
};
use std::collections::{BTreeMap, BTreeSet};

fn record(path: &str, reason: &str) -> AuditRecord {
    AuditRecord {
        declared_path: path.into(),
        verdict: Verdict::ExistsAsDeclared,
        equivalents: vec![],
        required_action: RequiredAction::None,
        reason: reason.into(),
    }
}

fn report(snapshot: &str, records: Vec<AuditRecord>) -> AuditReport {
    AuditReport {
        schema_version: 1,
        snapshot: snapshot.into(),
        records,
    }
}

fn declared(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| (*p).to_string()).collect()
}

fn by_path(view: &PriorView<'_>) -> BTreeMap<String, PriorRecord> {
    view.records
        .iter()
        .map(|r| (r.declared_path.clone(), r.clone()))
        .collect()
}

#[test]
fn each_declared_path_takes_its_most_recent_record() {
    let mut ledger = AuditLedger::default();
    for snapshot in ["s1", "s2", "s3"] {
        ledger.history.push(report(
            snapshot,
            ["a.rs", "b.rs", "c.rs"]
                .iter()
                .map(|p| record(p, &format!("{p}@{snapshot}")))
                .collect(),
        ));
    }
    let view = PriorView::build(&ledger, &declared(&["a.rs", "b.rs", "c.rs"]));
    assert_eq!(view.history_reports, 3);
    assert_eq!(view.records.len(), 3);
    for r in &view.records {
        assert_eq!(r.snapshot, "s3");
        assert_eq!(r.reason, format!("{}@s3", r.declared_path));
    }
}

#[test]
fn most_recent_means_last_report_mentioning_the_path() {
    let mut ledger = AuditLedger::default();
    ledger.history.push(report(
        "s1",
        vec![record("a.rs", "a@s1"), record("b.rs", "b@s1")],
    ));
    ledger
        .history
        .push(report("s2", vec![record("a.rs", "a@s2")]));
    ledger
        .history
        .push(report("s3", vec![record("b.rs", "b@s3")]));
    let view = PriorView::build(&ledger, &declared(&["a.rs", "b.rs"]));
    let records = by_path(&view);
    assert_eq!(records["a.rs"].snapshot, "s2");
    assert_eq!(records["a.rs"].reason, "a@s2");
    assert_eq!(records["b.rs"].snapshot, "s3");
    assert_eq!(records["b.rs"].reason, "b@s3");
}

#[test]
fn path_only_in_an_early_report_keeps_that_early_snapshot() {
    let mut ledger = AuditLedger::default();
    ledger.history.push(report(
        "s1",
        vec![record("old.rs", "assessed once"), record("a.rs", "a@s1")],
    ));
    ledger
        .history
        .push(report("s2", vec![record("a.rs", "a@s2")]));
    ledger
        .history
        .push(report("s3", vec![record("a.rs", "a@s3")]));
    let view = PriorView::build(&ledger, &declared(&["old.rs", "a.rs", "never.rs"]));
    let records = by_path(&view);
    assert_eq!(records.len(), 2);
    assert_eq!(records["old.rs"].snapshot, "s1");
    assert_eq!(records["old.rs"].reason, "assessed once");
    assert_eq!(records["a.rs"].snapshot, "s3");
    assert!(!records.contains_key("never.rs"));
}

#[test]
fn undeclared_paths_are_not_carried() {
    let mut ledger = AuditLedger::default();
    ledger.history.push(report(
        "s1",
        vec![record("a.rs", "a"), record("dropped.rs", "d")],
    ));
    let view = PriorView::build(&ledger, &declared(&["a.rs"]));
    assert_eq!(view.records.len(), 1);
    assert_eq!(view.records[0].declared_path, "a.rs");
}

#[test]
fn reason_is_clipped_at_the_limit_with_marker() {
    let short = "x".repeat(REASON_LIMIT);
    assert_eq!(clip_reason(&short), short);
    let long = "y".repeat(REASON_LIMIT + 1);
    let clipped = clip_reason(&long);
    assert_eq!(
        clipped,
        format!("{}{}", "y".repeat(REASON_LIMIT), CLIP_MARKER)
    );
    assert!(clipped.ends_with(CLIP_MARKER));
    assert_eq!(clipped.len(), REASON_LIMIT + CLIP_MARKER.len());

    let mut ledger = AuditLedger::default();
    ledger.history.push(report(
        "s1",
        vec![record("a.rs", &long), record("b.rs", "brief")],
    ));
    let view = PriorView::build(&ledger, &declared(&["a.rs", "b.rs"]));
    let records = by_path(&view);
    assert_eq!(records["a.rs"].reason, clipped);
    assert_eq!(records["b.rs"].reason, "brief");
}

#[test]
fn clip_respects_multibyte_char_boundaries() {
    let multibyte = "é".repeat(REASON_LIMIT);
    let clipped = clip_reason(&multibyte);
    assert!(clipped.len() <= REASON_LIMIT + CLIP_MARKER.len());
    assert!(clipped.ends_with(CLIP_MARKER));
    assert!(
        clipped
            .trim_end_matches(CLIP_MARKER)
            .chars()
            .all(|c| c == 'é')
    );
}

#[test]
fn bookkeeping_collections_are_carried_verbatim() {
    let mut ledger = AuditLedger::default();
    ledger.history.push(report("s1", vec![record("a.rs", "a")]));
    ledger.obligations.insert(
        "a.rs".into(),
        Obligation {
            opened_snapshot: "s1".into(),
            proposed_explanation: Some("will wire".into()),
            applied_commit: Some("abc".into()),
            resolved_snapshot: None,
        },
    );
    ledger.waivers.push(Waiver {
        declared_path: "a.rs".into(),
        snapshot: "s1".into(),
        action_id: "w1".into(),
        reason: "waived".into(),
        assessment_count: 1,
    });
    ledger.reassessments.push(Reassessment {
        declared_path: "a.rs".into(),
        snapshot: "s1".into(),
        action_id: "r1".into(),
        reason: "dispute".into(),
        attempted: false,
    });
    ledger.corrections.push(Correction {
        declared_path: "a.rs".into(),
        snapshot: "s1".into(),
        action_id: "r1".into(),
        reason: "was wrong".into(),
        evidence_paths: vec!["b.rs".into()],
    });
    let view = PriorView::build(&ledger, &declared(&["a.rs"]));
    let json = serde_json::to_value(&view).unwrap();
    let raw = serde_json::to_value(&ledger).unwrap();
    for key in ["obligations", "waivers", "reassessments", "corrections"] {
        assert_eq!(json[key], raw[key], "{key} must be carried verbatim");
    }
    assert_eq!(json["history_reports"], 1);
    assert!(
        json.get("history").is_none(),
        "raw history must not leak into the view"
    );
}

#[test]
fn view_size_is_bounded_by_declared_paths_not_history() {
    const REPORTS: usize = 20;
    const PATHS: usize = 80;
    const REASON_BYTES: usize = 2000;
    let paths: Vec<String> = (0..PATHS)
        .map(|i| format!("src/module_{i:03}/file.rs"))
        .collect();
    let mut ledger = AuditLedger::default();
    for n in 0..REPORTS {
        ledger.history.push(report(
            &format!("snapshot-{n:02}"),
            paths
                .iter()
                .map(|p| record(p, &"r".repeat(REASON_BYTES)))
                .collect(),
        ));
    }
    let declared_paths: BTreeSet<String> = paths.iter().cloned().collect();
    let view = PriorView::build(&ledger, &declared_paths);
    let view_bytes = serde_json::to_string(&view).unwrap().len();
    let raw_bytes = serde_json::to_string(&ledger).unwrap().len();
    const VIEW_CEILING: usize = PATHS * 700 + 4096;
    assert!(
        raw_bytes > 3 * 1024 * 1024,
        "raw ledger {raw_bytes} bytes should exceed 3 MB"
    );
    assert!(
        view_bytes < VIEW_CEILING,
        "view {view_bytes} bytes must stay under {VIEW_CEILING}"
    );
    assert_eq!(view.records.len(), PATHS);
    eprintln!("prior view {view_bytes} bytes vs raw ledger {raw_bytes} bytes");
}

use archon_cognitive::self_model::{PreflightRecommendation, ReasoningQualityPreflight};
use archon_reasoning_quality::store::ReasoningQualityStore;
use archon_reasoning_quality::{
    ConfidenceSignal, ReasoningEventKind, ReasoningQualityEvent, ReasoningSubject,
};

fn event(kind: ReasoningEventKind, severity: f32) -> ReasoningQualityEvent {
    ReasoningQualityEvent {
        event_id: format!("{kind:?}-{severity}"),
        session_id: "s1".into(),
        event_kind: kind,
        subject: ReasoningSubject::Codebase,
        confidence_signal: ConfidenceSignal::Confident,
        severity_base: severity,
        severity_effective: severity,
        ..ReasoningQualityEvent::default()
    }
}

#[test]
fn clear_when_no_risky_events_exist() {
    let report = ReasoningQualityPreflight::default()
        .run_for_events(&[event(ReasoningEventKind::SourceVerifiedClaim, 0.0)]);

    assert!(report.store_available);
    assert!(!report.has_risks());
    assert_eq!(report.recommendation, PreflightRecommendation::Clear);
}

#[test]
fn risk_flags_are_raised_for_unsupported_and_completion_claims() {
    let report = ReasoningQualityPreflight::default().run_for_events(&[
        event(ReasoningEventKind::UnsupportedClaim, 0.6),
        event(ReasoningEventKind::CompletionClaimWithoutEvidence, 0.7),
        event(ReasoningEventKind::TestStatusClaimWithoutCommand, 0.6),
    ]);

    assert_eq!(report.recommendation, PreflightRecommendation::Caution);
    assert_eq!(report.flags.len(), 3);
    assert!(
        report
            .flags
            .iter()
            .any(|flag| flag.kind == "false_completion_history")
    );
    assert!(
        report
            .flags
            .iter()
            .any(|flag| flag.kind == "test_build_claim_risk")
    );
}

#[test]
fn high_severity_contradiction_blocks_completion_claims() {
    let report = ReasoningQualityPreflight::default()
        .run_for_events(&[event(ReasoningEventKind::ClaimContradictedBySource, 1.0)]);

    assert_eq!(report.recommendation, PreflightRecommendation::Block);
    assert_eq!(report.flags[0].kind, "source_contradiction");
}

#[test]
fn preflight_reads_reasoning_quality_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = ReasoningQualityStore::open(temp.path()).expect("rq store");
    store
        .append_events(&[
            event(ReasoningEventKind::ClaimCorrectedByUser, 1.0),
            event(ReasoningEventKind::SourceVerifiedClaim, 0.0),
        ])
        .expect("append");

    let report = ReasoningQualityPreflight::default().run_for_store(&store, Some("s1"));

    assert_eq!(report.recommendation, PreflightRecommendation::Block);
    assert!(
        report
            .flags
            .iter()
            .any(|flag| flag.kind == "recent_user_correction")
    );
}

#[test]
fn unavailable_report_is_caution_without_flags() {
    let report = archon_cognitive::self_model::RiskReport::unavailable();

    assert!(!report.store_available);
    assert!(!report.has_risks());
    assert_eq!(report.recommendation, PreflightRecommendation::Caution);
}

use super::*;

#[test]
fn validation_coverage_and_large_gaps_fail_closed() {
    let bars = vec![
        bar("2026-01-01T00:00:00Z", 10.0, 100.0),
        bar("2026-01-02T00:00:00Z", 11.0, 110.0),
    ];
    let mut metadata = complete_metadata(&bars);
    metadata.coverage.expected_bars = 200;
    metadata.gaps.expected_bars = 200;
    metadata.gaps.missing_bars = 198;
    let policy = coverage_policy();
    assert!(coverage_inputs_are_consistent(&metadata, &bars, &policy));
    assert!(!large_gap_within_policy(&metadata, &policy));

    let report = validation_report(&metadata, &bars, metadata.created_at.clone());
    assert_eq!(report.coverage_policy, policy);
    assert_eq!(report.session_calendar_evidence.observed_bar_count, 2);
    assert!(report.is_consistent());

    let mut invalid_policy = policy.clone();
    invalid_policy.minimum_bar_count = 0;
    assert!(!coverage_inputs_are_consistent(
        &metadata,
        &bars,
        &invalid_policy
    ));

    metadata.coverage.end = "2026-01-03T00:00:00Z".into();
    let report = validation_report(&metadata, &bars, metadata.created_at.clone());
    assert_failed_check(&report, "ohlcv.coverage_inputs");

    metadata = complete_metadata(&bars);
    metadata.coverage.observed_bars = 1;
    metadata.gaps.missing_bars = 1;
    let report = validation_report(&metadata, &bars, metadata.created_at.clone());
    assert_failed_check(&report, "ohlcv.coverage_inputs");

    metadata = complete_metadata(&bars);
    metadata.coverage.expected_bars = 5;
    metadata.gaps.expected_bars = 5;
    metadata.gaps.missing_bars = 3;
    let report = validation_report(&metadata, &bars, metadata.created_at.clone());
    let large_gap = report
        .checks
        .iter()
        .find(|check| check.id == "ohlcv.large_gaps")
        .unwrap();
    assert_eq!(large_gap.severity, ValidationSeverity::Warning);
    assert_eq!(report.status, ValidationStatus::Degraded);
    assert!(!report.production_eligible);
    let mut isolated = report.checks.clone();
    isolated
        .iter_mut()
        .filter(|check| check.id != "ohlcv.large_gaps")
        .for_each(|check| check.status = ValidationStatus::Passed);
    assert_eq!(
        ValidationReport::status_from_checks(&isolated),
        ValidationStatus::Degraded
    );
}

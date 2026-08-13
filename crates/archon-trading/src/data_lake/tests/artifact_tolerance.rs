//! What a hand-authored artifact is allowed to say — and what it must still be
//! refused for saying.
//!
//! Both halves of this file exist because a whole registry loads as a unit, so
//! one over-strict rule against one file takes the entire lake offline. The
//! `DataType` tests pin the tolerance that was added (casing and separators);
//! the `ValidationReport` tests pin the one-sidedness of the consistency rule.
//! Tolerance that runs in the wrong direction would let a dataset claim to be
//! something it is not, which is exactly what these guard against.

/// `data_type` is authored by hand in metadata.json and manifest.json, and the
/// lowercase spelling is the natural one — every dataset id, filename and CLI
/// argument around it is lowercase. Five artifacts on one installation used it,
/// and because the registry loads as a unit, one of them made the entire lake
/// unreadable: `trading data status` and `list` both failed outright.
#[test]
fn data_type_reads_any_casing_but_writes_pascal_case() {
    use crate::data_lake::DataType;

    for raw in ["\"Ohlcv\"", "\"ohlcv\"", "\"OHLCV\"", "\"oHlCv\""] {
        let parsed: DataType =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("{raw} must parse: {error}"));
        assert_eq!(parsed, DataType::Ohlcv, "{raw}");
    }

    // Separators are normalised too, so a snake_case authoring habit works.
    for raw in [
        "\"CorporateActions\"",
        "\"corporate_actions\"",
        "\"corporate-actions\"",
    ] {
        let parsed: DataType =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("{raw} must parse: {error}"));
        assert_eq!(parsed, DataType::CorporateActions, "{raw}");
    }

    // Output is unchanged, so nothing already on disk needs rewriting and no
    // recorded checksum is invalidated by this.
    assert_eq!(
        serde_json::to_string(&DataType::Ohlcv).unwrap(),
        "\"Ohlcv\""
    );
}

/// Case tolerance must not become kind tolerance: an unrecognised data type is
/// a real error and has to keep failing, or a dataset could claim to hold
/// something it does not.
#[test]
fn unknown_data_type_still_fails_with_a_listing_of_valid_kinds() {
    use crate::data_lake::DataType;

    let error = serde_json::from_str::<DataType>("\"candlesticks\"")
        .expect_err("an unknown kind must not be silently accepted");
    let text = error.to_string();
    assert!(text.contains("candlesticks"), "{text}");
    assert!(
        text.contains("ohlcv"),
        "the error must list valid kinds: {text}"
    );
}

/// Persisted status is derived from checks, so even a pessimistic mismatch is
/// rejected rather than allowing two competing sources of truth.
#[test]
fn a_status_more_severe_than_its_checks_is_rejected() {
    use crate::data_lake::contracts::{
        ValidationCheck, ValidationReport, ValidationSeverity, ValidationStatus,
    };

    let passing_check = ValidationCheck {
        id: "row_count".into(),
        status: ValidationStatus::Passed,
        severity: ValidationSeverity::Error,
        message: "14 rows".into(),
    };
    let report = ValidationReport {
        schema_version: "archon-trading-validation-v1".into(),
        dataset_id: "yfinance-SPY-1D-raw".into(),
        version: "20240101".into(),
        status: ValidationStatus::Failed, // provider unavailable — not a check
        native_interval: false,
        native_lineage: None,
        production_eligible: false,
        coverage_policy: crate::data_lake::CoverageValidationPolicy {
            minimum_bar_count: 1,
            large_gap_threshold_bps: 100,
        },
        session_calendar_evidence: crate::data_lake::SessionCalendarEvidence {
            session: "24x7".into(),
            calendar: "continuous_24x7".into(),
            timezone: "UTC".into(),
            coverage_start: "2026-01-01T00:00:00Z".into(),
            coverage_end: "2026-01-01T00:00:00Z".into(),
            first_observed_at: "2026-01-01T00:00:00Z".into(),
            last_observed_at: "2026-01-01T00:00:00Z".into(),
            expected_bar_count: 1,
            observed_bar_count: 1,
            derivation: "fixture calendar".into(),
        },
        checks: vec![passing_check],
        normalized_sha256: "normalized123".into(),
        content_sha256: "abc123".into(),
        summary: Default::default(),
        validated_at: "2026-01-01T00:00:00Z".into(),
    };

    assert!(!report.is_consistent());
    assert!(!report.allows_production());
}

/// The one-sided rule must stay one-sided: claiming Passed while a check failed
/// is the false pass this contract exists to prevent.
#[test]
fn a_status_better_than_its_checks_is_still_a_contradiction() {
    use crate::data_lake::contracts::{
        ValidationCheck, ValidationReport, ValidationSeverity, ValidationStatus,
    };

    let failing_check = ValidationCheck {
        id: "volume_present".into(),
        status: ValidationStatus::Failed,
        severity: ValidationSeverity::Error,
        message: "all volumes zero".into(),
    };
    let report = ValidationReport {
        schema_version: "archon-trading-validation-v1".into(),
        dataset_id: "tradingview-GOLD-1D-raw".into(),
        version: "20260218".into(),
        status: ValidationStatus::Passed, // contradicts the failing check
        native_interval: true,
        native_lineage: None,
        production_eligible: true,
        coverage_policy: crate::data_lake::CoverageValidationPolicy {
            minimum_bar_count: 1,
            large_gap_threshold_bps: 100,
        },
        session_calendar_evidence: crate::data_lake::SessionCalendarEvidence {
            session: "24x7".into(),
            calendar: "continuous_24x7".into(),
            timezone: "UTC".into(),
            coverage_start: "2026-01-01T00:00:00Z".into(),
            coverage_end: "2026-01-01T00:00:00Z".into(),
            first_observed_at: "2026-01-01T00:00:00Z".into(),
            last_observed_at: "2026-01-01T00:00:00Z".into(),
            expected_bar_count: 1,
            observed_bar_count: 1,
            derivation: "fixture calendar".into(),
        },
        checks: vec![failing_check],
        normalized_sha256: "normalized123".into(),
        content_sha256: "abc123".into(),
        summary: Default::default(),
        validated_at: "2026-01-01T00:00:00Z".into(),
    };

    assert!(
        !report.is_consistent(),
        "claiming Passed over a failed check must remain a contradiction"
    );
    assert!(!report.allows_production());
}

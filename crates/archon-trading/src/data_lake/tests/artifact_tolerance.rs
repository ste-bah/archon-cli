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

/// A dataset can fail for reasons no check represents — provider unavailable,
/// zero bars returned, credential absent. Demanding the status equal
/// `status_from_checks` rejected those honest fail-closed records: 16 of them
/// on one installation made the entire registry unloadable for being too
/// careful. Pessimism is safe and must load.
#[test]
fn a_status_more_severe_than_its_checks_is_consistent() {
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
        production_eligible: false,
        checks: vec![passing_check],
        content_sha256: "abc123".into(),
        summary: Default::default(),
        validated_at: "2026-01-01T00:00:00Z".into(),
    };

    assert!(
        report.is_consistent(),
        "a report failing for a reason outside its checks must still load"
    );
    assert!(
        !report.allows_production(),
        "and it must certainly not be production-eligible"
    );
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
        production_eligible: true,
        checks: vec![failing_check],
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

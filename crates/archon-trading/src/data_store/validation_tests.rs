use super::*;
use crate::data_lake::{DerivationLineage, NativeObservationEvidence};

#[path = "validation_tests/contract_core.rs"]
mod contract_core;
#[path = "tests/coverage_contract.rs"]
mod coverage_contract;
#[path = "tests/metadata_artifact_gaps.rs"]
mod metadata_artifact_gaps;
#[path = "tests/native_lineage.rs"]
mod native_lineage;
#[path = "tests/production_gate.rs"]
mod production_gate;
#[path = "tests/row_values.rs"]
mod row_values;
#[path = "tests/status_contract.rs"]
mod status_contract;
#[path = "tests/validation_atomicity.rs"]
mod validation_atomicity;
#[path = "tests/volume_evidence.rs"]
mod volume_evidence;

fn passed_checks() -> Vec<ValidationCheck> {
    crate::data_lake::required_check_ids()
        .iter()
        .map(|id| ValidationCheck {
            id: (*id).into(),
            status: ValidationStatus::Passed,
            severity: ValidationSeverity::Error,
            message: "passed".into(),
        })
        .collect()
}

fn report(checks: Vec<ValidationCheck>) -> ValidationReport {
    let status = ValidationReport::status_from_checks(&checks);
    let normalized_sha256 = "normalized-sha256".to_string();
    let summary = ValidationSummary {
        row_count: 2,
        duplicate_timestamp_count: 0,
        gap_count: 0,
        bad_ohlc_count: 0,
        missing_volume_count: 0,
    };
    ValidationReport {
        schema_version: crate::data_lake::VALIDATION_REPORT_SCHEMA.into(),
        dataset_id: "polygon-SPY-1D-raw".into(),
        version: "20260101-live".into(),
        status,
        native_interval: true,
        native_lineage: Some(NativeLineageEvidence {
            observation: NativeObservationEvidence {
                dataset_id: "polygon-SPY-1D-raw".into(),
                version: "20260101-live".into(),
                provider: "polygon".into(),
                canonical_instrument: "SPY".into(),
                provider_symbol: "SPY".into(),
                timeframe: "1D".into(),
                retrieved_at: "2026-01-01T00:00:00Z".into(),
                exact_native_interval: true,
                complete: true,
            },
            lineage: DerivationLineage::default(),
        }),
        production_eligible: status == ValidationStatus::Passed,
        coverage_policy: CoverageValidationPolicy {
            minimum_bar_count: 1,
            large_gap_threshold_bps: 100,
        },
        session_calendar_evidence: SessionCalendarEvidence {
            session: "24x7".into(),
            calendar: "continuous_24x7".into(),
            timezone: "UTC".into(),
            coverage_start: "2026-01-01T00:00:00Z".into(),
            coverage_end: "2026-01-02T00:00:00Z".into(),
            first_observed_at: "2026-01-01T00:00:00Z".into(),
            last_observed_at: "2026-01-02T00:00:00Z".into(),
            expected_bar_count: 2,
            observed_bar_count: 2,
            derivation: "fixture calendar".into(),
        },
        content_sha256: ValidationReport::content_hash(&normalized_sha256, &checks, &summary),
        normalized_sha256,
        checks,
        summary,
        validated_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn bar(timestamp: &str, close: f64, volume: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume,
    }
}

#[test]
fn validation_compares_parsed_rfc3339_instants() {
    let timezone_less = vec![bar("2026-01-01T00:00:00", 9.0, 90.0)];
    assert!(!timestamp_values_are_rfc3339(&timezone_less));
    let timezone_less_report = validation_report(
        &complete_metadata(&timezone_less),
        &timezone_less,
        "2026-01-01T00:00:00Z".into(),
    );
    assert_failed_check(&timezone_less_report, "ohlcv.rfc3339_timestamps");

    let later = bar("2026-01-01T00:30:00Z", 10.0, 100.0);
    let earlier_with_offset = bar("2026-01-01T01:00:00+01:00", 11.0, 110.0);
    assert!(timestamp_values_are_rfc3339(&[
        later.clone(),
        earlier_with_offset.clone()
    ]));
    assert!(has_unsorted_timestamps(&[
        later.clone(),
        earlier_with_offset
    ]));

    let same_instant = bar("2025-12-31T19:30:00-05:00", 11.0, 110.0);
    let metadata = complete_metadata(&[later.clone(), same_instant.clone()]);
    assert_eq!(
        validation_summary(&metadata, &[later, same_instant]).duplicate_timestamp_count,
        1
    );
}

#[test]
fn validation_checks_every_numeric_and_ohlc_row() {
    row_values::run();
}

fn assert_failed_check(report: &ValidationReport, expected_id: &str) {
    assert!(
        report
            .checks
            .iter()
            .any(|check| { check.id == expected_id && check.status == ValidationStatus::Failed }),
        "expected failed check {expected_id}; checks were {:?}",
        report.checks
    );
}

#[test]
fn validation_rejects_each_metadata_and_artifact_gap() {
    metadata_artifact_gaps::run();
}

#[test]
fn validation_rejects_non_native_and_derived_data() {
    native_lineage::run();
}

#[test]
fn validation_gate_denies_backtest_and_promotion_faults() {
    production_gate::run();
}

#[test]
fn validation_report_atomic_failure_preserves_prior_state() {
    validation_atomicity::run();
}

#[test]
fn governed_single_write_failpoints_restore_prior_bytes() {
    for boundary in [
        "file.create",
        "file.write",
        "file.sync",
        "file.rename",
        "directory.sync",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("registry.json");
        std::fs::write(&path, b"prior-registry").unwrap();
        inject_io_failure(Some(boundary));
        let result = write_bytes(&path, b"replacement");
        inject_io_failure(None);
        assert!(result.is_err(), "{boundary} did not fail");
        assert_eq!(std::fs::read(path).unwrap(), b"prior-registry");
    }
}

fn complete_metadata(bars: &[OhlcvBar]) -> DatasetMetadata {
    let checksum = normalized_bars_checksum(bars).unwrap();
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id: "manual-BTCUSD-1D-raw".into(),
        version: "20260101-fixture".into(),
        canonical_instrument: "BTCUSD".into(),
        asset_class: "crypto".into(),
        provider: "manual".into(),
        provider_symbol: "BTCUSD".into(),
        timeframe: "1D".into(),
        native_interval: true,
        production_eligible: true,
        price_basis: "raw".into(),
        session: "24x7".into(),
        data_type: crate::data_lake::DataType::Ohlcv,
        symbol_map: BTreeMap::from([("BTCUSD".into(), "BTCUSD".into())]),
        timezone: "UTC".into(),
        adjustment: "raw".into(),
        license: "research".into(),
        coverage: crate::data_lake::CoverageWindow {
            start: bars.first().unwrap().timestamp.clone(),
            end: bars.last().unwrap().timestamp.clone(),
            expected_bars: bars.len() as u64,
            observed_bars: bars.len() as u64,
        },
        gaps: crate::data_lake::GapSummary {
            missing_bars: 0,
            expected_bars: bars.len() as u64,
        },
        checksum: checksum.clone(),
        checksums: crate::data_lake::DatasetChecksums {
            raw_sha256: "raw".into(),
            normalized_sha256: checksum,
            metadata_sha256: "metadata".into(),
        },
        paths: crate::data_lake::DatasetArtifactPaths {
            raw: "raw/response.csv".into(),
            raw_response: "raw/response.csv".into(),
            raw_request: "raw/request.json".into(),
            redacted_headers: "raw/headers.redacted.json".into(),
            provider_notes: "raw/provider-notes.md".into(),
            normalized: "ohlcv.jsonl".into(),
            validation: "validation.json".into(),
            manifest: "manifest.json".into(),
        },
        source: crate::data_lake::DatasetSourceMetadata {
            license_notes: "research use".into(),
            url_or_endpoint: "manual fixture".into(),
            retrieved_at: "2026-01-01T00:00:00Z".into(),
            credential_required: false,
        },
        quality_status: "passed".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        optional: false,
    }
}

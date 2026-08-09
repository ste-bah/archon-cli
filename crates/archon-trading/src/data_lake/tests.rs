//! The dataset metadata contract and the registry that enforces it.
//!
//! Provider capability and native-fetch behaviour live in
//! [`provider_capability`]; the tolerance rules for hand-authored artifacts
//! live in [`artifact_tolerance`]. Both are children of this module so they
//! keep reaching `data_lake`'s private items exactly as this file does.

mod artifact_tolerance;
mod provider_capability;

use super::*;
use crate::spec_registry::{Instrument, StrategySpec};

fn metadata(data_type: DataType) -> DatasetMetadata {
    DatasetMetadata {
        schema_version: dataset_schema(),
        dataset_id: "approvedprovider-SPY-1D-raw".into(),
        version: "20260101-abc123".into(),
        canonical_instrument: "SPY".into(),
        asset_class: "equity".into(),
        provider: "approvedprovider".into(),
        provider_symbol: "SPY".into(),
        timeframe: "1D".into(),
        native_interval: true,
        production_eligible: true,
        price_basis: "raw".into(),
        session: "regular".into(),
        data_type,
        symbol_map: BTreeMap::from([("SPY".into(), "SPY".into())]),
        timezone: "America/New_York".into(),
        adjustment: "split_and_dividend".into(),
        license: "licensed".into(),
        coverage: CoverageWindow {
            start: "2020-01-01".into(),
            end: "2024-01-01".into(),
            expected_bars: 100,
            observed_bars: 100,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: 100,
        },
        checksum: "abc123".into(),
        checksums: DatasetChecksums {
            raw_sha256: "raw123".into(),
            normalized_sha256: "abc123".into(),
            metadata_sha256: "meta123".into(),
        },
        paths: DatasetArtifactPaths {
            raw: "raw/raw-response.json".into(),
            raw_response: "raw/raw-response.json".into(),
            raw_request: "raw/raw-request.json".into(),
            redacted_headers: "raw/redacted-headers.json".into(),
            provider_notes: "provider-notes.md".into(),
            normalized: "ohlcv.jsonl".into(),
            validation: "validation.json".into(),
            manifest: "manifest.json".into(),
        },
        source: DatasetSourceMetadata::default(),
        quality_status: "passed".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        optional: false,
    }
}

#[test]
fn t_data_05_rejects_missing_required_metadata_field() {
    let mut missing = metadata(DataType::Ohlcv);
    missing.provider.clear();
    assert_eq!(
        validate_metadata(&missing),
        Err(DataLakeError::MissingField("provider"))
    );
}

#[test]
fn t_data_06_gap_above_one_percent_is_degraded_and_blocks_promotion() {
    let mut registry = DatasetRegistry::default();
    let mut degraded = metadata(DataType::Ohlcv);
    degraded.gaps = GapSummary {
        missing_bars: 2,
        expected_bars: 100,
    };
    let versioned = registry.register(degraded).unwrap();
    assert_eq!(versioned.status, DatasetStatus::Degraded);
    assert_eq!(
        registry.promotion_ready(&[InstrumentClass::Crypto], false),
        Err(DataLakeError::DegradedDataset)
    );
}

#[test]
fn degraded_optional_dataset_does_not_satisfy_mandatory_promotion_data() {
    let mut registry = DatasetRegistry::default();
    let mut degraded = metadata(DataType::Ohlcv);
    degraded.optional = true;
    degraded.gaps = GapSummary {
        missing_bars: 2,
        expected_bars: 100,
    };
    registry.register(degraded).unwrap();
    assert_eq!(
        registry.promotion_ready(&[InstrumentClass::Crypto], false),
        Err(DataLakeError::MissingMandatoryData(vec![
            DataType::Ohlcv,
            DataType::Funding,
        ]))
    );
}

#[test]
fn ec_trl_07_enforces_mandatory_matrix_and_event_news() {
    let mut registry = DatasetRegistry::default();
    for data_type in [
        DataType::Ohlcv,
        DataType::CorporateActions,
        DataType::Fundamentals,
        DataType::IndexConstituents,
        DataType::News,
    ] {
        registry.register(metadata(data_type)).unwrap();
    }
    assert!(
        registry
            .promotion_ready(&[InstrumentClass::Equity], true)
            .is_ok()
    );
    assert_eq!(
        registry.promotion_ready(&[InstrumentClass::Future], false),
        Err(DataLakeError::MissingMandatoryData(vec![
            DataType::ContinuousContract,
            DataType::ContractSpecs,
        ]))
    );
}

#[test]
fn fx_or_options_need_spec_amendment_before_advancing_past_idea() {
    let mut spec = StrategySpec {
        spec_f01_instrument_universe: Some(vec![Instrument {
            symbol: "EURUSD".into(),
            venue: "OTC".into(),
            asset_class: "fx".into(),
        }]),
        spec_f02_timeframe_session: None,
        spec_f03_market_regime_assumptions: None,
        spec_f04_data_dependencies: None,
        spec_f05_entry_exit_rules: None,
        spec_f06_indicator_formulas: None,
        spec_f07_position_sizing: None,
        spec_f08_stops: None,
        spec_f09_invalidation_rules: None,
        spec_f10_no_trade_conditions: None,
        spec_f11_cost_assumptions: None,
        spec_f12_benchmark: None,
        spec_f13_expected_failure_modes: None,
        spec_f14_data_quality_tolerances_ms: None,
        spec_f15_promotion_status: Some(PromotionStatus::Idea),
    };
    assert!(spec_can_advance_past_idea(&spec).is_ok());
    spec.spec_f15_promotion_status = Some(PromotionStatus::Research);
    assert_eq!(
        spec_can_advance_past_idea(&spec),
        Err(DataLakeError::FxOptionsNeedSpecAmendment)
    );
}

#[test]
fn production_metadata_requires_native_passed_quality() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.native_interval = false;
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::NonNativeProductionDataset)
    );
    metadata.native_interval = true;
    metadata.quality_status = "degraded".into();
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::MetadataIncompleteForProduction)
    );
}

#[test]
fn yfinance_metadata_is_degraded_and_never_production_eligible() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.provider = "yfinance".into();
    metadata.dataset_id = "yfinance-SPY-1D-raw".into();
    metadata.quality_status = "degraded".into();
    metadata.production_eligible = false;
    assert!(validate_metadata(&metadata).is_ok());
    metadata.production_eligible = true;
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::MetadataIncompleteForProduction)
    );
}

#[test]
fn dataset_id_and_version_reject_spaces() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.dataset_id = "bad id".into();
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::InvalidDatasetId)
    );
    metadata.dataset_id = "approvedprovider-SPY-1D-raw".into();
    metadata.version = "bad version".into();
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::InvalidVersion)
    );
}

#[test]
fn provider_identity_cannot_be_mislabelled_for_known_providers() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.provider = "polygon".into();
    metadata.dataset_id = "yfinance-SPY-1D-raw".into();
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::MetadataIncompleteForProduction)
    );
}

#[test]
fn fallback_or_non_production_metadata_is_degraded_for_registry_status() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.production_eligible = false;
    metadata.quality_status = "degraded".into();
    assert_eq!(status_from_metadata(&metadata), DatasetStatus::Degraded);

    metadata.production_eligible = true;
    metadata.quality_status = "passed".into();
    metadata.gaps.missing_bars = 0;
    assert_eq!(status_from_metadata(&metadata), DatasetStatus::Healthy);
}

#[test]
fn dataset_id_must_encode_provider_instrument_timeframe_and_price_basis() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.dataset_id = "approvedprovider-QQQ-1D-raw".into();
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::MissingField("symbol_map"))
    );

    metadata.dataset_id = "approvedprovider-SPY-240-raw".into();
    metadata.timeframe = "240".into();
    let err = validate_metadata(&metadata);
    assert!(err.is_ok(), "unexpected validation error: {err:?}");

    metadata.dataset_id = "approvedprovider-SPY-240-tdl080-live200".into();
    metadata.price_basis = "raw".into();
    let err = validate_metadata(&metadata);
    assert!(
        err.is_ok(),
        "expected captured live200 dataset id suffix to preserve provider identity: {err:?}"
    );
}

#[test]
fn version_must_use_date_and_stable_suffix() {
    let mut metadata = metadata(DataType::Ohlcv);
    metadata.version = "v1".into();
    assert_eq!(
        validate_metadata(&metadata),
        Err(DataLakeError::InvalidVersion)
    );
    metadata.version = "20260101-run_1.abc".into();
    assert!(validate_metadata(&metadata).is_ok());
    metadata.version = "20240102-tv_native_1D_2".into();
    assert!(validate_metadata(&metadata).is_ok());
}

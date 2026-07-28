use super::*;
use crate::ohlcv::OhlcvBar;
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
fn capability_results_fail_closed_without_full_fetch() {
    let result = can_fetch_symbol_timeframe("stooq", "ES", "240", "2026-06-10T00:00:00Z");
    assert!(!result.can_fetch);
    assert!(!result.native_interval);
    assert!(result.unavailable_reason.is_some());
    assert!(!result.production_eligible);
}

#[test]
fn snapshots_are_stale_after_five_minutes() {
    assert!(snapshot_is_fresh(1_000, 1_300));
    assert!(!snapshot_is_fresh(1_000, 1_301));
    assert!(!snapshot_is_fresh(1_300, 1_000));
    assert_eq!(snapshot_freshness(None, 1_300), SnapshotFreshness::Missing);
    assert_eq!(
        snapshot_freshness(Some(1_000), 1_301),
        SnapshotFreshness::Stale
    );
}

#[test]
fn capability_maps_timeframes_and_unavailable_reasons_fail_closed() {
    let previous_key = std::env::var_os("POLYGON_API_KEY");
    unsafe { std::env::remove_var("POLYGON_API_KEY") };
    let four_hour = can_fetch_symbol_timeframe("stooq", "ES", "4H", "now");
    assert_eq!(four_hour.timeframe, "240");
    assert!(!four_hour.native_interval);
    assert!(!four_hour.can_fetch);
    assert!(
        four_hour
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("provider_blocked_or_unavailable")
    );

    let blocked = can_fetch_symbol_timeframe("polygon", "403", "1D", "now");
    assert_eq!(
        blocked.unavailable_reason.as_deref(),
        Some("provider blocked access")
    );
    assert!(!blocked.can_fetch);

    let unauthorized = can_fetch_symbol_timeframe("polygon", "401", "1D", "now");
    assert_eq!(
        unauthorized.unavailable_reason.as_deref(),
        Some("missing or invalid provider credentials")
    );
    assert!(!unauthorized.can_fetch);

    let missing_credentials = can_fetch_symbol_timeframe("polygon", "SPY", "1D", "now");
    assert!(missing_credentials.requires_credentials);
    assert_eq!(
        missing_credentials.unavailable_reason.as_deref(),
        Some("missing provider credentials")
    );
    assert!(!missing_credentials.can_fetch);

    let not_found = can_fetch_symbol_timeframe("polygon", "404", "1D", "now");
    assert_eq!(
        not_found.unavailable_reason.as_deref(),
        Some("provider symbol or endpoint not found")
    );
    assert!(!not_found.can_fetch);

    let fallback = can_fetch_symbol_timeframe("yfinance", "SPY", "1D", "now");
    assert_eq!(
        fallback.unavailable_reason.as_deref(),
        Some("yfinance fallback is degraded and ineligible for promotion")
    );
    assert!(!fallback.native_interval);
    assert!(!fallback.can_fetch);
    restore_env("POLYGON_API_KEY", previous_key);
}

#[test]
fn yfinance_interval_limitation_mapping_is_degraded_and_fail_closed() {
    let daily = can_fetch_symbol_timeframe("yfinance", "SPY", "1D", "now");
    assert!(daily.historical_supported);
    assert!(!daily.native_interval);
    assert!(!daily.production_eligible);
    assert!(!daily.can_fetch);
    assert_eq!(
        daily.unavailable_reason.as_deref(),
        Some("yfinance fallback is degraded and ineligible for promotion")
    );

    let unsupported = can_fetch_symbol_timeframe("yfinance", "SPY", "1M", "now");
    assert!(!unsupported.historical_supported);
    assert!(!unsupported.native_interval);
    assert!(!unsupported.production_eligible);
    assert_eq!(
        unsupported.unavailable_reason.as_deref(),
        Some("exact native interval is unsupported")
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

struct UnavailableProvider;

impl NativeOhlcvProvider for UnavailableProvider {
    fn can_fetch_symbol_timeframe(
        &self,
        symbol: &str,
        timeframe: &str,
        checked_at: &str,
    ) -> ProviderCapabilityResult {
        can_fetch_symbol_timeframe("stooq", symbol, timeframe, checked_at)
    }

    fn fetch_ohlcv_native(
        &self,
        _symbol: &str,
        _timeframe: &str,
        _start: &str,
        _end: &str,
    ) -> Result<Vec<OhlcvBar>, ProviderFetchError> {
        Err(ProviderFetchError {
            provider: "stooq".into(),
            action: "fetch_ohlcv_native",
            reason: "exact native fetch unavailable".into(),
        })
    }

    fn fetch_current_snapshot(&self, _symbol: &str) -> Result<CurrentSnapshot, ProviderFetchError> {
        Err(ProviderFetchError {
            provider: "stooq".into(),
            action: "fetch_current_snapshot",
            reason: "snapshot unavailable".into(),
        })
    }
}

#[test]
fn parses_stooq_csv_fixture_as_daily_native_bars() {
    let csv = b"Date,Open,High,Low,Close,Volume\n2026-01-02,470,472,469,471,1000\n";
    let normalized = String::from_utf8_lossy(csv)
        .replace("Date,", "timestamp,")
        .replace("Open,High,Low,Close,Volume", "open,high,low,close,volume")
        .replace("2026-01-02,", "2026-01-02T00:00:00Z,");
    let bars =
        crate::ohlcv::parse_ohlcv(normalized.as_bytes(), crate::ohlcv::OhlcvFormat::Csv).unwrap();
    assert_eq!(bars.len(), 1);
    assert_eq!(bars[0].timestamp, "2026-01-02T00:00:00Z");
    assert_eq!(bars[0].close, 471.0);
}

#[test]
fn stooq_html_block_fixture_fails_closed_without_bars() {
    let html = b"<!doctype html><html><body>verification required</body></html>";
    let result = crate::ohlcv::parse_ohlcv(html, crate::ohlcv::OhlcvFormat::Csv);
    assert!(result.is_err());
}

#[test]
fn stooq_non_daily_interval_refuses_resampling() {
    let result = can_fetch_symbol_timeframe("stooq", "SPY", "4H", "2026-06-10T00:00:00Z");
    assert_eq!(result.timeframe, "240");
    assert!(!result.native_interval);
    assert!(!result.can_fetch);
    assert!(!result.production_eligible);
    assert!(
        result
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .contains("resampling")
    );
}

#[test]
fn provider_trait_contract_fails_closed_without_download() {
    let provider = UnavailableProvider;
    assert!(
        provider
            .fetch_ohlcv_native("ES", "240", "2026-01-01", "2026-01-02")
            .is_err()
    );
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

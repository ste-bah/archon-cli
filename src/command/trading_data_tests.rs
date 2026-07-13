use super::*;

#[test]
fn normalizes_provider_native_csv_fixture() {
    let csv = b"timestamp,open,high,low,close,volume\n2026-01-01T00:00:00Z,10,12,9,11,100\n";
    let bars = parse_ohlcv(csv, OhlcvFormat::Csv).unwrap();
    assert_eq!(bars.len(), 1);
    assert_eq!(bars[0].timestamp, "2026-01-01T00:00:00Z");
}

#[test]
fn trading_data_status_and_show_dispatch_with_target() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(test_store_request()).unwrap();

    let status = render_data(&TradingCliDataAction::Status {
        target: Some(temp.path().to_path_buf()),
    })
    .unwrap();
    assert!(status.contains(".archon/trading-lab/data/registry.json"));

    let show = render_data(&TradingCliDataAction::Show {
        target: Some(temp.path().to_path_buf()),
        dataset_id: "manual-BTCUSD-1D-raw".into(),
        version: "20260101-fixture".into(),
        out: None,
    })
    .unwrap();
    assert!(show.contains("artifact_contract"));
}

#[test]
fn trading_data_list_json_dispatches_to_registry() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(test_store_request()).unwrap();

    let output = render_data(&TradingCliDataAction::List {
        target: Some(temp.path().to_path_buf()),
        json: true,
        out: None,
    })
    .unwrap();

    let registry: archon_trading::data_store::PersistentDatasetRegistry =
        serde_json::from_str(&output).unwrap();
    assert_eq!(registry.schema_version, "archon-trading-data-registry-v2");
    assert!(
        registry
            .datasets
            .contains_key("manual-BTCUSD-1D-raw:20260101-fixture")
    );
}

#[test]
fn trading_data_export_dispatches_to_dataset_bars() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(test_store_request()).unwrap();
    let out = temp.path().join("exported-bars.json");

    let output = render_data(&TradingCliDataAction::Export {
        target: Some(temp.path().to_path_buf()),
        dataset_id: "manual-BTCUSD-1D-raw".into(),
        version: "20260101-fixture".into(),
        out: out.clone(),
    })
    .unwrap();

    assert!(output.contains("Wrote Trading Lab report"));
    assert!(out.exists());
    let bars: Vec<archon_trading::ohlcv::OhlcvBar> =
        serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].timestamp, "2026-01-01T00:00:00Z");
}

#[test]
fn trading_data_ingest_dispatches_with_target() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("candles.csv");
    std::fs::write(
        &source,
        "timestamp,open,high,low,close,volume\n2026-01-01T00:00:00Z,10,11,9,10,1\n",
    )
    .unwrap();

    let output = render_data(&TradingCliDataAction::IngestOhlcv {
        target: Some(temp.path().to_path_buf()),
        source,
        format: TradingCliOhlcvFormat::Csv,
        dataset_id: "manual-BTCUSD-1D-raw".into(),
        version: "20260101-fixture".into(),
        provider: "manual".into(),
        symbol: "BTCUSD".into(),
        timezone: "UTC".into(),
        provider_symbol: None,
        asset_class: "crypto".into(),
        adjustment: "raw".into(),
        license: "research".into(),
        expected_bars: Some(1),
        timeframe: "1D".into(),
        native_interval: true,
        production_eligible: true,
        price_basis: "raw".into(),
        session: "24x7".into(),
        quality_status: "passed".into(),
        missing_bars: 0,
        optional: false,
        out: None,
    })
    .unwrap();

    assert!(output.contains("manual-BTCUSD-1D-raw"));
    assert!(TradingDataLake::new(temp.path()).registry_path().exists());
}

#[test]
fn trading_data_validate_dispatches_with_target() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(test_store_request()).unwrap();

    let output = render_data(&TradingCliDataAction::Validate {
        target: Some(temp.path().to_path_buf()),
        dataset_id: "manual-BTCUSD-1D-raw".into(),
        version: "20260101-fixture".into(),
        out: None,
    })
    .unwrap();

    let report: archon_trading::data_lake::ValidationReport =
        serde_json::from_str(&output).unwrap();
    assert_eq!(
        report.status,
        archon_trading::data_lake::ValidationStatus::Passed
    );
    assert!(report.production_eligible);
}

#[test]
fn trading_data_validate_surfaces_failed_validation() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(test_store_request()).unwrap();
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: DatasetMetadata =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    metadata.provider.clear();
    std::fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();

    let result = render_data(&TradingCliDataAction::Validate {
        target: Some(temp.path().to_path_buf()),
        dataset_id: "manual-BTCUSD-1D-raw".into(),
        version: "20260101-fixture".into(),
        out: None,
    });

    assert!(result.is_err());
    let report: archon_trading::data_lake::ValidationReport = serde_json::from_str(
        &std::fs::read_to_string(temp.path().join(&record.validation_path)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report.status,
        archon_trading::data_lake::ValidationStatus::Failed
    );
}

#[test]
fn manual_ingest_contract_requires_deterministic_id_and_version() {
    let input = IngestInput {
        target: None,
        source: Path::new("bars.csv"),
        format: TradingCliOhlcvFormat::Csv,
        dataset_id: "manual-SPY-1D-raw",
        version: "20260101-abc123",
        provider: "manual",
        symbol: "SPY",
        timezone: "UTC",
        provider_symbol: None,
        asset_class: "equity",
        timeframe: "1D",
        native_interval: true,
        production_eligible: true,
        price_basis: "raw",
        session: "regular",
        quality_status: "passed",
        adjustment: "raw",
        license: "research",
        expected_bars: None,
        missing_bars: 0,
        optional: false,
        out: None,
    };
    assert!(validate_dataset_contract(&input).is_ok());

    let mut invalid = input;
    invalid.dataset_id = "manual-SPY-4H-raw";
    assert!(validate_dataset_contract(&invalid).is_err());
    invalid.dataset_id = "manual-SPY-1D-raw";
    invalid.version = "v1";
    assert!(validate_dataset_contract(&invalid).is_err());
}

#[test]
fn fetch_native_reports_yfinance_degraded_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let output = render_data(&TradingCliDataAction::FetchNative {
        target: Some(temp.path().to_path_buf()),
        provider: "yfinance".into(),
        symbol: "SPY".into(),
        timeframe: "5".into(),
        start: "2024-01-01".into(),
        end: "2024-01-05".into(),
        dataset_id: "yfinance-SPY-5-raw".into(),
    })
    .unwrap();
    let report: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(report["provider"], "yfinance");
    assert_eq!(report["can_fetch"], false);
    assert_eq!(report["quality_status"], "degraded_fallback");
    assert_eq!(report["production_eligible"], false);
    assert_eq!(report["provider_blocked_or_unavailable"], true);
    assert!(
        report["unavailable_reason"]
            .as_str()
            .unwrap()
            .contains("unsupported native timeframe `5`")
    );
    assert!(!TradingDataLake::new(temp.path()).registry_path().exists());
}

fn test_store_request() -> StoreOhlcvRequest {
    StoreOhlcvRequest {
        metadata: DatasetMetadata {
            schema_version: "archon-trading-dataset-v2".into(),
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
            data_type: DataType::Ohlcv,
            symbol_map: BTreeMap::from([("BTCUSD".into(), "BTCUSD".into())]),
            timezone: "UTC".into(),
            adjustment: "raw".into(),
            license: "research".into(),
            coverage: CoverageWindow {
                start: String::new(),
                end: String::new(),
                expected_bars: 2,
                observed_bars: 0,
            },
            gaps: GapSummary {
                missing_bars: 0,
                expected_bars: 2,
            },
            checksum: String::new(),
            checksums: DatasetChecksums::default(),
            paths: DatasetArtifactPaths::default(),
            source: DatasetSourceMetadata::default(),
            quality_status: "passed".into(),
            created_at: String::new(),
            optional: false,
        },
        bars: vec![
            archon_trading::ohlcv::OhlcvBar {
                timestamp: "2026-01-01T00:00:00Z".into(),
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                volume: 1.0,
            },
            archon_trading::ohlcv::OhlcvBar {
                timestamp: "2026-01-02T00:00:00Z".into(),
                open: 11.0,
                high: 12.0,
                low: 10.0,
                close: 11.0,
                volume: 2.0,
            },
        ],
        raw_body: b"timestamp,open,high,low,close,volume\n".to_vec(),
        raw_format: OhlcvFormat::Csv,
        raw_request: json!({"source":"test"}),
        redacted_headers: json!({}),
        provider_notes: "test fixture".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

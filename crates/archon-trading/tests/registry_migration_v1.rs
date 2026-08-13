use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, DatasetStatus, GapSummary,
};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat};
use std::collections::BTreeMap;

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn request(version: &str, raw_body: &[u8], created_at: &str) -> StoreOhlcvRequest {
    StoreOhlcvRequest {
        metadata: DatasetMetadata {
            schema_version: "archon-trading-dataset-v1".into(),
            dataset_id: "manual-BTCUSD-1D-raw".into(),
            version: version.into(),
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
        bars: [10.0, 11.0]
            .into_iter()
            .enumerate()
            .map(|(index, close)| OhlcvBar {
                timestamp: format!("2026-01-0{}T00:00:00Z", index + 1),
                open: close,
                high: close + 1.0,
                low: close - 1.0,
                close,
                volume: close * 1_000.0,
            })
            .collect(),
        raw_body: raw_body.to_vec(),
        raw_format: OhlcvFormat::Csv,
        raw_request: serde_json::json!({"source":"test"}),
        redacted_headers: serde_json::json!({}),
        provider_notes: "test fixture".into(),
        created_at: created_at.into(),
    }
}

#[test]
fn counts_reconcile_and_second_run_is_byte_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let original = lake
        .store_ohlcv(request(
            "20260101-fixture",
            b"legacy raw",
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();
    let mut v1 = read_json(&lake.registry_path());
    v1["schema"] = serde_json::json!("archon-trading-data-registry-v1");
    let v1_bytes = serde_json::to_vec_pretty(&v1).unwrap();
    std::fs::write(lake.registry_path(), &v1_bytes).unwrap();

    lake.store_ohlcv(request(
        "20260102-fixture",
        b"new raw",
        "2026-01-02T00:00:00Z",
    ))
    .unwrap();

    let backup = lake
        .data_root()
        .join("registry.json.backup-2026-01-02T00_00_00Z");
    assert_eq!(std::fs::read(&backup).unwrap(), v1_bytes);
    let migrated = lake.load_registry().unwrap();
    assert_eq!(migrated.schema_version, "archon-trading-data-registry-v1");
    assert_eq!(migrated.datasets.len(), 2);
    let preserved = migrated
        .datasets
        .get(&format!("{}:{}", original.dataset_id, original.version))
        .unwrap();
    assert_eq!(preserved.status, DatasetStatus::Healthy);
    assert!(preserved.native_interval);
    assert!(preserved.production_eligible);

    let first_registry = std::fs::read(lake.registry_path()).unwrap();
    let first_report = lake.migration_report().unwrap();
    let second_report = lake.migration_report().unwrap();
    assert_eq!(first_report, second_report);
    assert_eq!(first_report.migrated, 0);
    assert_eq!(first_report.skipped, 0);
    assert_eq!(first_report.degraded, 0);
    assert_eq!(first_report.failed, 0);
    assert_eq!(std::fs::read(lake.registry_path()).unwrap(), first_registry);
}

#[test]
fn populated_v1_preserves_backup_records_and_artifact_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let original = lake
        .store_ohlcv(request("ignored", b"legacy raw", "2026-01-01T00:00:00Z"))
        .unwrap();
    let raw_before = std::fs::read(temp.path().join(&original.raw_response_path)).unwrap();
    let mut v1 = read_json(&lake.registry_path());
    v1["schema"] = serde_json::json!("archon-trading-data-registry-v1");
    let v1_bytes = serde_json::to_vec_pretty(&v1).unwrap();
    std::fs::write(lake.registry_path(), &v1_bytes).unwrap();

    lake.store_ohlcv(request("ignored", b"next raw", "2026-01-02T00:00:00Z"))
        .unwrap();
    let backup = lake
        .data_root()
        .join("registry.json.backup-2026-01-02T00_00_00Z");
    assert_eq!(std::fs::read(backup).unwrap(), v1_bytes);
    assert_eq!(
        std::fs::read(temp.path().join(&original.raw_response_path)).unwrap(),
        raw_before
    );
    assert!(
        lake.load_registry()
            .unwrap()
            .datasets
            .keys()
            .any(|key| key.starts_with(&original.dataset_id))
    );
}

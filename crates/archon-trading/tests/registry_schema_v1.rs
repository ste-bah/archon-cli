use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, GapSummary,
};
use archon_trading::data_store::{DataStoreError, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat};
use std::collections::BTreeMap;

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn request(
    version: &str,
    raw_body: &[u8],
    created_at: &str,
) -> archon_trading::data_store::StoreOhlcvRequest {
    archon_trading::data_store::StoreOhlcvRequest {
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
fn writes_strict_v1_registry_with_relative_artifact_paths() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake
        .store_ohlcv(request(
            "20260101-fixture",
            b"exact raw bytes",
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();

    let registry = read_json(&lake.registry_path());
    assert_eq!(registry["schema"], "archon-trading-data-registry-v1");
    assert!(registry.get("schema_version").is_none());
    for path in [
        &record.dataset_path,
        &record.metadata_path,
        &record.normalized_path,
        &record.raw_response_path,
        &record.raw_request_path,
        &record.redacted_headers_path,
        &record.provider_notes_path,
        &record.validation_path,
        &record.manifest_path,
    ] {
        assert!(!std::path::Path::new(path).is_absolute());
        assert!(!path.split('/').any(|part| part == ".."));
    }
}

#[test]
fn complete_artifacts_precede_registry_commit() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake
        .store_ohlcv(request(
            "ignored",
            b"exact raw bytes",
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();
    for path in [
        &record.metadata_path,
        &record.normalized_path,
        &record.raw_response_path,
        &record.manifest_path,
    ] {
        assert!(temp.path().join(path).is_file());
    }
    assert!(lake.registry_path().is_file());
}

#[test]
fn interrupted_publication_preserves_prior_registry_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request("ignored", b"first", "2026-01-01T00:00:00Z"))
        .unwrap();
    let before = std::fs::read(lake.registry_path()).unwrap();
    std::fs::create_dir(lake.registry_path().with_extension("tmp")).unwrap();

    assert!(
        lake.store_ohlcv(request("ignored", b"second", "2026-01-02T00:00:00Z"))
            .is_err()
    );
    assert_eq!(std::fs::read(lake.registry_path()).unwrap(), before);
}

#[test]
fn rejects_traversal_and_symlink_artifacts_without_rewriting_registry() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake
        .store_ohlcv(request(
            "20260101-fixture",
            b"exact raw bytes",
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();
    let registry_before = std::fs::read(lake.registry_path()).unwrap();
    let mut registry = read_json(&lake.registry_path());
    let key = format!("{}:{}", record.dataset_id, record.version);
    registry["datasets"][&key]["metadata_path"] = serde_json::json!("../outside.json");
    std::fs::write(
        &lake.registry_path(),
        serde_json::to_vec_pretty(&registry).unwrap(),
    )
    .unwrap();
    let malicious_bytes = std::fs::read(lake.registry_path()).unwrap();

    let result = lake.load_registry();
    assert!(matches!(
        result,
        Err(DataStoreError::IncompleteArtifactContract(_))
    ));
    assert_eq!(
        std::fs::read(lake.registry_path()).unwrap(),
        malicious_bytes
    );

    std::fs::write(&lake.registry_path(), &registry_before).unwrap();
    let metadata_path = temp.path().join(&record.metadata_path);
    let real_path = metadata_path.with_extension("real.json");
    std::fs::rename(&metadata_path, &real_path).unwrap();
    create_symlink(&real_path, &metadata_path);
    assert!(matches!(
        lake.load_registry(),
        Err(DataStoreError::IncompleteArtifactContract(_))
    ));
    assert_eq!(
        std::fs::read(lake.registry_path()).unwrap(),
        registry_before
    );
}

#[cfg(unix)]
fn create_symlink(target: &std::path::Path, link: &std::path::Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn create_symlink(target: &std::path::Path, link: &std::path::Path) {
    std::os::windows::fs::symlink_file(target, link).unwrap();
}

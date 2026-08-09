use super::*;
use crate::data_lake::{CoverageWindow, DataType, GapSummary};
#[test]
fn first_dataset_write_initializes_missing_registry_under_data_root() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    assert!(!lake.registry_path().exists());
    lake.store_ohlcv(request()).unwrap();
    assert_eq!(lake.registry_path(), lake.data_root().join("registry.json"));
    assert!(lake.registry_path().exists());
}
#[test]
fn stores_and_loads_ohlcv_dataset() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    assert_eq!(record.bars, 2);
    let loaded = lake
        .load_ohlcv("manual-BTCUSD-1D-raw", "20260101-fixture")
        .unwrap();
    assert_eq!(loaded.bars.len(), 2);
    assert!(lake.registry_path().exists());
    assert_eq!(
        lake.registry_path(),
        temp.path().join(".archon/trading-lab/data/registry.json")
    );
    let registry = lake.status().unwrap();
    assert_eq!(registry.schema_version, REGISTRY_SCHEMA_V2);
    assert!(temp.path().join(&record.validation_path).exists());
    assert!(temp.path().join(&record.manifest_path).exists());
    assert_eq!(record.symbol, "BTCUSD");
    assert_eq!(record.timeframe, "1D");
    assert!(record.native_interval);
    assert!(record.production_eligible);
    assert!(record.raw_path.ends_with("raw/response.csv"));
    for required in [
        "metadata.json",
        "ohlcv.jsonl",
        "raw/response.csv",
        "raw/request.json",
        "raw/headers.redacted.json",
        "raw/provider-notes.md",
        "validation.json",
        "manifest.json",
    ] {
        assert!(
            temp.path()
                .join(&record.dataset_path)
                .join(required)
                .exists(),
            "missing required artifact {required}"
        );
    }
}
#[test]
fn metadata_json_contains_self_describing_paths_and_checksums() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let metadata: DatasetMetadata = read_json(&temp.path().join(&record.metadata_path)).unwrap();

    assert_eq!(metadata.paths.raw, record.raw_path);
    assert_eq!(metadata.paths.raw_response, record.raw_response_path);
    assert_eq!(metadata.paths.raw_request, record.raw_request_path);
    assert_eq!(
        metadata.paths.redacted_headers,
        record.redacted_headers_path
    );
    assert_eq!(metadata.paths.provider_notes, record.provider_notes_path);
    assert_eq!(metadata.paths.normalized, record.normalized_path);
    assert_eq!(metadata.paths.validation, record.validation_path);
    assert_eq!(metadata.paths.manifest, record.manifest_path);
    assert_eq!(metadata.checksums.raw_sha256, bytes_checksum(b"raw"));
    let normalized = std::fs::read(temp.path().join(&record.normalized_path)).unwrap();
    assert_eq!(
        metadata.checksums.normalized_sha256,
        bytes_checksum(&normalized)
    );
    assert_eq!(metadata.checksum, metadata.checksums.normalized_sha256);
    assert_eq!(record.metadata_checksum, metadata.checksums.metadata_sha256);
    assert_eq!(record.raw_checksum, metadata.checksums.raw_sha256);
    assert!(
        record
            .dataset_path
            .ends_with("manual-BTCUSD-1D-raw/20260101-fixture")
    );
    assert!(!metadata.checksums.metadata_sha256.is_empty());
    assert_eq!(metadata.source.license_notes, "research");
    assert_eq!(metadata.created_at, "2026-01-01T00:00:00Z");
}

#[test]
fn d49_normalized_artifact_follows_manifest_pointer_without_literal_directory() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let manifest: serde_json::Value = read_json(&temp.path().join(&record.manifest_path)).unwrap();
    let normalized_path = manifest["normalized_path"]
        .as_str()
        .expect("manifest normalized_path");

    assert_eq!(normalized_path, record.normalized_path);
    assert!(temp.path().join(normalized_path).is_file());
    assert!(
        !temp
            .path()
            .join(&record.dataset_path)
            .join("normalized")
            .exists()
    );
}

#[test]
fn identical_content_reuses_existing_dataset_version() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let first = lake.store_ohlcv(request()).unwrap();
    let second = lake.store_ohlcv(request()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn changed_content_refuses_existing_dataset_version() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    let mut changed = request();
    changed.bars[1].close = 12.0;
    let result = lake.store_ohlcv(changed);
    assert!(matches!(
        result,
        Err(DataStoreError::InvalidMetadata(message))
            if message.contains("different normalized checksum")
    ));
}

#[test]
fn provider_capability_persists_fail_closed_result() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let previous_key = std::env::var_os("POLYGON_API_KEY");
    unsafe { std::env::remove_var("POLYGON_API_KEY") };
    let result = lake
        .persist_capability("polygon", "SPY", "1D", "2026-01-01T00:00:00Z")
        .unwrap();
    restore_env("POLYGON_API_KEY", previous_key);

    assert!(!result.can_fetch);
    assert!(!result.production_eligible);
    assert!(result.requires_credentials);
    assert!(result.missing_credentials);
    assert_eq!(result.credential_state, "missing");
    assert!(lake.provider_capability_latest_path().exists());

    let text = std::fs::read_to_string(lake.provider_capabilities_path()).unwrap();
    assert!(text.contains("provider"));
    assert!(text.contains("symbol"));
    assert!(text.contains("timeframe"));
    assert!(text.contains("native_interval"));
    assert!(text.contains("can_fetch"));
    assert!(text.contains("unavailable_reason"));
    assert!(text.contains("checked_at"));
    let latest_text = std::fs::read_to_string(lake.provider_capability_latest_path()).unwrap();
    assert!(latest_text.contains("provider_environment"));
    assert!(latest_text.contains("POLYGON_API_KEY"));
    assert!(latest_text.contains("OPENBB_API_URL"));
    assert!(latest_text.contains("missing"));
    assert!(!latest_text.contains("do-not-store"));
}

#[test]
fn provider_capability_does_not_create_dataset_registry_entries() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.persist_capability("unknown-provider", "ES", "1D", "2026-01-01T00:00:00Z")
        .unwrap();

    assert!(lake.load_registry().unwrap().datasets.is_empty());
    assert!(lake.provider_capability_latest_path().exists());
}

#[test]
fn capability_missing_native_interval_treats_can_fetch_false() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let result = lake
        .persist_capability("tradingview", "ES", "5", "2026-01-01T00:00:00Z")
        .unwrap();

    assert!(!result.native_interval);
    assert!(!result.production_eligible);
    assert!(!result.can_fetch);
    assert!(result.unsupported);
}

#[test]
fn redaction_rejects_secret_material_in_provider_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.provider = "polygon".into();
    request.metadata.dataset_id = "polygon-SPY-1D-raw".into();
    request.raw_request = serde_json::json!({"api_key":"do-not-store"});
    let result = lake.store_ohlcv(request);
    assert!(matches!(result, Err(DataStoreError::InvalidMetadata(_))));
    assert!(lake.status().unwrap().datasets.is_empty());
}

#[test]
fn v1_registry_migrates_with_backup_on_first_write() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    std::fs::create_dir_all(lake.data_root()).unwrap();
    std::fs::write(
        lake.registry_path(),
        r#"{"schema_version":"archon-trading-data-registry-v1","datasets":{},"last_updated":"old"}"#,
    )
    .unwrap();
    lake.store_ohlcv(request()).unwrap();
    assert_eq!(
        lake.load_registry().unwrap().schema_version,
        REGISTRY_SCHEMA_V2
    );
    assert!(std::fs::read_dir(lake.data_root()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("registry.json.backup-")
    }));
    let registry_text = std::fs::read_to_string(lake.registry_path()).unwrap();
    assert!(registry_text.contains("\"snapshots\""));
    let registry_json: serde_json::Value = serde_json::from_str(&registry_text).unwrap();
    assert_eq!(registry_json["schema"], REGISTRY_SCHEMA_V2);
    assert!(registry_json.get("schema_version").is_none());
    let report = lake.migration_report().unwrap();
    assert_eq!(report.schema_version, REGISTRY_SCHEMA_V2);
    assert_eq!(report.migrated, 1);
    assert!(report.backup_path.is_some());
}

#[test]
fn non_empty_v1_registry_migration_preserves_and_degrades_records() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let existing = lake.store_ohlcv(request()).unwrap();
    let mut registry = lake.load_registry().unwrap();
    registry.schema_version = REGISTRY_SCHEMA_V1.into();
    write_json(&lake.registry_path(), &registry).unwrap();
    std::fs::remove_file(temp.path().join(&existing.validation_path)).unwrap();
    std::fs::remove_file(temp.path().join(&existing.manifest_path)).unwrap();

    let mut next = request();
    next.metadata.version = "20260102-fixture".into();
    next.created_at = "2026-01-02T00:00:00Z".into();
    lake.store_ohlcv(next).unwrap();

    let migrated = lake.load_registry().unwrap();
    let preserved = migrated
        .datasets
        .get(&registry_key(&existing.dataset_id, &existing.version))
        .unwrap();
    assert_eq!(migrated.schema_version, REGISTRY_SCHEMA_V2);
    assert_eq!(preserved.dataset_id, existing.dataset_id);
    assert_eq!(preserved.status, DatasetStatus::Degraded);
    assert!(temp.path().join(&preserved.validation_path).exists());
    assert!(temp.path().join(&preserved.manifest_path).exists());
    let migrated_metadata: DatasetMetadata =
        read_json(&temp.path().join(&preserved.metadata_path)).unwrap();
    assert!(!migrated_metadata.native_interval);
    assert!(!migrated_metadata.production_eligible);
    assert_eq!(migrated_metadata.quality_status, "degraded");
    assert_eq!(preserved.symbol, "BTCUSD");
    assert_eq!(preserved.timeframe, "1D");
    assert!(!preserved.native_interval);
    assert!(!preserved.production_eligible);
    let report = lake.migration_report().unwrap();
    assert_eq!(report.migrated, 2);
    assert_eq!(report.skipped, 2);
    assert_eq!(report.degraded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(
        report.report_path.as_deref(),
        Some(".archon/trading-lab/data/registry-migration-report.json")
    );
    assert!(temp.path().join(report.report_path.unwrap()).exists());
}

#[test]
fn v2_migration_report_is_idempotent_and_skips_existing_v2() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();

    let first = lake.migration_report().unwrap();
    let second = lake.migration_report().unwrap();
    assert_eq!(first, second);
    assert_eq!(second.schema_version, REGISTRY_SCHEMA_V2);
    assert_eq!(lake.registry_path(), lake.data_root().join("registry.json"));
    assert_eq!(second.migrated, 1);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.failed, 0);
    let report_json: serde_json::Value =
        read_json(&lake.data_root().join("registry-migration-report.json")).unwrap();
    assert_eq!(report_json["schema"], REGISTRY_SCHEMA_V2);
    assert!(report_json.get("schema_version").is_none());
    assert_eq!(
        second.report_path.as_deref(),
        Some(".archon/trading-lab/data/registry-migration-report.json")
    );
    assert!(temp.path().join(second.report_path.unwrap()).exists());
}

#[test]
fn registry_write_creates_backup_before_atomic_replace() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();

    let mut next = request();
    next.metadata.version = "20260104-fixture".into();
    next.created_at = "2026-01-04T00:00:00Z".into();
    lake.store_ohlcv(next).unwrap();

    let backup = lake
        .data_root()
        .join("registry.json.backup-2026-01-04T00_00_00Z");
    assert!(backup.exists());
    let backup_registry: PersistentDatasetRegistry = read_json(&backup).unwrap();
    assert_eq!(backup_registry.datasets.len(), 1);
    let registry: PersistentDatasetRegistry = read_json(&lake.registry_path()).unwrap();
    assert_eq!(registry.datasets.len(), 2);
}

#[test]
fn interrupted_registry_write_temp_file_does_not_replace_registry() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    let original_registry = std::fs::read_to_string(lake.registry_path()).unwrap();
    let interrupted_temp = lake.registry_path().with_extension("tmp");
    std::fs::write(
        &interrupted_temp,
        r#"{"schema":"archon-trading-data-registry-v2","datasets":{}}"#,
    )
    .unwrap();

    let loaded = lake.load_registry().unwrap();

    assert_eq!(loaded.datasets.len(), 1);
    assert_eq!(
        std::fs::read_to_string(lake.registry_path()).unwrap(),
        original_registry
    );
    assert!(interrupted_temp.exists());
}

#[test]
fn load_ohlcv_fails_closed_when_artifact_metadata_is_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: DatasetMetadata = read_json(&metadata_path).unwrap();
    metadata.production_eligible = true;
    metadata.checksums.raw_sha256.clear();
    write_json(&metadata_path, &metadata).unwrap();

    let result = lake.load_ohlcv("manual-BTCUSD-1D-raw", "20260101-fixture");
    assert!(matches!(
        result,
        Err(DataStoreError::IncompleteArtifactContract(message))
            if message.contains("checksum chain mismatch")
    ));
}

#[test]
fn status_fails_closed_when_registry_artifact_contract_is_incomplete() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    std::fs::remove_file(temp.path().join(&record.redacted_headers_path)).unwrap();

    let result = lake.status();
    assert!(matches!(
        result,
        Err(DataStoreError::IncompleteArtifactContract(path))
            if path.ends_with("headers.redacted.json")
    ));
}

#[test]
fn v1_metadata_missing_v2_flags_migrates_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let existing = lake.store_ohlcv(request()).unwrap();
    let mut registry = lake.load_registry().unwrap();
    registry.schema_version = REGISTRY_SCHEMA_V1.into();
    write_json(&lake.registry_path(), &registry).unwrap();

    let metadata_path = temp.path().join(&existing.metadata_path);
    let mut metadata_json: serde_json::Value = read_json(&metadata_path).unwrap();
    metadata_json
        .as_object_mut()
        .unwrap()
        .remove("native_interval");
    metadata_json
        .as_object_mut()
        .unwrap()
        .remove("production_eligible");
    write_json(&metadata_path, &metadata_json).unwrap();

    let mut next = request();
    next.metadata.version = "20260103-fixture".into();
    next.created_at = "2026-01-03T00:00:00Z".into();
    lake.store_ohlcv(next).unwrap();

    let migrated = lake
        .load_ohlcv("manual-BTCUSD-1D-raw", "20260101-fixture")
        .unwrap();
    assert_eq!(migrated.record.status, DatasetStatus::Degraded);
    assert_eq!(migrated.record.schema_version, REGISTRY_SCHEMA_V2);
    assert!(!migrated.record.dataset_path.is_empty());
    assert!(!migrated.record.metadata_checksum.is_empty());
    assert!(!migrated.record.raw_checksum.is_empty());
    assert!(!migrated.metadata.native_interval);
    assert!(!migrated.metadata.production_eligible);
    assert_eq!(migrated.metadata.quality_status, "degraded");
}

fn request() -> StoreOhlcvRequest {
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
            bar("2026-01-01T00:00:00Z", 10.0),
            bar("2026-01-02T00:00:00Z", 11.0),
        ],
        raw_body: b"raw".to_vec(),
        raw_format: OhlcvFormat::Csv,
        raw_request: serde_json::json!({"source":"test"}),
        redacted_headers: serde_json::json!({}),
        provider_notes: "test fixture".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: close * 1_000.0,
    }
}

/// Registry status is DERIVED on every load, so a quarantine that lives only in
/// metadata is silently undone by the next read. Live installation: 33 datasets
/// were marked quarantined and every one of them was stamped `Healthy` again,
/// because their validation.json still said `passed`. Marking a dataset
/// untrustworthy has to survive the reconciliation that follows it.
#[test]
fn a_quarantined_dataset_is_never_reconciled_back_to_healthy() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();

    // Baseline: this dataset earns Healthy on its own merits.
    let before = lake.status().unwrap();
    let key = registry_key(&record.dataset_id, &record.version);
    assert_eq!(
        before.datasets[&key].status,
        DatasetStatus::Healthy,
        "fixture must start Healthy or the test proves nothing"
    );
    assert!(before.datasets[&key].production_eligible);

    // Quarantine it the way an operator does — a marker in its metadata, with
    // the validation report left untouched and still claiming it passed.
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    metadata["quarantined_at"] = serde_json::json!("2026-08-09T09:00:00Z");
    metadata["quarantine_reason"] = serde_json::json!("provenance unprovable");
    std::fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();

    let after = lake.status().unwrap();
    assert_eq!(
        after.datasets[&key].status,
        DatasetStatus::Degraded,
        "a quarantined dataset must not be reconciled back to Healthy"
    );
    assert!(
        !after.datasets[&key].production_eligible,
        "nor may it stay production-eligible"
    );

    // And it must STAY demoted across repeated loads — the reconciliation
    // rewrites the registry, so a fix that only held for one read is no fix.
    let again = lake.status().unwrap();
    assert_eq!(again.datasets[&key].status, DatasetStatus::Degraded);
}

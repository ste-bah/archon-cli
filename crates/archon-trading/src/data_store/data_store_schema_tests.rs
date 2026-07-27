use super::*;
use crate::data_lake::CoverageWindow;

#[test]
fn schema_artifact_replaces_legacy_schema_version() {
    let value = serde_json::json!({
        "schema_version": "archon-example-v1",
        "records": { "one": { "status": "accepted" } }
    });

    let artifact = schema_artifact_value(&value).unwrap();

    assert_eq!(artifact["schema"], "archon-example-v1");
    assert!(artifact.get("schema_version").is_none());
    assert_eq!(artifact["records"]["one"]["status"], "accepted");
}

#[test]
fn provider_capabilities_write_envelope_and_read_legacy_maps() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let result = capability_result("2026-07-13T00:00:00Z");

    lake.persist_capability_result(result.clone()).unwrap();
    let artifact: serde_json::Value = read_json(&lake.provider_capabilities_path()).unwrap();

    assert_eq!(artifact["schema"], "archon-provider-capabilities-v1");
    assert_eq!(artifact["checked_at"], result.checked_at);
    assert_eq!(artifact["capabilities"].as_object().unwrap().len(), 1);
    assert!(artifact.get("schema_version").is_none());

    let legacy = serde_json::json!({ "polygon:ES:1D": result });
    write_json(&lake.provider_capabilities_path(), &legacy).unwrap();
    assert_eq!(lake.load_capabilities().unwrap().len(), 1);
}

#[test]
fn provider_capabilities_mark_fetchable_records_available() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut result = capability_result("2026-07-13T00:00:00Z");
    result.can_fetch = true;
    result.production_eligible = true;
    result.requires_credentials = false;
    result.missing_credentials = false;
    result.current_snapshot_supported = true;
    result.credential_state = "not_required".into();
    result.unavailable_reason = None;

    lake.persist_capability_result(result.clone()).unwrap();
    let artifact: serde_json::Value = read_json(&lake.provider_capabilities_path()).unwrap();
    let record = &artifact["capabilities"]["polygon:ES:1D"];

    assert_eq!(record["registry_status"], "Available");
    assert_eq!(record["registry_dataset_id"], "polygon:ES:1D");
    assert_eq!(record["registry_version"], result.checked_at);
}

#[test]
fn schema_artifact_requires_a_declared_schema() {
    let error = schema_artifact_value(&serde_json::json!({ "records": {} })).unwrap_err();

    assert!(matches!(error, DataStoreError::InvalidMetadata(_)));
}

#[test]
fn registry_writer_stamps_canonical_dataset_status_casing() {
    let value = serde_json::json!({
        "schema": REGISTRY_SCHEMA_V2,
        "datasets": {
            "one:v1": {"status": "healthy"},
            "two:v1": {"status": "DEGRADED"},
        },
    });

    let artifact = schema_artifact_value(&value).unwrap();

    assert_eq!(artifact["datasets"]["one:v1"]["status"], "Healthy");
    assert_eq!(artifact["datasets"]["two:v1"]["status"], "Degraded");
    assert!(serde_json::from_value::<DatasetStatus>(serde_json::json!("healthy")).is_err());
}

#[test]
fn legacy_quarantined_registry_status_deserializes_as_degraded() {
    let status: DatasetStatus = serde_json::from_value(serde_json::json!("Quarantined"))
        .expect("legacy Quarantined registry status must migrate fail-closed as Degraded");

    assert_eq!(status, DatasetStatus::Degraded);
}

#[test]
fn registry_fails_closed_on_unknown_native_or_production_fields() {
    let registry = serde_json::json!({
        "schema": REGISTRY_SCHEMA_V2,
        "datasets": {
            "manual-BTCUSD-1D-raw:20260101-fixture": registry_record_with_extra(
                "unknown_native_field",
            ),
        },
        "snapshots": {},
        "last_updated": "2026-01-01T00:00:00Z",
    });

    let error = serde_json::from_value::<PersistentDatasetRegistry>(registry).unwrap_err();
    assert!(error.to_string().contains("unknown_native_field"));

    let registry = serde_json::json!({
        "schema": REGISTRY_SCHEMA_V2,
        "datasets": {
            "manual-BTCUSD-1D-raw:20260101-fixture": registry_record_with_extra(
                "unknown_production_field",
            ),
        },
        "snapshots": {},
        "last_updated": "2026-01-01T00:00:00Z",
    });

    let error = serde_json::from_value::<PersistentDatasetRegistry>(registry).unwrap_err();
    assert!(error.to_string().contains("unknown_production_field"));
}

#[test]
fn dataset_metadata_fails_closed_on_unknown_native_or_production_fields() {
    let metadata = serde_json::json!({
        "schema": "archon-trading-dataset-v2",
        "dataset_id": "manual-BTCUSD-1D-raw",
        "version": "20260101-fixture",
        "provider": "manual",
        "native_interval": true,
        "production_eligible": true,
        "data_type": "Ohlcv",
        "symbol_map": {"BTCUSD": "BTCUSD"},
        "timezone": "UTC",
        "adjustment": "raw",
        "license": "research",
        "coverage": {
            "start": "2026-01-01T00:00:00Z",
            "end": "2026-01-02T00:00:00Z",
            "expected_bars": 2,
            "observed_bars": 2
        },
        "gaps": {"missing_bars": 0, "expected_bars": 2},
        "checksum": "sha256",
        "optional": false,
        "unknown_production_field": true,
    });

    let error = serde_json::from_value::<DatasetMetadata>(metadata).unwrap_err();
    assert!(error.to_string().contains("unknown_production_field"));

    let coverage = serde_json::json!({
        "start": "2026-01-01T00:00:00Z",
        "end": "2026-01-02T00:00:00Z",
        "expected_bars": 2,
        "observed_bars": 2,
        "unknown_native_field": true,
    });

    let error = serde_json::from_value::<CoverageWindow>(coverage).unwrap_err();
    assert!(error.to_string().contains("unknown_native_field"));
}

fn registry_record_with_extra(extra_field: &str) -> serde_json::Value {
    let mut record = serde_json::json!({
        "dataset_id": "manual-BTCUSD-1D-raw",
        "version": "20260101-fixture",
        "schema": REGISTRY_SCHEMA_V2,
        "dataset_path": ".archon/trading-lab/data/datasets/manual-BTCUSD-1D-raw/20260101-fixture",
        "metadata_checksum": "metadata-sha256",
        "raw_checksum": "raw-sha256",
        "validation_checksum": "validation-sha256",
        "raw_response_path": "raw/response.csv",
        "raw_request_path": "raw/request.json",
        "redacted_headers_path": "raw/headers.redacted.json",
        "provider_notes_path": "raw/provider-notes.md",
        "provider": "manual",
        "data_type": "Ohlcv",
        "symbol": "BTCUSD",
        "timeframe": "1D",
        "native_interval": true,
        "production_eligible": true,
        "status": "Healthy",
        "checksum": "normalized-sha256",
        "bars": 2,
        "coverage_start": "2026-01-01T00:00:00Z",
        "coverage_end": "2026-01-02T00:00:00Z",
        "metadata_path": "metadata.json",
        "normalized_path": "ohlcv.jsonl",
        "raw_path": "raw/response.csv",
        "validation_path": "validation.json",
        "manifest_path": "manifest.json",
        "created_at": "2026-01-01T00:00:00Z"
    });
    record
        .as_object_mut()
        .unwrap()
        .insert(extra_field.into(), true.into());
    record
}

fn capability_result(checked_at: &str) -> ProviderCapabilityResult {
    ProviderCapabilityResult {
        provider: "polygon".into(),
        symbol: "ES".into(),
        canonical_instrument: "ES".into(),
        provider_symbol: "I:ES".into(),
        timeframe: "1D".into(),
        native_interval: true,
        production_eligible: false,
        can_fetch: false,
        current_snapshot_supported: false,
        historical_supported: true,
        history_horizon: None,
        requires_credentials: true,
        missing_credentials: true,
        provider_blocked: false,
        unsupported: false,
        credential_state: "missing".into(),
        unavailable_reason: Some("missing credentials".into()),
        checked_at: checked_at.into(),
    }
}

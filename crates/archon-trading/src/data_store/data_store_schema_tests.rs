use super::*;

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
fn schema_artifact_requires_a_declared_schema() {
    let error = schema_artifact_value(&serde_json::json!({ "records": {} })).unwrap_err();

    assert!(matches!(error, DataStoreError::InvalidMetadata(_)));
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

use super::*;

#[test]
fn validation_rejects_secret_material_without_publishing_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let bars = vec![
        bar("2026-01-01T00:00:00Z", 10.0, 100.0),
        bar("2026-01-02T00:00:00Z", 11.0, 110.0),
    ];
    let request = StoreOhlcvRequest {
        metadata: complete_metadata(&bars),
        bars,
        raw_body: b"captured provider response".to_vec(),
        raw_format: OhlcvFormat::Json,
        raw_request: serde_json::json!({"api_key": "must-not-be-persisted"}),
        redacted_headers: serde_json::json!({}),
        provider_notes: "captured provider response".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    };

    let error = lake
        .store_ohlcv(request)
        .expect_err("secret-bearing evidence must fail closed");

    assert!(
        matches!(&error, DataStoreError::InvalidMetadata(message) if message == "secret material rejected")
    );
    assert!(!lake.data_root().join("datasets").exists());
    let persisted = std::fs::read_dir(temp.path()).unwrap().count();
    assert_eq!(
        persisted, 0,
        "rejected secret material left persisted state"
    );
    assert!(!format!("{error:?}").contains("must-not-be-persisted"));
}

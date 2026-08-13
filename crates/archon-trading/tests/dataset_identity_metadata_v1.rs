use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, GapSummary,
};
use archon_trading::data_store::{DataStoreError, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat, bytes_checksum};
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
fn identity_and_version_are_raw_checksum_bound() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let original = request(
        "derive",
        b"first raw representation",
        "2026-01-01T00:00:00Z",
    );
    let record = lake.store_ohlcv(original.clone()).unwrap();
    assert_eq!(record.raw_checksum, bytes_checksum(&original.raw_body));
    assert_eq!(record.dataset_id, "manual-BTCUSD-1D-raw");
    assert_eq!(
        record.version,
        format!("20260101-{}", &record.raw_checksum[..8])
    );
    assert_eq!(
        std::fs::read(temp.path().join(&record.raw_response_path)).unwrap(),
        original.raw_body
    );

    let collision = request(
        "derive",
        b"different bytes, identical bars",
        "2026-01-01T00:00:00Z",
    );
    let changed = lake.store_ohlcv(collision).unwrap();
    assert_ne!(changed.version, record.version);
    assert_eq!(
        changed.version,
        format!("20260101-{}", &changed.raw_checksum[..8])
    );

    let metadata = read_json(&temp.path().join(&record.metadata_path));
    assert_eq!(metadata["checksums"]["raw_sha256"], record.raw_checksum);
    assert_eq!(metadata["paths"]["raw_response"], record.raw_response_path);
    assert_eq!(metadata["source"]["retrieved_at"], "2026-01-01T00:00:00Z");
    let serialized = serde_json::to_string(&metadata)
        .unwrap()
        .to_ascii_lowercase();
    assert!(!serialized.contains("do-not-store"));
}

#[test]
fn rejects_secrets_in_metadata_and_provider_notes_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut metadata_secret = request("20260101-fixture", b"raw", "2026-01-01T00:00:00Z");
    metadata_secret.metadata.source.url_or_endpoint =
        "https://example.test?api_key=do-not-store".into();
    assert!(matches!(
        lake.store_ohlcv(metadata_secret),
        Err(DataStoreError::InvalidMetadata(_))
    ));
    assert!(!lake.registry_path().exists());

    let mut notes_secret = request("20260101-fixture", b"raw", "2026-01-01T00:00:00Z");
    notes_secret.provider_notes = "Authorization: Bearer do-not-store".into();
    assert!(matches!(
        lake.store_ohlcv(notes_secret),
        Err(DataStoreError::InvalidMetadata(_))
    ));
    assert!(!lake.registry_path().exists());

    let raw_secret = request(
        "20260101-fixture",
        b"Authorization: Bearer do-not-store",
        "2026-01-01T00:00:00Z",
    );
    assert!(matches!(
        lake.store_ohlcv(raw_secret),
        Err(DataStoreError::InvalidMetadata(_))
    ));
    assert!(!lake.registry_path().exists());
}

#[test]
fn metadata_is_complete_deterministic_relative_and_secret_free() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake
        .store_ohlcv(request("ignored", b"raw", "2026-01-01T00:00:00Z"))
        .unwrap();
    let first = std::fs::read(temp.path().join(&record.metadata_path)).unwrap();
    let second = std::fs::read(temp.path().join(&record.metadata_path)).unwrap();
    assert_eq!(first, second);
    assert!(
        !String::from_utf8(first)
            .unwrap()
            .contains(temp.path().to_string_lossy().as_ref())
    );
}

#[test]
fn typed_asset_provenance_round_trips_for_each_governed_asset() {
    use archon_trading::data_lake::{
        AssetProvenance, CryptoProvenance, EtfProvenance, FuturesContinuityMethod,
        FuturesProvenance, FuturesRolloverRule, RolloverTrigger,
    };
    let provenance = AssetProvenance {
        futures: Some(FuturesProvenance {
            contract_chain: vec!["ESH26".into(), "ESM26".into()],
            continuity_method: FuturesContinuityMethod::BackAdjusted,
            rollover_rule: FuturesRolloverRule {
                trigger: RolloverTrigger::Volume,
                offset_days: 1,
            },
            adjustment_method: "difference".into(),
        }),
        etf: Some(EtfProvenance {
            corporate_action_source: "exchange notices".into(),
            split_adjusted: true,
            dividend_adjusted: true,
            as_of: "2026-01-01T00:00:00Z".into(),
        }),
        crypto: Some(CryptoProvenance {
            venue: "coinbase".into(),
            market_type: "spot".into(),
            instrument_id: "BTC-USD".into(),
        }),
    };
    let encoded = serde_json::to_vec(&provenance).unwrap();
    let decoded: AssetProvenance = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, provenance);
}

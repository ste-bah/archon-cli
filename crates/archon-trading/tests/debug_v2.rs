use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, GapSummary, NativeLineageEvidence,
};
use std::collections::BTreeMap;

fn make_metadata(version: &str, _created_at: &str) -> DatasetMetadata {
    DatasetMetadata {
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
            start: "2026-01-01T00:00:00Z".into(),
            end: "2026-01-02T00:00:00Z".into(),
            expected_bars: 2,
            observed_bars: 2,
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
    }
}

fn make_native_lineage(version: &str, retrieved_at: &str) -> NativeLineageEvidence {
    serde_json::from_value(serde_json::json!({
        "observation": {
            "dataset_id": "manual-BTCUSD-1D-raw",
            "version": version,
            "provider": "manual",
            "canonical_instrument": "BTCUSD",
            "provider_symbol": "BTCUSD",
            "timeframe": "1D",
            "retrieved_at": retrieved_at,
            "exact_native_interval": true,
            "complete": true
        },
        "lineage": {
            "aggregated": false,
            "resampled": false,
            "downsampled": false,
            "upsampled": false,
            "interpolated": false,
            "synthesized": false
        }
    }))
    .unwrap()
}

#[test]
fn debug_migration() {
    let created_at = "2026-01-01T00:00:00Z";
    let version = "20260101-fixture";

    let mut metadata = make_metadata(version, created_at);
    let evidence = make_native_lineage(version, created_at);

    // Simulate what store_ohlcv does
    if metadata.source.retrieved_at.trim().is_empty() {
        metadata.source.retrieved_at = created_at.to_string();
    }

    eprintln!("=== BEFORE CHECK ===");
    eprintln!("production_eligible: {:?}", metadata.production_eligible);
    eprintln!("source.retrieved_at: {:?}", metadata.source.retrieved_at);
    eprintln!(
        "observation.retrieved_at: {:?}",
        evidence.observation.retrieved_at
    );
    eprintln!(
        "EQUAL: {}",
        metadata.source.retrieved_at == evidence.observation.retrieved_at
    );
    eprintln!(
        "dataset_id match: {}",
        metadata.dataset_id == evidence.observation.dataset_id
    );
    eprintln!(
        "version match: {}",
        metadata.version == evidence.observation.version
    );
    eprintln!(
        "provider match: {}",
        metadata.provider == evidence.observation.provider
    );
    eprintln!(
        "instrument match: {}",
        metadata.canonical_instrument == evidence.observation.canonical_instrument
    );
    eprintln!(
        "symbol match: {}",
        metadata.provider_symbol == evidence.observation.provider_symbol
    );
    eprintln!(
        "timeframe match: {}",
        metadata.timeframe == evidence.observation.timeframe
    );
    eprintln!(
        "exact_native_interval: {:?}",
        evidence.observation.exact_native_interval
    );
    eprintln!("complete: {:?}", evidence.observation.complete);
    eprintln!(
        "lineage.is_exact_native: {:?}",
        evidence.lineage.is_exact_native()
    );
    eprintln!(
        "rfc3339 valid: {:?}",
        chrono::DateTime::parse_from_rfc3339(&evidence.observation.retrieved_at).is_ok()
    );

    // Now try the full match
    let obs_matches = evidence.observation.dataset_id == metadata.dataset_id
        && evidence.observation.version == metadata.version
        && evidence.observation.provider == metadata.provider
        && evidence.observation.canonical_instrument == metadata.canonical_instrument
        && evidence.observation.provider_symbol == metadata.provider_symbol
        && evidence.observation.timeframe == metadata.timeframe
        && evidence.observation.retrieved_at == metadata.source.retrieved_at
        && chrono::DateTime::parse_from_rfc3339(&evidence.observation.retrieved_at).is_ok()
        && evidence.observation.exact_native_interval
        && evidence.observation.complete;

    eprintln!("native_observation_matches: {}", obs_matches);
    eprintln!(
        "native_lineage_matches: {}",
        obs_matches && evidence.lineage.is_exact_native()
    );

    // Now test via serde round-trip
    let raw_request = serde_json::json!({
        "source": "test",
        "native_lineage_evidence": {
            "observation": {
                "dataset_id": "manual-BTCUSD-1D-raw",
                "version": version,
                "provider": "manual",
                "canonical_instrument": "BTCUSD",
                "provider_symbol": "BTCUSD",
                "timeframe": "1D",
                "retrieved_at": created_at,
                "exact_native_interval": true,
                "complete": true
            },
            "lineage": {
                "aggregated": false,
                "resampled": false,
                "downsampled": false,
                "upsampled": false,
                "interpolated": false,
                "synthesized": false
            }
        }
    });

    eprintln!("\n=== SERDE TEST ===");
    let from_json: Option<NativeLineageEvidence> = raw_request
        .get("native_lineage_evidence")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    eprintln!("serde result: {:?}", from_json.is_some());
    if let Some(ref deser) = from_json {
        eprintln!(
            "deser.observation.retrieved_at: {:?}",
            deser.observation.retrieved_at
        );
        eprintln!(
            "match result: {}",
            metadata.source.retrieved_at == deser.observation.retrieved_at
        );
    }
}

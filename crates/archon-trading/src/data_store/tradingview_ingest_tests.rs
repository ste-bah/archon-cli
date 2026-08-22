use super::*;
use crate::data_lake::{
    CurrentSnapshot, TV_CHART_GET_STATE, TV_DATA_GET_OHLCV, TV_HEALTH_CHECK,
    TradingViewHistoryRequest, TradingViewMcpActionCounts,
};
use serde_json::json;

fn history() -> TradingViewNativeHistory {
    TradingViewNativeHistory {
        request: TradingViewHistoryRequest {
            canonical_instrument: "ES".into(),
            provider_symbol: "CME_MINI:ES1!".into(),
            timeframe: "1D".into(),
            start: "2026-01-01T00:00:00Z".into(),
            end: "2026-01-02T00:00:00Z".into(),
            expected_bars: 2,
        },
        bars: vec![
            bar("2026-01-01T00:00:00Z", 10.0),
            bar("2026-01-02T00:00:00Z", 11.0),
        ],
        raw_response: json!({
            "symbol":"CME_MINI:ES1!", "timeframe":"1D",
            "start":"2026-01-01T00:00:00Z", "end":"2026-01-02T00:00:00Z",
            "coverage_complete":true,
            "bars":[
                {"timestamp":"2026-01-01T00:00:00Z","open":10.0,"high":11.0,"low":9.0,"close":10.0,"volume":1000.0},
                {"timestamp":"2026-01-02T00:00:00Z","open":11.0,"high":12.0,"low":10.0,"close":11.0,"volume":1100.0}
            ]
        }),
        returned_bars: 2,
        action_counts: TradingViewMcpActionCounts::exact_history_sequence(),
    }
}

fn publication() -> TradingViewPublication {
    TradingViewPublication {
        asset_class: "future".into(),
        session: "CME".into(),
        timezone: "America/Chicago".into(),
        adjustment: "provider_native_continuous".into(),
        license: "research".into(),
        created_at: "2026-01-03T00:00:00Z".into(),
    }
}

#[test]
fn tradingview_history_publication_is_registry_last() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake
        .publish_tradingview_history(history(), publication())
        .unwrap();

    let registry = lake.load_registry().unwrap();
    assert!(
        registry
            .datasets
            .contains_key(&registry_key(&record.dataset_id, &record.version))
    );
    for path in [
        &record.raw_response_path,
        &record.raw_request_path,
        &record.redacted_headers_path,
        &record.provider_notes_path,
        &record.normalized_path,
        &record.metadata_path,
        &record.validation_path,
        &record.manifest_path,
    ] {
        assert!(temp.path().join(path).is_file(), "missing {path}");
    }
    let request: serde_json::Value =
        read_json(&temp.path().join(&record.raw_request_path)).unwrap();
    assert_eq!(
        request["sequence"],
        json!([TV_HEALTH_CHECK, TV_CHART_GET_STATE, TV_DATA_GET_OHLCV])
    );
    assert_eq!(request["action_counts"][TV_HEALTH_CHECK], 1);
    assert_eq!(request["action_counts"][TV_CHART_GET_STATE], 1);
    assert_eq!(request["action_counts"][TV_DATA_GET_OHLCV], 1);
    assert_eq!(request["action_counts"][crate::data_lake::TV_QUOTE_GET], 0);
    assert_eq!(
        request["redacted_request_fingerprint"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    let loaded = lake
        .load_ohlcv(&record.dataset_id, &record.version)
        .unwrap();
    assert_eq!(loaded.bars, history().bars);
    assert_eq!(loaded.metadata.coverage.observed_bars, 2);
}

#[test]
fn tradingview_snapshot_is_separate_from_history() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let snapshot = CurrentSnapshot {
        provider: "tradingview".into(),
        canonical_instrument: "ES".into(),
        provider_symbol: "CME_MINI:ES1!".into(),
        captured_at_unix_seconds: 1_767_225_600,
        payload: json!({"symbol":"CME_MINI:ES1!", "last": 10.5}),
    };
    let path = lake
        .publish_tradingview_snapshot(snapshot, 1_767_225_600)
        .unwrap();

    assert!(path.ends_with("snapshots/tradingview/ES.json"));
    let registry = lake.load_registry().unwrap();
    assert!(registry.datasets.is_empty());
    assert_eq!(registry.snapshots.len(), 1);
    assert!(!lake.data_root().join("datasets").exists());
}

#[test]
fn tradingview_snapshot_rejects_foreign_identity() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let snapshot = CurrentSnapshot {
        provider: "tradingview".into(),
        canonical_instrument: "ES".into(),
        provider_symbol: "CME_MINI:NQ1!".into(),
        captured_at_unix_seconds: 1_767_225_600,
        payload: json!({"symbol":"CME_MINI:NQ1!", "last": 10.5}),
    };

    assert!(
        lake.publish_tradingview_snapshot(snapshot, 1_767_225_600)
            .is_err()
    );
    assert!(!lake.snapshot_path("tradingview", "ES").exists());
    assert!(lake.load_registry().unwrap().snapshots.is_empty());
}

#[test]
fn history_without_captured_mcp_sequence_is_not_published() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut unproven = history();
    unproven.action_counts = TradingViewMcpActionCounts::default();

    assert!(
        lake.publish_tradingview_history(unproven, publication())
            .is_err()
    );
    assert!(!lake.data_root().join("datasets").exists());
    assert!(lake.load_registry().unwrap().datasets.is_empty());
}

#[test]
fn incomplete_history_never_creates_registry_entry() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut incomplete = history();
    incomplete.returned_bars = 1;
    let registry_path = lake.registry_path();
    let prior_registry = std::fs::read(&registry_path).ok();
    assert!(
        lake.publish_tradingview_history(incomplete, publication())
            .is_err()
    );
    assert_eq!(std::fs::read(&registry_path).ok(), prior_registry);
    assert!(lake.load_registry().unwrap().datasets.is_empty());
}

#[test]
fn tradingview_publish_failure_removes_partial_dataset() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let history = history();
    let checksum = bytes_checksum(&serde_json::to_vec(&history.raw_response).unwrap());
    let version = raw_bound_version(&publication().created_at, &checksum).unwrap();
    let dataset_dir = lake.dataset_dir("tradingview-ES-1D-raw", &version);
    let registry_path = lake.registry_path();
    let prior_registry = std::fs::read(&registry_path).ok();

    inject_io_failure(Some("file.rename"));
    let result = lake.publish_tradingview_history(history, publication());
    inject_io_failure(None);

    assert!(result.is_err());
    assert!(!dataset_dir.exists());
    assert_eq!(std::fs::read(&registry_path).ok(), prior_registry);
    assert!(lake.load_registry().unwrap().datasets.is_empty());
}

#[test]
fn tradingview_secret_bearing_history_is_not_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut unsafe_history = history();
    unsafe_history.raw_response["nested"] = json!({"authorization":"seeded-sensitive"});

    assert!(
        lake.publish_tradingview_history(unsafe_history, publication())
            .is_err()
    );
    assert!(!lake.data_root().join("datasets").exists());
    assert!(lake.load_registry().unwrap().datasets.is_empty());
}

fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: close * 100.0,
    }
}

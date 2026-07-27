use crate::command::trading_data_provider::fetch_native;

use super::mock_openbb::raw_http_server;
use super::openbb::{EnvGuard, env_lock};

#[test]
fn fetch_native_stooq_direct_csv_writes_complete_artifacts() {
    let _lock = env_lock();
    let server = raw_http_server(
        "Date,Open,High,Low,Close,Volume\n2026-01-02,470,472,469,471,1000\n",
        "text/csv",
        &["GET /", "archon-cli trading-data stooq-native"],
    );
    let _guard = EnvGuard::set("ARCHON_STOOQ_CSV_URL", &server.base_url);
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "stooq",
        "SPY",
        "1D",
        "2026-01-02",
        "2026-01-02",
        "stooq-SPY-1D-raw",
    )
    .unwrap();
    server.join();

    assert!(text.contains("\"can_fetch\": true"));
    assert!(text.contains("\"production_eligible\": true"));
    let dataset_root = temp
        .path()
        .join(".archon/trading-lab/data/datasets/stooq-SPY-1D-raw/20260102-native_stooq_1D");
    assert!(dataset_root.join("manifest.json").exists());
    assert!(dataset_root.join("ohlcv.jsonl").exists());
    assert!(dataset_root.join("validation.json").exists());
    assert!(dataset_root.join("raw/request.json").exists());
    assert!(dataset_root.join("raw/headers.redacted.json").exists());
    assert!(dataset_root.join("raw/response.csv").exists());
    assert!(dataset_root.join("raw/provider-notes.md").exists());
}

#[test]
fn fetch_native_stooq_html_block_fails_closed_without_registry() {
    let _lock = env_lock();
    let server = raw_http_server(
        "<!doctype html><html><body>verification required</body></html>",
        "text/html",
        &["GET /", "archon-cli trading-data stooq-native"],
    );
    let _guard = EnvGuard::set("ARCHON_STOOQ_CSV_URL", &server.base_url);
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "stooq",
        "SPY",
        "1D",
        "2026-01-02",
        "2026-01-02",
        "stooq-SPY-1D-raw",
    )
    .unwrap();
    server.join();

    assert!(text.contains("provider_blocked_or_unavailable"));
    assert!(text.contains("non-data response"));
    assert!(text.contains("unavailable_manifest_path"));
    assert!(
        !temp
            .path()
            .join(".archon/trading-lab/data/registry.json")
            .exists()
    );
    let dataset_root = temp
        .path()
        .join(".archon/trading-lab/data/datasets/stooq-SPY-1D-raw");
    let manifest_path = std::fs::read_dir(&dataset_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["status"], "provider_blocked_or_unavailable");
    assert_eq!(manifest["production_eligible"], false);
    assert_eq!(manifest["registered_healthy_dataset"], false);
    assert!(!dataset_root.join("raw/response.csv").exists());
    assert!(!dataset_root.join("ohlcv.jsonl").exists());
}

#[test]
fn fetch_native_stooq_non_daily_refusal_writes_unavailable_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "stooq",
        "ES",
        "240",
        "2026-01-01",
        "2026-01-02",
        "stooq-ES-240-raw",
    )
    .unwrap();

    assert!(text.contains("provider_blocked_or_unavailable"));
    assert!(text.contains("resampling is refused"));
    assert!(text.contains("unavailable_manifest_path"));
    assert!(
        temp.path()
            .join(".archon/trading-lab/data/provider-capabilities.json")
            .exists()
    );
    assert!(
        !temp
            .path()
            .join(".archon/trading-lab/data/registry.json")
            .exists()
    );
    let dataset_root = temp
        .path()
        .join(".archon/trading-lab/data/datasets/stooq-ES-240-raw");
    let manifest_path = std::fs::read_dir(&dataset_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["status"], "provider_blocked_or_unavailable");
    assert_eq!(manifest["native_interval"], false);
    assert_eq!(manifest["production_eligible"], false);
    assert_eq!(manifest["registered_healthy_dataset"], false);
    assert!(!dataset_root.join("raw/response.csv").exists());
    assert!(!dataset_root.join("ohlcv.jsonl").exists());
}

use crate::command::trading_data_provider::fetch_native;
use crate::command::trading_data_provider_openbb::{
    fetch_native_with_base_url, probe_capability_with_base_url,
};
use archon_trading::data_store::TradingDataLake;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use super::mock_openbb::openbb_server;

pub(super) static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn fetch_native_reports_openbb_api_unavailable_fail_closed() {
    let _lock = env_lock();
    let _guard = EnvGuard::set("POLYGON_API_KEY", "redacted-test-key");
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        "http://127.0.0.1:9",
        "openbb",
        "SPY",
        "1D",
        "2026-01-01",
        "2026-01-02",
        "openbb-SPY-1D-raw",
    )
    .unwrap();
    assert!(text.contains("OpenBB API request failed"));
    assert!(text.contains("no dataset registry entry is written"));
}

#[test]
fn openbb_capability_probe_persists_true_with_small_limit() {
    let _lock = env_lock();
    let server = openbb_server(
        json!({
            "results": [
                {"date":"2024-01-02","open":472.16,"high":473.67,"low":470.49,"close":472.65,"volume":123623700}
            ]
        }),
        &[
            "/api/v1/equity/price/historical",
            "limit=2",
            "provider=polygon",
        ],
    );
    let _guard = EnvGuard::set("POLYGON_API_KEY", "redacted-test-key");
    let temp = tempfile::tempdir().unwrap();
    let result =
        probe_capability_with_base_url(temp.path(), &server.base_url, "openbb", "SPY", "1D")
            .unwrap();
    server.join();

    assert!(result.can_fetch);
    assert!(result.native_interval);
    assert!(result.unavailable_reason.is_none());
    let horizon = result
        .history_horizon
        .as_ref()
        .expect("verified history horizon");
    assert!(horizon.start < horizon.end);
    assert_eq!(horizon.basis, "successful_recent_capability_probe");
    let records: BTreeMap<_, _> = TradingDataLake::new(temp.path())
        .load_capabilities()
        .unwrap();
    assert!(records.values().any(|record| record.can_fetch));
}

#[test]
fn fetch_native_uses_recent_capability_horizon_instead_of_stale_window() {
    let _lock = env_lock();
    let server = openbb_server(
        json!({"results": [
                {"date":"2025-07-02","open":472.16,"high":473.67,"low":470.49,"close":472.65,"volume":123623700},
                {"date":"2025-07-03","open":470.43,"high":471.19,"low":468.17,"close":468.79,"volume":103585900}
            ], "provider": "polygon"}),
        &[
            "start_date=2025-07-01",
            "end_date=2026-07-01",
            "provider=polygon",
        ],
    );
    let _guard = EnvGuard::set("POLYGON_API_KEY", "redacted-test-key");
    let temp = tempfile::tempdir().unwrap();
    let mut capability = archon_trading::data_lake::can_fetch_symbol_timeframe(
        "openbb",
        "SPY",
        "1D",
        "2026-07-01T00:00:00Z",
    );
    capability.history_horizon = Some(archon_trading::data_lake::ProviderHistoryHorizon {
        start: "2025-07-01".into(),
        end: "2026-07-01".into(),
        basis: "fixture_entitlement".into(),
    });
    TradingDataLake::new(temp.path())
        .persist_capability_result(capability)
        .unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        &server.base_url,
        "openbb",
        "SPY",
        "1D",
        "2024-01-01",
        "2024-06-01",
        "openbb-SPY-1D-raw",
    )
    .unwrap();
    server.join();
    assert!(text.contains("\"window_status\": \"window-outside-entitlement\""));
    assert!(text.contains("\"start\": \"2025-07-01\""));
    assert!(text.contains("\"end\": \"2026-07-01\""));
    assert!(text.contains("20250701-native_polygon_1D"));
}

#[test]
fn outside_entitlement_failure_is_distinct_from_generic_no_content() {
    let _lock = env_lock();
    let _guard = EnvGuard::set("POLYGON_API_KEY", "redacted-test-key");
    let temp = tempfile::tempdir().unwrap();
    let mut capability = archon_trading::data_lake::can_fetch_symbol_timeframe(
        "openbb",
        "SPY",
        "1D",
        "2026-07-01T00:00:00Z",
    );
    capability.history_horizon = Some(archon_trading::data_lake::ProviderHistoryHorizon {
        start: "2025-07-01".into(),
        end: "2026-07-01".into(),
        basis: "fixture".into(),
    });
    TradingDataLake::new(temp.path())
        .persist_capability_result(capability)
        .unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        "http://127.0.0.1:9",
        "openbb",
        "SPY",
        "1D",
        "2024-01-01",
        "2024-06-01",
        "openbb-SPY-1D-raw",
    )
    .unwrap();
    assert!(text.contains("window-outside-entitlement: requested 2024-01-01..2024-06-01"));
    assert!(text.contains("OpenBB API request failed"));
}

#[test]
fn fetch_native_openbb_stores_registered_dataset() {
    let _lock = env_lock();
    let server = openbb_server(
        json!({
            "results": [
                {"date":"2024-01-02","open":472.16,"high":473.67,"low":470.49,"close":472.65,"volume":123623700},
                {"date":"2024-01-03","open":470.43,"high":471.19,"low":468.17,"close":468.79,"volume":103585900}
            ],
            "provider": "polygon"
        }),
        &[
            "/api/v1/equity/price/historical",
            "provider=polygon",
            "symbol=SPY",
            "interval=1d",
        ],
    );
    let _guard = EnvGuard::set("POLYGON_API_KEY", "redacted-test-key");
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        &server.base_url,
        "openbb",
        "SPY",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "openbb-SPY-1D-raw",
    )
    .unwrap();
    server.join();

    assert!(text.contains("\"can_fetch\": true"));
    assert!(text.contains("\"production_eligible\": false"));
    assert!(temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
    assert!(temp
        .path()
        .join(".archon/trading-lab/data/datasets/openbb-SPY-1D-raw")
        .exists());
    assert!(text.contains("raw/response.json"));
    assert!(text.contains("\"credential_state\""));
    assert!(text.contains("POLYGON_API_KEY"));

    let dataset_root = temp
        .path()
        .join(".archon/trading-lab/data/datasets/openbb-SPY-1D-raw/20240101-native_polygon_1D");
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dataset_root.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["coverage"]["expected_bars"], 5);
    assert_eq!(metadata["coverage"]["observed_bars"], 2);
    assert_eq!(metadata["gaps"]["missing_bars"], 3);
    assert_eq!(metadata["production_eligible"], false);
    assert_eq!(
        metadata["source"]["license_notes"],
        "Licensed native OHLCV via OpenBB provider polygon; credentials supplied by runtime profile and not stored"
    );
    assert_eq!(
        metadata["source"]["url_or_endpoint"],
        "/api/v1/equity/price/historical"
    );
    assert_eq!(metadata["source"]["credential_required"], true);
    assert!(dataset_root.join("manifest.json").exists());
}

#[test]
fn fetch_native_reports_stooq_interval_refusal_fail_closed() {
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
    assert!(text.contains("exact native Stooq data was not directly supplied"));
    assert!(text.contains("resampling is refused"));
    assert!(text.contains("\"production_eligible\": false"));
}

#[test]
fn fetch_native_yfinance_stores_degraded_fallback_dataset() {
    let server = openbb_server(
        json!({
            "results": [
                {"date":"2024-01-02","open":472.16,"high":473.67,"low":470.49,"close":472.65,"volume":123623700}
            ],
            "provider": "yfinance"
        }),
        &["/api/v1/equity/price/historical", "provider=yfinance"],
    );
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        &server.base_url,
        "yfinance",
        "SPY",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "yfinance-SPY-1D-raw",
    )
    .unwrap();
    server.join();
    let dataset_root = temp
        .path()
        .join(".archon/trading-lab/data/datasets/yfinance-SPY-1D-raw/20240101-native_yfinance_1D");
    let metadata_path = dataset_root.join("metadata.json");
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();

    assert!(text.contains("\"can_fetch\": true"));
    assert!(text.contains("\"quality_status\": \"degraded\""));
    assert!(text.contains("\"production_eligible\": false"));
    assert_eq!(metadata["provider"], "yfinance");
    assert_eq!(metadata["quality_status"], "degraded");
    assert_eq!(metadata["production_eligible"], false);
    assert_eq!(
        metadata["source"]["license_notes"],
        "ResearchOnly via OpenBB/yfinance degraded fallback"
    );
    assert!(dataset_root.join("validation.json").exists());
    assert!(dataset_root.join("manifest.json").exists());
    assert!(dataset_root.join("ohlcv.jsonl").exists());
    assert!(dataset_root.join("raw/request.json").exists());
    assert!(dataset_root.join("raw/response.json").exists());
    assert!(dataset_root.join("raw/headers.redacted.json").exists());
    assert!(dataset_root.join("raw/provider-notes.md").exists());

    let lake = TradingDataLake::new(temp.path());
    let refusal = lake
        .backtest_data_gate("yfinance-SPY-1D-raw", "20240101-native_yfinance_1D", false)
        .unwrap_err();
    let archon_trading::data_store::DataStoreError::InvalidMetadata(refusal) = refusal else {
        panic!("expected invalid metadata refusal");
    };
    assert!(refusal.contains("dataset is not production eligible"));
    assert!(refusal.contains("dataset registry status is degraded"));

    let diagnostic = lake
        .backtest_data_gate("yfinance-SPY-1D-raw", "20240101-native_yfinance_1D", true)
        .unwrap();
    assert!(diagnostic.diagnostic);
    assert!(!diagnostic.promotion_eligible);
    assert_eq!(diagnostic.overridden_issues, diagnostic.issues);
}

#[test]
fn fetch_native_yfinance_interval_limitation_is_degraded_non_promotion() {
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        "http://127.0.0.1:9",
        "yfinance",
        "SPY",
        "5",
        "2024-01-01",
        "2024-01-05",
        "yfinance-SPY-5-raw",
    )
    .unwrap();

    assert!(text.contains("unsupported native timeframe `5`"));
    assert!(text.contains("\"quality_status\": \"degraded_fallback\""));
    assert!(text.contains("\"production_eligible\": false"));
    assert!(text.contains("provider_blocked_or_unavailable"));
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn fetch_native_openbb_polygon_requires_credentials_fail_closed() {
    let _lock = env_lock();
    let _guard = EnvGuard::unset("POLYGON_API_KEY");
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native_with_base_url(
        temp.path(),
        "http://127.0.0.1:9",
        "openbb",
        "SPY",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "openbb-SPY-1D-raw",
    )
    .unwrap();
    assert!(text.contains("OpenBB credentials unavailable for polygon"));
    assert!(text.contains("POLYGON_API_KEY"));
    assert!(text.contains("provider_blocked_or_unavailable"));
}

#[test]
fn fetch_native_openbb_polygon_requires_openbb_api_url_fail_closed() {
    let _lock = env_lock();
    let _polygon_guard = EnvGuard::set("POLYGON_API_KEY", "redacted-test-key");
    let _url_guard = EnvGuard::unset("OPENBB_API_URL");
    let temp = tempfile::tempdir().unwrap();
    let text = fetch_native(
        Some(&temp.path().to_path_buf()),
        "polygon",
        "SPY",
        "1D",
        "2024-01-01",
        "2024-01-05",
        "polygon-SPY-1D-raw",
    )
    .unwrap();

    assert!(text.contains("OpenBB credentials unavailable for polygon"));
    assert!(text.contains("OPENBB_API_URL"));
    assert!(text.contains("provider_blocked_or_unavailable"));
    assert!(!temp
        .path()
        .join(".archon/trading-lab/data/registry.json")
        .exists());
}

#[test]
fn fetch_native_stooq_unavailable_report_includes_capability_state() {
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

    assert!(text.contains("\"checked_at\""));
    assert!(text.contains("\"credential_state\": \"not_required\""));
    assert!(text.contains("\"provider_blocked\": false"));
    assert!(text.contains("\"unsupported\": true"));
    assert!(text.contains("\"production_eligible\": false"));
    assert!(temp
        .path()
        .join(".archon/trading-lab/data/provider-capabilities.json")
        .exists());
}

pub(super) struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

pub(super) fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().expect("provider test env mutex poisoned")
}

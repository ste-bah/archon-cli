use super::trading_data_provider::*;

#[cfg(test)]
#[path = "trading_data_provider_test_server.rs"]
mod mock_openbb;

#[cfg(test)]
mod tests {
    use super::super::trading_data_provider_openbb::fetch_native_with_base_url;
    use super::mock_openbb::openbb_server;
    use super::*;
    use archon_trading::data_store::TradingDataLake;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let result = super::super::trading_data_provider_openbb::probe_capability_with_base_url(
            temp.path(),
            &server.base_url,
            "openbb",
            "SPY",
            "1D",
        )
        .unwrap();
        server.join();

        assert!(result.can_fetch);
        assert!(result.native_interval);
        assert!(result.unavailable_reason.is_none());
        let records: BTreeMap<_, _> = TradingDataLake::new(temp.path())
            .load_capabilities()
            .unwrap();
        assert!(records.values().any(|record| record.can_fetch));
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
        assert!(text.contains("\"production_eligible\": true"));
        assert!(
            temp.path()
                .join(".archon/trading-lab/data/registry.json")
                .exists()
        );
        assert!(
            temp.path()
                .join(".archon/trading-lab/data/datasets/openbb-SPY-1D-raw")
                .exists()
        );
        assert!(text.contains("raw/response.json"));
        assert!(text.contains("\"credential_state\""));
        assert!(text.contains("POLYGON_API_KEY"));
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
        let dataset_root = temp.path().join(
            ".archon/trading-lab/data/datasets/yfinance-SPY-1D-raw/20240101-native_yfinance_1D",
        );
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
        assert!(
            !temp
                .path()
                .join(".archon/trading-lab/data/registry.json")
                .exists()
        );
    }

    #[test]
    fn coverage_reports_required_universe_as_json_and_text() {
        let temp = tempfile::tempdir().unwrap();
        let json = coverage(
            Some(&temp.path().to_path_buf()),
            "trading-core-v1",
            true,
            None,
        )
        .unwrap();
        assert!(json.contains("archon-trading-coverage-v1"));
        assert!(json.contains("BTCUSDT"));
        assert!(json.contains("fallback_reason"));
        assert!(
            temp.path()
                .join(".archon/trading-lab/data/coverage/latest.json")
                .exists()
        );
        assert!(
            temp.path()
                .join(".archon/trading-lab/data/coverage/latest.md")
                .exists()
        );

        let text = coverage(
            Some(&temp.path().to_path_buf()),
            "trading-core-v1",
            false,
            None,
        )
        .unwrap();
        assert!(text.contains("Trading coverage matrix"));
        assert!(text.contains("latest_json:"));
        assert!(text.contains("latest_md:"));
        assert!(text.contains("gaps: 30"));
    }

    #[test]
    fn fetch_native_tradingview_unavailable_writes_no_registry_dataset() {
        let temp = tempfile::tempdir().unwrap();
        let text = fetch_native(
            Some(&temp.path().to_path_buf()),
            "tradingview",
            "CME_MINI:ES1!",
            "1D",
            "2024-01-01",
            "2024-01-03",
            "tradingview-ES-1D-raw",
        )
        .unwrap();
        assert!(text.contains("TradingView MCP CLI missing"));
        assert!(text.contains("no dataset registry entry is written"));
        assert!(
            !temp
                .path()
                .join(".archon/trading-lab/data/registry.json")
                .exists()
        );
    }

    #[test]
    fn fetch_native_tradingview_stores_complete_artifact_contract() {
        let temp = tempfile::tempdir().unwrap();
        let cli = temp
            .path()
            .join(".archon/tools/tradingview-mcp/src/cli/index.js");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(
            &cli,
            r#"console.log(JSON.stringify({bars:[
                {ts:"2024-01-02T00:00:00Z",open:100,high:105,low:99,close:104,volume:10},
                {ts:"2024-01-03T00:00:00Z",open:104,high:106,low:101,close:102,volume:20}
            ]}));"#,
        )
        .unwrap();

        let text = fetch_native(
            Some(&temp.path().to_path_buf()),
            "tradingview",
            "CME_MINI:ES1!",
            "1D",
            "2024-01-01",
            "2024-01-05",
            "tradingview-ES-1D-raw",
        )
        .unwrap();
        let dataset_root = temp.path().join(
            ".archon/trading-lab/data/datasets/tradingview-ES-1D-raw/20240102-tv_native_1D_2",
        );
        assert!(text.contains("\"can_fetch\": true"));
        assert!(text.contains("\"production_eligible\": true"));
        assert!(text.contains("20240102-tv_native_1D_2"));
        assert!(dataset_root.join("metadata.json").exists());
        assert!(dataset_root.join("validation.json").exists());
        assert!(dataset_root.join("manifest.json").exists());
        assert!(dataset_root.join("ohlcv.jsonl").exists());
        assert!(dataset_root.join("raw/request.json").exists());
        assert!(dataset_root.join("raw/response.json").exists());
        assert!(dataset_root.join("raw/headers.redacted.json").exists());
        assert!(dataset_root.join("raw/provider-notes.md").exists());
        assert!(
            temp.path()
                .join(".archon/trading-lab/data/registry.json")
                .exists()
        );
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
        assert!(
            temp.path()
                .join(".archon/trading-lab/data/provider-capabilities.json")
                .exists()
        );
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
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

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().expect("provider test env mutex poisoned")
    }
}

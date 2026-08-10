//! Provider capability and native-fetch behaviour.
//!
//! Everything here is about the question "can this provider actually give us
//! these bars, right now, at this exact interval?" — and about the answer being
//! *no* by default. The parent module's tests cover the metadata contract; these
//! cover the capability interface in front of it, including snapshot freshness
//! and the raw-payload parsing that a fetch would feed into.

use super::*;
use crate::ohlcv::OhlcvBar;

#[test]
fn capability_results_fail_closed_without_full_fetch() {
    let result = can_fetch_symbol_timeframe("stooq", "ES", "240", "2026-06-10T00:00:00Z");
    assert!(!result.can_fetch);
    assert!(!result.native_interval);
    assert!(result.unavailable_reason.is_some());
    assert!(!result.production_eligible);
}

#[test]
fn snapshots_are_stale_after_five_minutes() {
    assert!(snapshot_is_fresh(1_000, 1_300));
    assert!(!snapshot_is_fresh(1_000, 1_301));
    assert!(!snapshot_is_fresh(1_300, 1_000));
    assert_eq!(snapshot_freshness(None, 1_300), SnapshotFreshness::Missing);
    assert_eq!(
        snapshot_freshness(Some(1_000), 1_301),
        SnapshotFreshness::Stale
    );
}

#[test]
fn capability_maps_timeframes_and_unavailable_reasons_fail_closed() {
    let previous_key = std::env::var_os("POLYGON_API_KEY");
    unsafe { std::env::remove_var("POLYGON_API_KEY") };
    let four_hour = can_fetch_symbol_timeframe("stooq", "ES", "4H", "now");
    assert_eq!(four_hour.timeframe, "240");
    assert!(!four_hour.native_interval);
    assert!(!four_hour.can_fetch);
    assert!(
        four_hour
            .unavailable_reason
            .as_deref()
            .unwrap()
            .contains("provider_blocked_or_unavailable")
    );

    let blocked = can_fetch_symbol_timeframe("polygon", "403", "1D", "now");
    assert_eq!(
        blocked.unavailable_reason.as_deref(),
        Some("provider blocked access")
    );
    assert!(!blocked.can_fetch);

    let unauthorized = can_fetch_symbol_timeframe("polygon", "401", "1D", "now");
    assert_eq!(
        unauthorized.unavailable_reason.as_deref(),
        Some("missing or invalid provider credentials")
    );
    assert!(!unauthorized.can_fetch);

    let missing_credentials = can_fetch_symbol_timeframe("polygon", "SPY", "1D", "now");
    assert!(missing_credentials.requires_credentials);
    assert_eq!(
        missing_credentials.unavailable_reason.as_deref(),
        Some("missing provider credentials")
    );
    assert!(!missing_credentials.can_fetch);

    let not_found = can_fetch_symbol_timeframe("polygon", "404", "1D", "now");
    assert_eq!(
        not_found.unavailable_reason.as_deref(),
        Some("provider symbol or endpoint not found")
    );
    assert!(!not_found.can_fetch);

    let fallback = can_fetch_symbol_timeframe("yfinance", "SPY", "1D", "now");
    assert_eq!(
        fallback.unavailable_reason.as_deref(),
        Some("yfinance fallback is degraded and ineligible for promotion")
    );
    assert!(!fallback.native_interval);
    assert!(!fallback.can_fetch);
    restore_env("POLYGON_API_KEY", previous_key);
}

#[test]
fn yfinance_interval_limitation_mapping_is_degraded_and_fail_closed() {
    let daily = can_fetch_symbol_timeframe("yfinance", "SPY", "1D", "now");
    assert!(daily.historical_supported);
    assert!(!daily.native_interval);
    assert!(!daily.production_eligible);
    assert!(!daily.can_fetch);
    assert_eq!(
        daily.unavailable_reason.as_deref(),
        Some("yfinance fallback is degraded and ineligible for promotion")
    );

    let unsupported = can_fetch_symbol_timeframe("yfinance", "SPY", "1M", "now");
    assert!(!unsupported.historical_supported);
    assert!(!unsupported.native_interval);
    assert!(!unsupported.production_eligible);
    assert_eq!(
        unsupported.unavailable_reason.as_deref(),
        Some("exact native interval is unsupported")
    );
}

struct UnavailableProvider;

impl NativeOhlcvProvider for UnavailableProvider {
    fn can_fetch_symbol_timeframe(
        &self,
        symbol: &str,
        timeframe: &str,
        checked_at: &str,
    ) -> ProviderCapabilityResult {
        can_fetch_symbol_timeframe("stooq", symbol, timeframe, checked_at)
    }

    fn fetch_ohlcv_native(
        &self,
        _symbol: &str,
        _timeframe: &str,
        _start: &str,
        _end: &str,
    ) -> Result<Vec<OhlcvBar>, ProviderFetchError> {
        Err(ProviderFetchError {
            provider: "stooq".into(),
            action: "fetch_ohlcv_native",
            reason: "exact native fetch unavailable".into(),
        })
    }

    fn fetch_current_snapshot(&self, _symbol: &str) -> Result<CurrentSnapshot, ProviderFetchError> {
        Err(ProviderFetchError {
            provider: "stooq".into(),
            action: "fetch_current_snapshot",
            reason: "snapshot unavailable".into(),
        })
    }
}

#[test]
fn parses_stooq_csv_fixture_as_daily_native_bars() {
    let csv = b"Date,Open,High,Low,Close,Volume\n2026-01-02,470,472,469,471,1000\n";
    let normalized = String::from_utf8_lossy(csv)
        .replace("Date,", "timestamp,")
        .replace("Open,High,Low,Close,Volume", "open,high,low,close,volume")
        .replace("2026-01-02,", "2026-01-02T00:00:00Z,");
    let bars =
        crate::ohlcv::parse_ohlcv(normalized.as_bytes(), crate::ohlcv::OhlcvFormat::Csv).unwrap();
    assert_eq!(bars.len(), 1);
    assert_eq!(bars[0].timestamp, "2026-01-02T00:00:00Z");
    assert_eq!(bars[0].close, 471.0);
}

#[test]
fn stooq_html_block_fixture_fails_closed_without_bars() {
    let html = b"<!doctype html><html><body>verification required</body></html>";
    let result = crate::ohlcv::parse_ohlcv(html, crate::ohlcv::OhlcvFormat::Csv);
    assert!(result.is_err());
}

#[test]
fn stooq_non_daily_interval_refuses_resampling() {
    let result = can_fetch_symbol_timeframe("stooq", "SPY", "4H", "2026-06-10T00:00:00Z");
    assert_eq!(result.timeframe, "240");
    assert!(!result.native_interval);
    assert!(!result.can_fetch);
    assert!(!result.production_eligible);
    assert!(
        result
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .contains("resampling")
    );
}

#[test]
fn provider_trait_contract_fails_closed_without_download() {
    let provider = UnavailableProvider;
    assert!(
        provider
            .fetch_ohlcv_native("ES", "240", "2026-01-01", "2026-01-02")
            .is_err()
    );
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

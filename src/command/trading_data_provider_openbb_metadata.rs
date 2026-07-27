use anyhow::Result;
use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetMetadata, GapSummary, ProviderCapabilityResult,
    ProviderHistoryHorizon,
};
use archon_trading::ohlcv::OhlcvBar;
use chrono::NaiveDate;
use serde_json::json;
use std::collections::BTreeMap;

use crate::command::trading_io::write_or_render;

use super::request::{FetchWindowSelection, OpenBbNativeRequest, is_crypto, is_future};

pub(super) fn native_metadata_from_bars(
    dataset_id: &str,
    provider: &str,
    symbol: &str,
    timeframe: &str,
    request: &OpenBbNativeRequest,
    bars: &[OhlcvBar],
) -> DatasetMetadata {
    let provider_key = provider.trim().to_ascii_lowercase();
    let observed_bars = bars.len() as u64;
    let expected_bars = requested_span_expected_bars(timeframe, request);
    let missing_bars = expected_bars.saturating_sub(observed_bars);
    let production_eligible =
        provider_key != "yfinance" && expected_bars > 0 && observed_bars == expected_bars;
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id: dataset_id.into(),
        version: native_version(timeframe, request),
        canonical_instrument: dataset_instrument(dataset_id)
            .unwrap_or_else(|| symbol.trim().into()),
        asset_class: request.asset_class.clone(),
        provider: provider_key.clone(),
        provider_symbol: request.provider_symbol.clone(),
        timeframe: timeframe.trim().into(),
        native_interval: true,
        production_eligible,
        price_basis: dataset_price_basis(dataset_id).unwrap_or_else(|| "raw".into()),
        session: session_for(symbol).into(),
        data_type: DataType::Ohlcv,
        symbol_map: BTreeMap::from([(symbol.trim().into(), request.provider_symbol.clone())]),
        timezone: timezone_for(symbol).into(),
        adjustment: adjustment_for(symbol).into(),
        license: license_for(&provider_key, &request.openbb_provider),
        coverage: CoverageWindow {
            start: bars
                .first()
                .map(|bar| bar.timestamp.clone())
                .unwrap_or_else(|| {
                    request
                        .params
                        .get("start_date")
                        .cloned()
                        .unwrap_or_default()
                }),
            end: bars
                .last()
                .map(|bar| bar.timestamp.clone())
                .unwrap_or_else(|| request.params.get("end_date").cloned().unwrap_or_default()),
            expected_bars,
            observed_bars,
        },
        gaps: GapSummary {
            missing_bars,
            expected_bars,
        },
        checksum: String::new(),
        checksums: Default::default(),
        paths: Default::default(),
        source: Default::default(),
        quality_status: native_quality_status(&provider_key).into(),
        created_at: String::new(),
        optional: false,
    }
}

pub(super) fn native_production_eligible(
    provider: &str,
    timeframe: &str,
    request: &OpenBbNativeRequest,
    observed_bars: usize,
) -> bool {
    if provider.trim().eq_ignore_ascii_case("yfinance") || observed_bars == 0 {
        return false;
    }
    let expected_bars = requested_span_expected_bars(timeframe, request);
    expected_bars > 0 && observed_bars as u64 == expected_bars
}

fn requested_span_expected_bars(timeframe: &str, request: &OpenBbNativeRequest) -> u64 {
    let start = request
        .params
        .get("start_date")
        .map(String::as_str)
        .unwrap_or_default();
    let end = request
        .params
        .get("end_date")
        .map(String::as_str)
        .unwrap_or_default();
    expected_bars_for_span(start, end, timeframe, &request.native_interval).unwrap_or(0)
}

fn expected_bars_for_span(
    start: &str,
    end: &str,
    timeframe: &str,
    native_interval: &str,
) -> Option<u64> {
    let start = NaiveDate::parse_from_str(start, "%Y-%m-%d").ok()?;
    let end = NaiveDate::parse_from_str(end, "%Y-%m-%d").ok()?;
    if end < start {
        return Some(0);
    }
    let inclusive_days = (end - start).num_days() as u64 + 1;
    let key = timeframe.trim().to_ascii_lowercase();
    match key.as_str() {
        "1d" => Some(inclusive_days),
        "1w" => Some(inclusive_days.div_ceil(7)),
        "240" | "4h" => Some(inclusive_days * 6),
        "60" | "1h" => Some(inclusive_days * 24),
        "15" | "15m" => Some(inclusive_days * 96),
        _ => match native_interval.trim().to_ascii_lowercase().as_str() {
            "1d" => Some(inclusive_days),
            "1w" => Some(inclusive_days.div_ceil(7)),
            "4h" => Some(inclusive_days * 6),
            "1h" => Some(inclusive_days * 24),
            "15m" => Some(inclusive_days * 96),
            _ => None,
        },
    }
}

pub(super) fn unavailable_report(
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    reason: &str,
    window: &FetchWindowSelection,
) -> Result<String> {
    let missing_credentials = is_missing_credentials_reason(reason);
    let unavailable_reason = if window.status == "window-outside-entitlement" {
        format!(
            "window-outside-entitlement: requested {}..{}; effective {}..{}; provider response: {reason}",
            window.requested_start,
            window.requested_end,
            window.effective_start,
            window.effective_end,
        )
    } else {
        reason.to_string()
    };
    let report = json!({
        "provider": provider.trim().to_ascii_lowercase(),
        "symbol": symbol,
        "timeframe": timeframe,
        "start": start,
        "end": end,
        "requested_window": {
            "start": window.requested_start,
            "end": window.requested_end,
        },
        "effective_window": {
            "start": window.effective_start,
            "end": window.effective_end,
        },
        "window_status": window.status,
        "history_horizon": window.history_horizon,
        "dataset_id": dataset_id,
        "can_fetch": false,
        "native_interval": false,
        "unavailable_reason": unavailable_reason,
        "quality_status": provider_quality_status(provider),
        "production_eligible": false,
        "provider_blocked_or_unavailable": true,
        "requires_credentials": requires_openbb_credentials(provider),
        "missing_credentials": missing_credentials,
        "credential_state": if missing_credentials { "missing" } else { "unavailable" },
        "secret_policy": "credential values are never stored; only env key presence is recorded",
        "fail_closed_behavior": "no dataset registry entry is written unless provider-native fetch returns complete artifacts"
    });
    write_or_render(&report, None)
}

pub(super) fn probed_history_horizon(now: chrono::DateTime<chrono::Utc>) -> ProviderHistoryHorizon {
    let end = now.date_naive();
    let start = end - chrono::Duration::days(365);
    ProviderHistoryHorizon {
        start: start.format("%Y-%m-%d").to_string(),
        end: end.format("%Y-%m-%d").to_string(),
        basis: "successful_recent_capability_probe".into(),
    }
}

pub(super) fn native_quality_status(provider: &str) -> &'static str {
    if provider.trim().eq_ignore_ascii_case("yfinance") {
        "degraded"
    } else {
        "passed"
    }
}

fn provider_quality_status(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "stooq" => "baseline_unavailable",
        "yfinance" => "degraded_fallback",
        _ => "unavailable",
    }
}

pub(super) fn apply_unavailable_capability_reason(
    result: &mut ProviderCapabilityResult,
    reason: &str,
) {
    let missing_credentials = is_missing_credentials_reason(reason);
    result.can_fetch = false;
    result.production_eligible = false;
    result.unavailable_reason = Some(reason.to_string());

    if missing_credentials {
        result.requires_credentials = true;
        result.missing_credentials = true;
        result.credential_state = "missing".into();
        result.provider_blocked = false;
        result.historical_supported = result.native_interval;
        return;
    }

    result.missing_credentials = false;
    result.credential_state = "unavailable".into();
    if is_native_unsupported_reason(reason) {
        result.unsupported = true;
    }
    result.native_interval = false;
    result.historical_supported = false;
}

fn is_missing_credentials_reason(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("credentials unavailable")
        || lower.contains("missing provider credentials")
        || lower.contains("env keys checked")
}

fn is_native_unsupported_reason(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("futures native fetch is unavailable")
        || lower.contains("unsupported native timeframe")
        || lower.contains("not backed by the openbb native fetch path")
}

fn requires_openbb_credentials(provider: &str) -> bool {
    matches!(
        provider.trim().to_ascii_lowercase().as_str(),
        "openbb" | "polygon"
    )
}

fn native_version(timeframe: &str, request: &OpenBbNativeRequest) -> String {
    let start = request
        .params
        .get("start_date")
        .cloned()
        .unwrap_or_default();
    format!(
        "{}-native_{}_{}",
        version_date_part(&start),
        request.openbb_provider,
        safe_version_part(timeframe)
    )
}

fn version_date_part(value: &str) -> String {
    let digits: String = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(8)
        .collect();
    if digits.len() == 8 {
        digits
    } else {
        chrono::Utc::now().format("%Y%m%d").to_string()
    }
}

fn dataset_instrument(dataset_id: &str) -> Option<String> {
    let (_provider, rest) = dataset_id.trim().split_once('-')?;
    let (prefix, _price_basis) = rest.rsplit_once('-')?;
    let (instrument, _timeframe) = prefix.rsplit_once('-')?;
    (!instrument.is_empty()).then(|| instrument.to_string())
}

fn dataset_price_basis(dataset_id: &str) -> Option<String> {
    let (_prefix, price_basis) = dataset_id.trim().rsplit_once('-')?;
    (!price_basis.is_empty()).then(|| price_basis.to_string())
}

fn safe_version_part(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn session_for(symbol: &str) -> &'static str {
    if is_crypto(symbol) {
        "24x7"
    } else if is_future(symbol) {
        "provider_continuous_futures"
    } else {
        "regular_trading_hours"
    }
}

fn timezone_for(symbol: &str) -> &'static str {
    if is_crypto(symbol) {
        "UTC"
    } else {
        "America/New_York"
    }
}

fn adjustment_for(symbol: &str) -> &'static str {
    if is_crypto(symbol) || is_future(symbol) {
        "unadjusted"
    } else {
        "splits_only"
    }
}

fn license_for(provider: &str, openbb_provider: &str) -> String {
    if provider == "yfinance" {
        "ResearchOnly via OpenBB/yfinance degraded fallback".into()
    } else {
        format!(
            "Licensed native OHLCV via OpenBB provider {openbb_provider}; credentials supplied by runtime profile and not stored"
        )
    }
}

pub(super) fn provider_notes(request: &OpenBbNativeRequest) -> String {
    if request.openbb_provider.eq_ignore_ascii_case("yfinance") {
        return format!(
            "Live Yahoo Finance chart API fetch used as yfinance degraded fallback. No credentials were required or stored. This dataset is diagnostic-only, production_eligible=false, promotion_eligible=false, and must not be promoted. Requested data was captured from the provider raw response; interval used: {}.",
            request.native_interval
        );
    }
    format!(
        "OpenBB native OHLCV via {} at {}. native_interval={}; adjustment={}; session={}; credential_state={:?}; no values stored.",
        request.openbb_provider,
        request.endpoint,
        request.native_interval,
        adjustment_for(&request.provider_symbol),
        session_for(&request.provider_symbol),
        request.credential_state
    )
}

pub(super) fn credential_state(openbb_provider: &str) -> BTreeMap<String, bool> {
    credential_env_keys_for(openbb_provider)
        .into_iter()
        .map(|key| (key.into(), std::env::var_os(key).is_some()))
        .collect()
}

pub(super) fn credential_env_keys_for(openbb_provider: &str) -> Vec<&'static str> {
    match openbb_provider.trim().to_ascii_lowercase().as_str() {
        "openbb" | "polygon" => vec!["OPENBB_API_URL", "POLYGON_API_KEY"],
        _ => vec!["OPENBB_API_KEY"],
    }
}

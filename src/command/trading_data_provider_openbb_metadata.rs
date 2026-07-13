use anyhow::Result;
use archon_trading::data_lake::{CoverageWindow, DataType, DatasetMetadata, GapSummary};
use archon_trading::ohlcv::OhlcvBar;
use serde_json::json;
use std::collections::BTreeMap;

use crate::command::trading_io::write_or_render;

use super::request::{OpenBbNativeRequest, is_crypto, is_future};

pub(super) fn native_metadata_from_bars(
    dataset_id: &str,
    provider: &str,
    symbol: &str,
    timeframe: &str,
    request: &OpenBbNativeRequest,
    bars: &[OhlcvBar],
) -> DatasetMetadata {
    let provider_key = provider.trim().to_ascii_lowercase();
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
        production_eligible: provider_key != "yfinance",
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
                .unwrap_or_else(|| request.params.get("start_date").cloned().unwrap_or_default()),
            end: bars
                .last()
                .map(|bar| bar.timestamp.clone())
                .unwrap_or_else(|| request.params.get("end_date").cloned().unwrap_or_default()),
            expected_bars: bars.len() as u64,
            observed_bars: bars.len() as u64,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: bars.len() as u64,
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

pub(super) fn unavailable_report(
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    reason: &str,
) -> Result<String> {
    let report = json!({
        "provider": provider.trim().to_ascii_lowercase(),
        "symbol": symbol,
        "timeframe": timeframe,
        "start": start,
        "end": end,
        "dataset_id": dataset_id,
        "can_fetch": false,
        "native_interval": false,
        "unavailable_reason": reason,
        "quality_status": provider_quality_status(provider),
        "production_eligible": false,
        "provider_blocked_or_unavailable": true,
        "fail_closed_behavior": "no dataset registry entry is written unless provider-native fetch returns complete artifacts"
    });
    write_or_render(&report, None)
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
        "openbb" | "polygon" => vec!["POLYGON_API_KEY"],
        _ => vec!["OPENBB_API_KEY"],
    }
}

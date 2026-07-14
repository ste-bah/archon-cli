use anyhow::Result;
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::OhlcvFormat;
use chrono::Utc;
use serde_json::json;
use std::path::Path;

use crate::command::trading_data::data_error;
use crate::command::trading_io::write_or_render;

#[path = "trading_data_provider_openbb_http.rs"]
mod http;
#[path = "trading_data_provider_openbb_metadata.rs"]
mod metadata;
#[path = "trading_data_provider_openbb_parse.rs"]
mod parse;
#[path = "trading_data_provider_openbb_request.rs"]
mod request;

use http::fetch_openbb_response;
use metadata::{
    apply_unavailable_capability_reason, native_metadata_from_bars, native_quality_status,
    provider_notes, unavailable_report,
};
use parse::bars_from_openbb_response;
use request::openbb_native_request;

pub(crate) fn fetch_native_with_base_url(
    root: &Path,
    base_url: &str,
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
) -> Result<String> {
    let request = match openbb_native_request(provider, symbol, timeframe, start, end, 49_999) {
        Ok(request) => request,
        Err(reason) => {
            return unavailable_report(
                provider, symbol, timeframe, start, end, dataset_id, &reason,
            );
        }
    };
    let response = match fetch_openbb_response(base_url, &request) {
        Ok(response) => response,
        Err(reason) => {
            return unavailable_report(
                provider, symbol, timeframe, start, end, dataset_id, &reason,
            );
        }
    };
    let bars = match bars_from_openbb_response(&response.body) {
        Ok(bars) => bars,
        Err(reason) => {
            return unavailable_report(
                provider,
                symbol,
                timeframe,
                start,
                end,
                dataset_id,
                &reason.to_string(),
            );
        }
    };
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let record = TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata: native_metadata_from_bars(
                dataset_id, provider, symbol, timeframe, &request, &bars,
            ),
            bars,
            raw_body: response.body,
            raw_format: OhlcvFormat::Json,
            raw_request: json!({
                "provider": provider.trim().to_ascii_lowercase(),
                "openbb_provider": request.openbb_provider,
                "endpoint": request.endpoint,
                "params": request.params,
            }),
            redacted_headers: response.redacted_headers,
            provider_notes: provider_notes(&request),
            created_at: fetched_at,
        })
        .map_err(data_error)?;
    let report = json!({
        "provider": provider.trim().to_ascii_lowercase(),
        "symbol": symbol,
        "timeframe": timeframe,
        "start": start,
        "end": end,
        "dataset_id": dataset_id,
        "can_fetch": true,
        "native_interval": true,
        "exact_interval": request.native_interval,
        "provider_symbol": request.provider_symbol,
        "credential_state": request.credential_state,
        "quality_status": native_quality_status(provider),
        "production_eligible": !provider.trim().eq_ignore_ascii_case("yfinance"),
        "stored_ohlcv": record,
        "fail_closed_behavior": "dataset was registered only after provider response parsed, validated, and artifact writes completed"
    });
    write_or_render(&report, None)
}

pub(crate) fn probe_capability_with_base_url(
    root: &Path,
    base_url: &str,
    provider: &str,
    symbol: &str,
    timeframe: &str,
) -> Result<archon_trading::data_lake::ProviderCapabilityResult> {
    let checked_at = Utc::now().to_rfc3339();
    let mut result = archon_trading::data_lake::can_fetch_symbol_timeframe(
        provider,
        symbol,
        timeframe,
        &checked_at,
    );
    match probe_openbb(base_url, provider, symbol, timeframe) {
        Ok(()) => {
            result.can_fetch = true;
            result.native_interval = true;
            result.production_eligible = !provider.trim().eq_ignore_ascii_case("yfinance");
            result.historical_supported = true;
            result.missing_credentials = false;
            result.credential_state = "available".into();
            result.unavailable_reason = None;
        }
        Err(reason) => {
            apply_unavailable_capability_reason(&mut result, &reason);
        }
    }
    TradingDataLake::new(root)
        .persist_capability_result(result)
        .map_err(data_error)
}

fn probe_openbb(
    base_url: &str,
    provider: &str,
    symbol: &str,
    timeframe: &str,
) -> Result<(), String> {
    let request =
        openbb_native_request(provider, symbol, timeframe, "2026-01-01", "2026-01-02", 2)?;
    let response = fetch_openbb_response(base_url, &request)?;
    bars_from_openbb_response(&response.body)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

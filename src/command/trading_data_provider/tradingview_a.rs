#[path = "tradingview/native_mcp.rs"]
mod native_mcp;
#[path = "tradingview/paging.rs"]
mod paging;
#[path = "tradingview/span.rs"]
mod span;

use anyhow::{Result, anyhow};
use archon_trading::data_lake::{CoverageWindow, DataType, DatasetMetadata, GapSummary};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat, parse_ohlcv, validate_bars};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

use crate::command::trading_data::data_error;
use crate::command::trading_io::write_or_render;
use span::TradingViewRequestSpan;

fn asset_class(symbol: &str) -> &'static str {
    match symbol.trim().to_ascii_uppercase().as_str() {
        "ES" | "NQ" => "future",
        "BTCUSDT" | "ETHUSDT" => "crypto",
        _ => "equity",
    }
}

fn session_for(symbol: &str) -> &'static str {
    if matches!(asset_class(symbol), "crypto") {
        "24x7"
    } else {
        "provider_default"
    }
}

fn timezone_for(symbol: &str) -> &'static str {
    if matches!(asset_class(symbol), "crypto") {
        "UTC"
    } else {
        "America/New_York"
    }
}

pub(super) fn fetch_tradingview_native(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
) -> Result<String> {
    let request_span = span::requested_span(start, end, timeframe)?;
    // A span wider than one call is assembled from several rather than
    // refused. The scroll semantics this relies on are not yet verified
    // against a live chart, so paging::assert_contiguous rejects the join if a
    // page lands anywhere other than where the previous one ended — a wrong
    // assumption fails loudly here instead of registering a series with holes.
    if request_span.requested_bars > request_span.per_call_limit {
        return match paged_tradingview_fetch(
            root,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            request_span,
        ) {
            Ok(outcome) => outcome,
            Err(reason) => {
                tradingview_unavailable(symbol, timeframe, start, end, dataset_id, &reason)
            }
        };
    }
    let response = match native_mcp::response(root, symbol, timeframe, request_span) {
        Ok(response) => response,
        Err(reason) => {
            return tradingview_unavailable(symbol, timeframe, start, end, dataset_id, &reason);
        }
    };
    if let Err(reason) = validate_tradingview_response_identity(&response, symbol, timeframe) {
        return tradingview_unavailable(
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            &reason.to_string(),
        );
    }
    let bars = match bars_from_tradingview_response(&response) {
        Ok(bars) => bars,
        Err(reason) => {
            return tradingview_unavailable(
                symbol,
                timeframe,
                start,
                end,
                dataset_id,
                &reason.to_string(),
            );
        }
    };
    if bars.len() < request_span.requested_bars {
        return tradingview_unavailable(
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            &format!(
                "TradingView row shortfall: requested={} actual={}",
                request_span.requested_bars,
                bars.len()
            ),
        );
    }
    let captured_bars = bars.len();
    // Captured before `bars` moves into the store request below.
    let volume_degraded = tradingview_volume_degraded(&bars);
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let record = TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata: tradingview_metadata(dataset_id, symbol, timeframe, &bars),
            bars,
            raw_body: response,
            raw_format: OhlcvFormat::Json,
            raw_request: tradingview_request(symbol, timeframe, start, end, request_span),
            redacted_headers: json!({ "mcp_state": "available", "mcp_status": "success",
                "native_timeframe": timeframe.trim(), "captured_bars": captured_bars,
                "requested_bars": request_span.requested_bars,
                "provider_call_bar_limit": request_span.per_call_limit }),
            provider_notes: tradingview_provider_notes(
                symbol,
                timeframe,
                captured_bars,
                request_span,
            ),
            created_at: fetched_at,
        })
        .map_err(data_error)?;
    write_or_render(
        &tradingview_success(
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            request_span,
            &record,
            volume_degraded,
        ),
        None,
    )
}

/// Assemble a span wider than one provider call.
///
/// Returns Ok(Ok(report)) shape via the caller: a short series is NOT an
/// error. When the provider runs out of history the dataset is still
/// legitimate, and the residual gap records the span actually served so
/// production eligibility is decided against that rather than the span
/// requested.
fn paged_tradingview_fetch(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    request_span: span::TradingViewRequestSpan,
) -> Result<Result<String>, String> {
    let max_pages = request_span
        .requested_bars
        .div_ceil(request_span.per_call_limit.max(1))
        + 2;
    let mut raw_pages = Vec::new();
    let series = paging::fetch_paged(
        request_span.requested_bars,
        request_span.per_call_limit,
        max_pages,
        |request| {
            if let Some(scroll_to) = request.scroll_to.as_deref() {
                native_mcp::scroll(root, scroll_to)?;
            }
            let body = native_mcp::run_ohlcv(root, symbol, timeframe, request_span)?;
            raw_pages.push(body.clone());
            bars_from_tradingview_response(&body).map_err(|err| err.to_string())
        },
    )?;
    let interval_secs = span::timeframe_seconds(timeframe).map_err(|err| err.to_string())?;
    paging::assert_contiguous(&series.bars, interval_secs, &[])?;
    if !matches!(series.boundary, paging::SeriesBoundary::Complete)
        || series.bars.len() < request_span.requested_bars
    {
        return Ok(tradingview_unavailable(
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            &format!(
                "TradingView paged row shortfall: requested={} actual={} boundary={:?}",
                request_span.requested_bars,
                series.bars.len(),
                series.boundary
            ),
        ));
    }
    let raw_body = native_mcp::paged_raw_body(symbol, timeframe, &series.bars, raw_pages)?;
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let captured_bars = series.bars.len();
    let record = TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata: tradingview_metadata(dataset_id, symbol, timeframe, &series.bars),
            bars: series.bars,
            raw_body,
            raw_format: OhlcvFormat::Json,
            raw_request: tradingview_request(symbol, timeframe, start, end, request_span),
            redacted_headers: json!({ "mcp_state": "available", "mcp_status": "success",
                "native_timeframe": timeframe.trim(), "captured_bars": captured_bars,
                "requested_bars": request_span.requested_bars,
                "provider_call_bar_limit": request_span.per_call_limit,
                "pages_fetched": series.pages_fetched }),
            provider_notes: tradingview_provider_notes(
                symbol,
                timeframe,
                captured_bars,
                request_span,
            ),
            created_at: fetched_at,
        })
        .map_err(|err| format!("{err:?}"))?;
    Ok(Ok(format!(
        "paged TradingView fetch stored {} bars for {symbol} {timeframe} across {} page(s); version={}; requested {start}..{end}",
        record.bars, series.pages_fetched, record.version
    )))
}

fn bars_from_tradingview_response(body: &[u8]) -> Result<Vec<OhlcvBar>> {
    if let Ok(bars) = parse_ohlcv(body, OhlcvFormat::Json) {
        return Ok(bars);
    }
    let value: Value = serde_json::from_slice(body)?;
    let rows = value
        .get("bars")
        .or_else(|| value.get("candles"))
        .or_else(|| value.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("TradingView response missing bars/candles/data array"))?;
    let bars = rows
        .iter()
        .map(tradingview_bar)
        .collect::<Result<Vec<_>>>()?;
    validate_bars(&bars).map_err(|err| anyhow!("invalid TradingView OHLCV data: {err:?}"))?;
    Ok(bars)
}

fn validate_tradingview_response_identity(
    body: &[u8],
    requested_symbol: &str,
    requested_timeframe: &str,
) -> Result<()> {
    let value: Value = serde_json::from_slice(body)?;
    let actual_symbol = value
        .get("symbol")
        .or_else(|| value.get("actual_symbol"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("TradingView response missing authoritative symbol"))?;
    let actual_timeframe = value
        .get("timeframe")
        .or_else(|| value.get("actual_timeframe"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("TradingView response missing authoritative timeframe"))?;
    if !actual_symbol.eq_ignore_ascii_case(requested_symbol.trim()) {
        return Err(anyhow!(
            "TradingView response symbol mismatch: requested={} actual={actual_symbol}",
            requested_symbol.trim()
        ));
    }
    if normalized_tradingview_timeframe(actual_timeframe)
        != normalized_tradingview_timeframe(requested_timeframe)
    {
        return Err(anyhow!(
            "TradingView response timeframe mismatch: requested={} actual={actual_timeframe}",
            requested_timeframe.trim()
        ));
    }
    Ok(())
}

fn normalized_tradingview_timeframe(value: &str) -> String {
    match value.trim().to_ascii_uppercase().as_str() {
        "D" => "1D".into(),
        "W" => "1W".into(),
        "M" => "1M".into(),
        value => value.to_string(),
    }
}

/// Zero or missing volume makes a dataset diagnostic-only, never production
/// eligible. `tradingview_bar` maps an absent `volume` field to 0.0, so both
/// the zero and missing cases are caught by the same predicate.
fn tradingview_volume_degraded(bars: &[OhlcvBar]) -> bool {
    bars.iter().any(|bar| bar.volume <= 0.0)
}

fn tradingview_bar(row: &Value) -> Result<OhlcvBar> {
    Ok(OhlcvBar {
        timestamp: tv_timestamp(row)?,
        open: tv_number(row, "open")?,
        high: tv_number(row, "high")?,
        low: tv_number(row, "low")?,
        close: tv_number(row, "close")?,
        volume: tv_number(row, "volume").unwrap_or(0.0),
    })
}

fn tv_timestamp(row: &Value) -> Result<String> {
    let raw = row
        .get("timestamp")
        .or_else(|| row.get("time"))
        .or_else(|| row.get("ts"))
        .ok_or_else(|| anyhow!("TradingView candle missing timestamp"))?;
    if let Some(seconds) = raw.as_i64() {
        return chrono::DateTime::from_timestamp(seconds, 0)
            .map(|ts| ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .ok_or_else(|| anyhow!("invalid TradingView unix timestamp"));
    }
    let text = raw
        .as_str()
        .ok_or_else(|| anyhow!("TradingView timestamp was not text"))?
        .trim();
    if text.ends_with('Z') || text.contains('+') {
        Ok(text.into())
    } else {
        Ok(format!("{text}Z"))
    }
}

fn tv_number(row: &Value, field: &str) -> Result<f64> {
    row.get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("TradingView candle missing numeric `{field}`"))
}

fn tradingview_metadata(
    dataset_id: &str,
    symbol: &str,
    timeframe: &str,
    bars: &[OhlcvBar],
) -> DatasetMetadata {
    let rows = bars.len() as u64;
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id: dataset_id.into(),
        version: tradingview_dataset_version(timeframe, bars),
        canonical_instrument: dataset_instrument(dataset_id)
            .unwrap_or_else(|| symbol.trim().into()),
        asset_class: asset_class(symbol).into(),
        provider: "tradingview".into(),
        provider_symbol: symbol.trim().into(),
        timeframe: timeframe.trim().into(),
        native_interval: true,
        production_eligible: !tradingview_volume_degraded(bars),
        price_basis: "raw".into(),
        session: session_for(symbol).into(),
        data_type: DataType::Ohlcv,
        symbol_map: BTreeMap::from([(symbol.trim().into(), symbol.trim().into())]),
        timezone: timezone_for(symbol).into(),
        adjustment: "raw".into(),
        license: "TradingView MCP chart-equivalent research data; provider terms apply".into(),
        coverage: CoverageWindow {
            start: String::new(),
            end: String::new(),
            expected_bars: rows,
            observed_bars: rows,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: rows,
        },
        checksum: String::new(),
        checksums: Default::default(),
        paths: Default::default(),
        source: Default::default(),
        quality_status: "passed".into(),
        created_at: String::new(),
        optional: false,
    }
}

fn dataset_instrument(dataset_id: &str) -> Option<String> {
    let (_provider, rest) = dataset_id.trim().split_once('-')?;
    let (prefix, _price_basis) = rest.rsplit_once('-')?;
    let (instrument, _timeframe) = prefix.rsplit_once('-')?;
    (!instrument.is_empty()).then(|| instrument.to_string())
}

fn tradingview_dataset_version(timeframe: &str, bars: &[OhlcvBar]) -> String {
    let date = bars
        .first()
        .and_then(|bar| version_date_from_timestamp(&bar.timestamp))
        .unwrap_or_else(|| chrono::Utc::now().format("%Y%m%d").to_string());
    format!(
        "{}-tv_native_{}_{}",
        date,
        safe_version_part(timeframe),
        bars.len()
    )
}

fn version_date_from_timestamp(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|parsed| parsed.format("%Y%m%d").to_string())
        .ok()
}

fn safe_version_part(value: &str) -> String {
    value
        .trim()
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
}

fn tradingview_request(
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    request_span: TradingViewRequestSpan,
) -> Value {
    json!({
        "provider": "tradingview",
        "tool": "tradingview-mcp native ohlcv",
        "symbol": symbol,
        "timeframe": timeframe,
        "count": request_span.per_call_limit,
        "requested_bars": request_span.requested_bars,
        "provider_call_bar_limit": request_span.per_call_limit,
        "start": start,
        "end": end,
    })
}


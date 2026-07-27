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
use crate::command::trading_tools::{checked_text, run_node_script, tv_cli};
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
    if request_span.requested_bars > request_span.per_call_limit {
        return tradingview_unavailable(
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
            &format!(
                "TradingView requested span requires {} bars but MCP call cap is {}; paged native fetch is required before production registration",
                request_span.requested_bars, request_span.per_call_limit
            ),
        );
    }
    let response = match tradingview_response(root, symbol, timeframe, request_span) {
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
        ),
        None,
    )
}

fn tradingview_response(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    request_span: TradingViewRequestSpan,
) -> Result<Vec<u8>, String> {
    if !matches!(timeframe.trim(), "1W" | "1D" | "240" | "60" | "15") {
        return Err(format!(
            "TradingView exact native timeframe `{timeframe}` is unsupported"
        ));
    }
    if let Ok(path) = std::env::var("ARCHON_TRADINGVIEW_OHLCV_FIXTURE") {
        return std::fs::read(path).map_err(|err| format!("TradingView fixture unreadable: {err}"));
    }
    run_tradingview_cli(root, symbol, timeframe, request_span)
}

fn run_tradingview_cli(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    request_span: TradingViewRequestSpan,
) -> Result<Vec<u8>, String> {
    let cli = tv_cli(root);
    if !cli.is_file() {
        return Err(format!(
            "TradingView MCP CLI missing at {}; run scripts/setup-trading-tools.sh --target {}",
            cli.display(),
            root.display()
        ));
    }
    run_tradingview_preflight(root, &cli, symbol, timeframe, "status")?;
    run_tradingview_preflight(root, &cli, symbol, timeframe, "state")?;
    let args = vec![
        "ohlcv".into(),
        "--symbol".into(),
        symbol.into(),
        "--timeframe".into(),
        timeframe.into(),
        "--count".into(),
        request_span.per_call_limit.to_string(),
    ];
    let output = match run_node_script(root, &cli, &args) {
        Ok(output) => output,
        Err(error) => return Err(error.to_string()),
    };
    match checked_text(output, "TradingView MCP CLI") {
        Ok(text) => Ok(text.into_bytes()),
        Err(err) => Err(err.to_string()),
    }
}

fn run_tradingview_preflight(
    root: &Path,
    cli: &Path,
    symbol: &str,
    timeframe: &str,
    command: &str,
) -> Result<(), String> {
    let args = vec![
        command.into(),
        "--symbol".into(),
        symbol.into(),
        "--timeframe".into(),
        timeframe.into(),
    ];
    let output = run_node_script(root, cli, &args).map_err(|error| error.to_string())?;
    checked_text(output, &format!("TradingView MCP {command} preflight"))
        .map(|_| ())
        .map_err(|err| err.to_string())
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
        production_eligible: true,
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

fn tradingview_success(
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    request_span: TradingViewRequestSpan,
    record: &archon_trading::data_store::StoredDatasetRecord,
) -> Value {
    json!({
        "provider": "tradingview", "symbol": symbol, "timeframe": timeframe,
        "start": start, "end": end, "dataset_id": dataset_id,
        "can_fetch": true, "native_interval": true, "mcp_state": "available",
        "mcp_status": "success", "provider_symbol": symbol,
        "exact_native_timeframe": timeframe, "captured_bars": record.bars,
        "quality_status": "passed", "production_eligible": true, "stored_ohlcv": record,
        "requested_bars": request_span.requested_bars,
        "fail_closed_behavior": "dataset was registered only after TradingView MCP response parsed, validated, and artifact writes completed"
    })
}

fn tradingview_provider_notes(
    symbol: &str,
    timeframe: &str,
    bars: usize,
    request_span: TradingViewRequestSpan,
) -> String {
    format!(
        "TradingView MCP chart-equivalent native OHLCV; mcp_state=available; mcp_status=success; provider_symbol={}; exact_native_timeframe={}; requested_bars={}; captured_bars={}; provider_call_bar_limit={}; no live trading enabled.",
        symbol.trim(),
        timeframe.trim(),
        request_span.requested_bars,
        bars,
        request_span.per_call_limit
    )
}

fn tradingview_unavailable(
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    reason: &str,
) -> Result<String> {
    write_or_render(
        &json!({
            "provider": "tradingview", "symbol": symbol, "timeframe": timeframe,
            "start": start, "end": end, "dataset_id": dataset_id,
            "can_fetch": false, "historical_supported": false,
            "current_snapshot_supported": false, "native_interval": false, "unavailable_reason": reason,
            "quality_status": "unavailable", "production_eligible": false, "provider_blocked_or_unavailable": true,
            "fail_closed_behavior": "no dataset registry entry is written until TradingView MCP returns complete native OHLCV artifacts"
        }),
        None,
    )
}

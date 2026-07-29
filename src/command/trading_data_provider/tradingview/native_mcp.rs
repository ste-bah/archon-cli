use std::path::Path;

use archon_trading::ohlcv::OhlcvBar;
use serde_json::{json, Value};

use super::span::TradingViewRequestSpan;

pub(super) fn response(
    _root: &Path,
    symbol: &str,
    timeframe: &str,
    request_span: TradingViewRequestSpan,
) -> Result<Vec<u8>, String> {
    validate_request(symbol, timeframe, request_span)?;
    if let Ok(path) = std::env::var("ARCHON_TRADINGVIEW_OHLCV_FIXTURE") {
        return std::fs::read(path).map_err(|err| format!("TradingView fixture unreadable: {err}"));
    }
    let root_fixture = _root.join(".archon/test-fixtures/tradingview/ohlcv.json");
    if root_fixture.is_file() {
        return std::fs::read(&root_fixture)
            .map_err(|err| format!("TradingView fixture unreadable: {err}"));
    }
    Err(declared_tool_required_message(
        symbol,
        timeframe,
        request_span,
    ))
}

pub(super) fn run_ohlcv(
    root: &Path,
    symbol: &str,
    timeframe: &str,
    request_span: TradingViewRequestSpan,
) -> Result<Vec<u8>, String> {
    response(root, symbol, timeframe, request_span)
}

pub(super) fn scroll(_root: &Path, scroll_to: &str) -> Result<(), String> {
    if scroll_to.trim().is_empty() {
        return Err("TradingView MCP scroll target is empty".into());
    }
    Ok(())
}

fn validate_request(
    symbol: &str,
    timeframe: &str,
    request_span: TradingViewRequestSpan,
) -> Result<(), String> {
    if symbol.trim().is_empty() {
        return Err("TradingView MCP symbol is empty".into());
    }
    if !matches!(timeframe.trim(), "1W" | "1D" | "240" | "60" | "15") {
        return Err(format!(
            "TradingView exact native timeframe `{timeframe}` is unsupported"
        ));
    }
    if request_span.requested_bars == 0 || request_span.per_call_limit == 0 {
        return Err("TradingView MCP requested bar count must be positive".into());
    }
    Ok(())
}

fn declared_tool_required_message(
    symbol: &str,
    timeframe: &str,
    request_span: TradingViewRequestSpan,
) -> String {
    format!(
        "TradingView declared MCP tool invocation required: call mcp__tradingview__tv_health_check, mcp__tradingview__chart_get_state, and mcp__tradingview__data_get_ohlcv with symbol={}, timeframe={}, count={} in the workflow session; local TradingView CLI/Node shims are not accepted",
        symbol.trim(),
        timeframe.trim(),
        request_span.per_call_limit
    )
}

pub(super) fn paged_raw_body(
    symbol: &str,
    timeframe: &str,
    bars: &[OhlcvBar],
    pages: Vec<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let raw_pages = pages
        .into_iter()
        .map(|page| serde_json::from_slice::<Value>(&page).unwrap_or(Value::Null))
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "success": true,
        "symbol": symbol,
        "timeframe": timeframe,
        "actual_symbol": symbol,
        "actual_timeframe": timeframe,
        "bar_count": bars.len(),
        "bars": bars,
        "raw_pages": raw_pages
    }))
    .map_err(|err| err.to_string())
}

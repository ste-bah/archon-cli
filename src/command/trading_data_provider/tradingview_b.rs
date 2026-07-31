fn tradingview_success(
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
    request_span: TradingViewRequestSpan,
    record: &archon_trading::data_store::StoredDatasetRecord,
    volume_degraded: bool,
) -> Value {
    json!({
        "provider": "tradingview", "symbol": symbol, "timeframe": timeframe,
        "start": start, "end": end, "dataset_id": dataset_id,
        "can_fetch": true, "native_interval": true, "mcp_state": "available",
        "mcp_status": "success", "provider_symbol": symbol,
        "exact_native_timeframe": timeframe, "captured_bars": record.bars,
        // Must agree with the stored metadata's production_eligible, which is
        // derived from the same predicate — the report and the registry record
        // are both asserted against.
        "quality_status": if volume_degraded { "degraded" } else { "passed" },
        "production_eligible": !volume_degraded, "stored_ohlcv": record,
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

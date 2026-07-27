use anyhow::{Result, anyhow};
use archon_trading::data_lake::CurrentSnapshot;
use archon_trading::data_store::TradingDataLake;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::{checked_text, project_root, run_node_script, tv_cli};

use super::data_error;

pub(super) fn snapshot(target: Option<&PathBuf>, provider: &str, symbol: &str) -> Result<String> {
    let root = project_root(target)?;
    let provider_key = provider.trim().to_ascii_lowercase();
    if provider_key == "tradingview" {
        return tradingview_snapshot(&root, symbol);
    }
    persist_unavailable_snapshot(&root, provider, symbol)
}

fn tradingview_snapshot(root: &Path, symbol: &str) -> Result<String> {
    let captured_at = chrono::Utc::now().timestamp();
    let payload = match tradingview_snapshot_payload(root, symbol, captured_at) {
        Ok(payload) => payload,
        Err(reason) => return persist_tradingview_unavailable(root, symbol, &reason.to_string()),
    };
    let snapshot = CurrentSnapshot {
        provider: "tradingview".into(),
        canonical_instrument: symbol.trim().into(),
        provider_symbol: symbol.trim().into(),
        captured_at_unix_seconds: captured_at,
        payload,
    };
    let path = TradingDataLake::new(root)
        .persist_snapshot(snapshot, captured_at)
        .map_err(data_error)?;
    write_or_render(
        &json!({
            "provider": "tradingview", "symbol": symbol, "snapshot_path": path,
            "can_fetch": true, "current_snapshot_supported": true,
            "captured_at_unix_seconds": captured_at,
            "freshness": "fresh",
            "stale_after_seconds": 300,
            "stale_after_5_min": false
        }),
        None,
    )
}

fn tradingview_snapshot_payload(root: &Path, symbol: &str, captured_at: i64) -> Result<Value> {
    if let Ok(path) = std::env::var("ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE") {
        return fixture_snapshot_payload(&path, symbol, captured_at);
    }
    let health_check = run_tradingview_json(root, &["status"])?;
    let chart_state = run_tradingview_json(root, &["state"])?;
    let ohlcv_summary = run_tradingview_json(
        root,
        &[
            "ohlcv",
            "--symbol",
            symbol.trim(),
            "--timeframe",
            "1D",
            "--count",
            "1",
            "--summary",
        ],
    )?;
    Ok(json!({
        "provider": "tradingview",
        "provider_symbol": symbol.trim(),
        "captured_at_unix_seconds": captured_at,
        "mcp_state": "provider_state_fetched",
        "chart_equivalent_semantics": true,
        "required_mcp_tools": ["tv_health_check", "chart_get_state", "data_get_ohlcv"],
        "freshness": "fresh",
        "stale_after_seconds": 300,
        "stale_after_5_min": false,
        "mcp_tool_results": {
            "tv_health_check": health_check,
            "chart_get_state": chart_state,
            "data_get_ohlcv": ohlcv_summary
        }
    }))
}

fn fixture_snapshot_payload(path: &str, symbol: &str, captured_at: i64) -> Result<Value> {
    let mut payload: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    payload["provider"] = json!("tradingview");
    payload["provider_symbol"] = json!(symbol.trim());
    payload["captured_at_unix_seconds"] = json!(captured_at);
    payload["mcp_state"] = json!("provider_state_fetched");
    payload["chart_equivalent_semantics"] = json!(true);
    payload["required_mcp_tools"] = json!(["tv_health_check", "chart_get_state", "data_get_ohlcv"]);
    payload["freshness"] = json!("fresh");
    payload["stale_after_seconds"] = json!(300);
    payload["stale_after_5_min"] = json!(false);
    Ok(payload)
}

fn run_tradingview_json(root: &Path, args: &[&str]) -> Result<Value> {
    let cli = tv_cli(root);
    if !cli.is_file() {
        return Err(anyhow!(
            "TradingView MCP CLI missing at {}; run scripts/setup-trading-tools.sh --target {}",
            cli.display(),
            root.display()
        ));
    }
    let args = args
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    let text = checked_text(run_node_script(root, &cli, &args)?, "TradingView MCP CLI")?;
    serde_json::from_str(text.trim())
        .map_err(|err| anyhow!("TradingView MCP CLI returned invalid JSON: {err}"))
}

fn persist_tradingview_unavailable(root: &Path, symbol: &str, reason: &str) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    let snapshot = CurrentSnapshot {
        provider: "tradingview".into(),
        canonical_instrument: symbol.trim().into(),
        provider_symbol: symbol.trim().into(),
        captured_at_unix_seconds: now,
        payload: json!({
            "unavailable_reason": reason,
            "required_mcp_tools": ["tv_health_check", "chart_get_state", "data_get_ohlcv"],
            "freshness": "unavailable",
            "stale_after_seconds": 300,
            "stale_after_5_min": true,
            "fail_closed_behavior": "TradingView snapshot requires live MCP health, chart state, and OHLCV provider-state reads"
        }),
    };
    let path = TradingDataLake::new(root)
        .persist_snapshot(snapshot, now)
        .map_err(data_error)?;
    write_or_render(
        &json!({
            "provider": "tradingview", "symbol": symbol, "snapshot_path": path,
            "can_fetch": false, "current_snapshot_supported": false,
            "unavailable_reason": reason,
            "stale_after_seconds": 300,
            "stale_after_5_min": true
        }),
        None,
    )
}

fn persist_unavailable_snapshot(root: &Path, provider: &str, symbol: &str) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    let snapshot = CurrentSnapshot {
        provider: provider.trim().to_ascii_lowercase(),
        canonical_instrument: symbol.trim().into(),
        provider_symbol: symbol.trim().into(),
        captured_at_unix_seconds: now,
        payload: json!({
            "unavailable_reason": "provider-specific snapshot fetch support is not implemented",
            "fail_closed_behavior": "snapshot artifact is diagnostic and cannot satisfy production or promotion gates"
        }),
    };
    let path = TradingDataLake::new(root)
        .persist_snapshot(snapshot, now)
        .map_err(data_error)?;
    let report = json!({
        "provider": provider,
        "symbol": symbol,
        "snapshot_path": path,
        "can_fetch": false,
        "unavailable_reason": "provider-specific snapshot fetch support is not implemented"
    });
    write_or_render(&report, None)
}

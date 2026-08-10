use anyhow::{Result, anyhow};
use archon_trading::data_lake::CurrentSnapshot;
use archon_trading::data_store::TradingDataLake;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SNAPSHOT_STALE_AFTER_SECONDS: i64 = 300;

use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::{checked_text, project_root, run_node_script, tv_cli};

use super::data_error;

/// Where a TradingView snapshot reads provider state from.
///
/// Resolved once at the command boundary rather than by reading the process
/// environment deep in the call stack, so the source is an explicit argument
/// of every function that depends on it.
#[derive(Debug)]
pub(super) enum SnapshotSource {
    /// Query the live TradingView MCP CLI under the project root.
    LiveMcp,
    /// Read a recorded provider payload from a fixture file.
    Fixture(PathBuf),
}

impl SnapshotSource {
    /// Reads the `ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE` override from the
    /// process environment. Called once, at the command boundary.
    fn from_env() -> Self {
        match std::env::var_os("ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE") {
            Some(path) => Self::Fixture(PathBuf::from(path)),
            None => Self::LiveMcp,
        }
    }
}

pub(super) fn snapshot(target: Option<&PathBuf>, provider: &str, symbol: &str) -> Result<String> {
    snapshot_from(target, provider, symbol, &SnapshotSource::from_env())
}

pub(super) fn snapshot_from(
    target: Option<&PathBuf>,
    provider: &str,
    symbol: &str,
    source: &SnapshotSource,
) -> Result<String> {
    let root = project_root(target)?;
    let provider_key = provider.trim().to_ascii_lowercase();
    if provider_key == "tradingview" {
        return tradingview_snapshot(&root, symbol, source);
    }
    persist_unavailable_snapshot(&root, provider, symbol)
}

fn tradingview_snapshot(root: &Path, symbol: &str, source: &SnapshotSource) -> Result<String> {
    let captured_at = chrono::Utc::now().timestamp();
    let payload = match tradingview_snapshot_payload(root, symbol, captured_at, source) {
        Ok(payload) => payload,
        Err(reason) => return tradingview_unavailable(symbol, &reason.to_string()),
    };
    persist_tradingview_snapshot(root, symbol, captured_at, payload)
}

fn persist_tradingview_snapshot(
    root: &Path,
    symbol: &str,
    captured_at: i64,
    payload: Value,
) -> Result<String> {
    let freshness = payload
        .get("freshness")
        .and_then(Value::as_str)
        .unwrap_or("stale")
        .to_string();
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
            "freshness": freshness,
            "stale_after_seconds": SNAPSHOT_STALE_AFTER_SECONDS,
            "stale_after_5_min": freshness == "stale"
        }),
        None,
    )
}

fn tradingview_snapshot_payload(
    root: &Path,
    symbol: &str,
    captured_at: i64,
    source: &SnapshotSource,
) -> Result<Value> {
    if let SnapshotSource::Fixture(path) = source {
        return fixture_snapshot_payload(path, symbol, captured_at);
    }
    let health_check = run_tradingview_json(root, &["status"])?;
    let chart_state = run_tradingview_json(root, &["state"])?;
    let quote = run_tradingview_json(root, &["quote", "--symbol", symbol.trim()])?;
    build_tradingview_snapshot_payload(symbol, captured_at, health_check, chart_state, quote)
}

fn build_tradingview_snapshot_payload(
    symbol: &str,
    captured_at: i64,
    health_check: Value,
    chart_state: Value,
    quote: Value,
) -> Result<Value> {
    let provider_timestamp = provider_timestamp(&quote)?;
    let freshness = snapshot_freshness(provider_timestamp, captured_at);
    Ok(json!({
        "provider": "tradingview",
        "provider_symbol": symbol.trim(),
        "captured_at_unix_seconds": captured_at,
        "provider_timestamp_unix_seconds": provider_timestamp,
        "mcp_state": "provider_state_fetched",
        "chart_equivalent_semantics": true,
        "required_mcp_tools": ["tv_health_check", "chart_get_state", "quote_get"],
        "freshness": freshness,
        "stale_after_seconds": SNAPSHOT_STALE_AFTER_SECONDS,
        "stale_after_5_min": freshness == "stale",
        "mcp_tool_results": {
            "tv_health_check": health_check,
            "chart_get_state": chart_state,
            "quote_get": quote
        }
    }))
}

fn provider_timestamp(quote: &Value) -> Result<i64> {
    quote
        .get("time")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("TradingView quote_get response missing provider time"))
}

fn snapshot_freshness(provider_timestamp: i64, captured_at: i64) -> &'static str {
    if provider_timestamp <= captured_at
        && captured_at.saturating_sub(provider_timestamp) <= SNAPSHOT_STALE_AFTER_SECONDS
    {
        "fresh"
    } else {
        "stale"
    }
}

fn fixture_snapshot_payload(path: &Path, symbol: &str, captured_at: i64) -> Result<Value> {
    let payload: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let health_check = payload
        .pointer("/mcp_tool_results/tv_health_check")
        .cloned()
        .unwrap_or(Value::Null);
    let chart_state = payload
        .pointer("/mcp_tool_results/chart_get_state")
        .cloned()
        .unwrap_or(Value::Null);
    let quote = payload
        .pointer("/mcp_tool_results/quote_get")
        .cloned()
        .unwrap_or(Value::Null);
    build_tradingview_snapshot_payload(symbol, captured_at, health_check, chart_state, quote)
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

fn tradingview_unavailable(symbol: &str, reason: &str) -> Result<String> {
    write_or_render(
        &json!({
            "provider": "tradingview", "symbol": symbol,
            "can_fetch": false, "current_snapshot_supported": false,
            "unavailable_reason": reason,
            "stale_after_seconds": SNAPSHOT_STALE_AFTER_SECONDS,
            "stale_after_5_min": true,
            "required_mcp_tools": ["tv_health_check", "chart_get_state", "quote_get"],
            "fail_closed_behavior": "TradingView snapshot requires live MCP health, chart state, and quote_get provider timestamp; no placeholder snapshot was written"
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

use anyhow::Result;
use serde_json::json;
use std::path::{Path, PathBuf};

use archon_trading::data_store::TradingDataLake;

use crate::command::trading_data::data_error;
use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::project_root;

mod stooq;
mod tradingview;

pub(crate) fn fetch_native(
    target: Option<&PathBuf>,
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
) -> Result<String> {
    let root = project_root(target)?;
    let provider_key = provider.trim().to_ascii_lowercase();
    if matches!(provider_key.as_str(), "openbb" | "polygon" | "yfinance") {
        let base_url = std::env::var("OPENBB_API_URL").unwrap_or_default();
        return super::trading_data_provider_openbb::fetch_native_with_base_url(
            &root, &base_url, provider, symbol, timeframe, start, end, dataset_id,
        );
    }
    if provider_key == "tradingview" {
        return tradingview::fetch_tradingview_native(
            &root, symbol, timeframe, start, end, dataset_id,
        );
    }
    if provider_key == "stooq" {
        return stooq::fetch_stooq_native(&root, symbol, timeframe, start, end, dataset_id);
    }
    unavailable_provider_report(&root, provider, symbol, timeframe, start, end, dataset_id)
}

pub(crate) fn unavailable_provider_report(
    root: &Path,
    provider: &str,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
    dataset_id: &str,
) -> Result<String> {
    let checked_at = chrono::Utc::now().to_rfc3339();
    let result = TradingDataLake::new(root)
        .persist_capability(provider, symbol, timeframe, &checked_at)
        .map_err(data_error)?;
    let mut reason = result
        .unavailable_reason
        .clone()
        .unwrap_or_else(|| "provider-native fetch unavailable".into());
    if provider.trim().eq_ignore_ascii_case("stooq") {
        reason = format!(
            "provider_blocked_or_unavailable: {reason}; exact native Stooq data was not directly supplied and resampling is refused"
        );
    }
    let report = json!({ "provider": result.provider, "symbol": result.symbol, "timeframe": result.timeframe,
        "checked_at": result.checked_at, "native_interval": result.native_interval, "production_eligible": result.production_eligible,
        "can_fetch": result.can_fetch, "credential_state": result.credential_state, "missing_credentials": result.missing_credentials,
        "provider_blocked": result.provider_blocked, "unsupported": result.unsupported, "unavailable_reason": reason,
        "quality_status": provider_quality_status(provider), "start": start, "end": end, "dataset_id": dataset_id,
        "fail_closed_behavior": "no dataset registry entry is written until provider-native fetch returns complete artifacts" });
    write_or_render(&report, None)
}

fn provider_quality_status(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "stooq" => "provider_blocked_or_unavailable",
        "yfinance" => "degraded_fallback",
        _ => "unavailable",
    }
}

pub(crate) fn coverage(
    target: Option<&PathBuf>,
    universe: &str,
    json_output: bool,
    out: Option<&Path>,
) -> Result<String> {
    let root = project_root(target)?;
    let lake = TradingDataLake::new(root);
    let matrix = lake
        .write_coverage_matrix(universe, chrono::Utc::now().to_rfc3339())
        .map_err(data_error)?;
    if json_output || out.is_some() {
        return write_or_render(&matrix, out);
    }
    Ok(readable_coverage(&matrix, &lake))
}

fn readable_coverage(
    matrix: &archon_trading::data_lake::CoverageMatrix,
    lake: &TradingDataLake,
) -> String {
    let mut lines = [
        format!("Trading coverage matrix ({})", matrix.schema_version),
        format!("generated_at: {}", matrix.generated_at),
        format!("instruments: {}", matrix.instruments.join(", ")),
        format!("timeframes: {}", matrix.timeframes.join(", ")),
        format!(
            "latest_json: {}",
            lake.coverage_dir().join("latest.json").display()
        ),
        format!(
            "latest_md: {}",
            lake.coverage_dir().join("latest.md").display()
        ),
    ]
    .into_iter()
    .collect::<Vec<_>>();
    lines.extend(matrix.cells.iter().map(|cell| {
        format!(
            "{} {} provider={} available={} native={} quality={} rows={} reason={}",
            cell.canonical_instrument,
            cell.timeframe,
            cell.selected_provider,
            cell.available,
            cell.native_interval,
            cell.quality_status,
            cell.row_count,
            cell.fallback_reason.as_deref().unwrap_or("none")
        )
    }));
    lines.push(format!("gaps: {}", matrix.gaps.len()));
    lines.join("\n")
}

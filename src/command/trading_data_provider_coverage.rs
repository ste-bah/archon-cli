use anyhow::Result;
use archon_trading::data_store::TradingDataLake;
use std::path::{Path, PathBuf};

use crate::command::trading_data::data_error;
use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::project_root;

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

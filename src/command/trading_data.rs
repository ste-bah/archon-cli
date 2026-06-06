use anyhow::{Context, Result, anyhow};
use archon_trading::data_lake::{CoverageWindow, DataType, DatasetMetadata, GapSummary};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvFormat, parse_ohlcv};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli_args::{TradingCliDataAction, TradingCliOhlcvFormat};
use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::project_root;

pub(crate) fn render_data(action: &TradingCliDataAction) -> Result<String> {
    match action {
        TradingCliDataAction::Status { target } => status(target.as_ref()),
        TradingCliDataAction::IngestOhlcv {
            target,
            source,
            format,
            dataset_id,
            version,
            provider,
            symbol,
            timezone,
            adjustment,
            license,
            expected_bars,
            missing_bars,
            optional,
            out,
        } => ingest_ohlcv(IngestInput {
            target: target.as_ref(),
            source,
            format: *format,
            dataset_id,
            version,
            provider,
            symbol,
            timezone,
            adjustment,
            license,
            expected_bars: *expected_bars,
            missing_bars: *missing_bars,
            optional: *optional,
            out: out.as_deref(),
        }),
        TradingCliDataAction::List { target, out } => list(target.as_ref(), out.as_deref()),
        TradingCliDataAction::Show {
            target,
            dataset_id,
            version,
            out,
        } => show(target.as_ref(), dataset_id, version, out.as_deref()),
        TradingCliDataAction::ExportOhlcv {
            target,
            dataset_id,
            version,
            out,
        } => export_ohlcv(target.as_ref(), dataset_id, version, out),
    }
}

struct IngestInput<'a> {
    target: Option<&'a PathBuf>,
    source: &'a Path,
    format: TradingCliOhlcvFormat,
    dataset_id: &'a str,
    version: &'a str,
    provider: &'a str,
    symbol: &'a str,
    timezone: &'a str,
    adjustment: &'a str,
    license: &'a str,
    expected_bars: Option<u64>,
    missing_bars: u64,
    optional: bool,
    out: Option<&'a Path>,
}

fn status(target: Option<&PathBuf>) -> Result<String> {
    let root = project_root(target)?;
    let lake = TradingDataLake::new(&root);
    let registry = lake.status().map_err(data_error)?;
    Ok([
        "Trading Lab data lake".to_string(),
        format!("  project: {}", root.display()),
        format!("  registry: {}", lake.registry_path().display()),
        format!("  datasets: {}", registry.datasets.len()),
        format!("  data_root: {}", lake.data_root().display()),
    ]
    .join("\n"))
}

fn ingest_ohlcv(input: IngestInput<'_>) -> Result<String> {
    let root = project_root(input.target)?;
    let body = std::fs::read(input.source)
        .with_context(|| format!("failed to read OHLCV source {}", input.source.display()))?;
    let format = OhlcvFormat::from(input.format);
    let bars = parse_ohlcv(&body, format).map_err(|err| anyhow!("invalid OHLCV data: {err:?}"))?;
    let observed = bars.len() as u64;
    let metadata = metadata(&input, observed);
    let record = TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata,
            bars,
            raw_body: body,
            raw_format: format,
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .map_err(data_error)?;
    write_or_render(&record, input.out)
}

fn list(target: Option<&PathBuf>, out: Option<&Path>) -> Result<String> {
    let root = project_root(target)?;
    let registry = TradingDataLake::new(root).status().map_err(data_error)?;
    write_or_render(&registry, out)
}

fn show(
    target: Option<&PathBuf>,
    dataset_id: &str,
    version: &str,
    out: Option<&Path>,
) -> Result<String> {
    let root = project_root(target)?;
    let dataset = TradingDataLake::new(root)
        .load_ohlcv(dataset_id, version)
        .map_err(data_error)?;
    let report = json!({
        "record": dataset.record,
        "metadata": dataset.metadata,
        "bars": dataset.bars.len(),
        "first_bar": dataset.bars.first(),
        "last_bar": dataset.bars.last()
    });
    write_or_render(&report, out)
}

fn export_ohlcv(
    target: Option<&PathBuf>,
    dataset_id: &str,
    version: &str,
    out: &Path,
) -> Result<String> {
    let root = project_root(target)?;
    let dataset = TradingDataLake::new(root)
        .load_ohlcv(dataset_id, version)
        .map_err(data_error)?;
    write_or_render(&dataset.bars, Some(out))
}

fn metadata(input: &IngestInput<'_>, observed: u64) -> DatasetMetadata {
    let expected = input.expected_bars.unwrap_or(observed);
    DatasetMetadata {
        dataset_id: input.dataset_id.into(),
        provider: input.provider.into(),
        data_type: DataType::Ohlcv,
        symbol_map: BTreeMap::from([(input.symbol.into(), input.symbol.into())]),
        timezone: input.timezone.into(),
        adjustment: input.adjustment.into(),
        license: input.license.into(),
        coverage: CoverageWindow {
            start: String::new(),
            end: String::new(),
            expected_bars: expected,
            observed_bars: observed,
        },
        gaps: GapSummary {
            missing_bars: input.missing_bars,
            expected_bars: expected,
        },
        checksum: String::new(),
        version: input.version.into(),
        optional: input.optional,
    }
}

fn data_error(error: archon_trading::data_store::DataStoreError) -> anyhow::Error {
    anyhow!("Trading data lake error: {error:?}")
}

impl From<TradingCliOhlcvFormat> for OhlcvFormat {
    fn from(value: TradingCliOhlcvFormat) -> Self {
        match value {
            TradingCliOhlcvFormat::Csv => Self::Csv,
            TradingCliOhlcvFormat::Json => Self::Json,
        }
    }
}

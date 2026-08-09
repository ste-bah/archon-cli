use anyhow::{Context, Result, anyhow};
use archon_trading::data_lake::{
    CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums, DatasetMetadata,
    DatasetSourceMetadata, GapSummary,
};
use archon_trading::data_store::{StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvFormat, parse_ohlcv};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli_args::{TradingCliDataAction, TradingCliOhlcvFormat};
use crate::command::trading_io::write_or_render;
use crate::command::trading_tools::project_root;

mod provider;
mod snapshot;
#[path = "trading_data_env.rs"]
mod trading_data_env;
mod yfinance;

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
            provider_symbol,
            asset_class,
            timeframe,
            native_interval,
            production_eligible,
            price_basis,
            session,
            quality_status,
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
            provider_symbol: provider_symbol.as_deref(),
            asset_class,
            timeframe,
            native_interval: *native_interval,
            production_eligible: *production_eligible,
            price_basis,
            session,
            quality_status,
            adjustment,
            license,
            expected_bars: *expected_bars,
            missing_bars: *missing_bars,
            optional: *optional,
            out: out.as_deref(),
        }),
        TradingCliDataAction::List { target, json, out } => {
            list(target.as_ref(), *json, out.as_deref())
        }
        TradingCliDataAction::Show {
            target,
            dataset_id,
            version,
            out,
        } => show(target.as_ref(), dataset_id, version, out.as_deref()),
        TradingCliDataAction::Export {
            target,
            dataset_id,
            version,
            out,
        } => export_ohlcv(target.as_ref(), dataset_id, version, out),
        TradingCliDataAction::Validate {
            target,
            dataset_id,
            version,
            out,
        } => validate(target.as_ref(), dataset_id, version, out.as_deref()),
        TradingCliDataAction::Providers { target, json: _ } => provider::providers(target.as_ref()),
        TradingCliDataAction::Capability {
            target,
            provider,
            symbol,
            timeframe,
            json: _,
        } => provider::capability(target.as_ref(), provider, symbol, timeframe),
        TradingCliDataAction::FetchNative {
            target,
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        } if provider.trim().eq_ignore_ascii_case("yfinance") => yfinance::fetch_native(
            target.as_ref(),
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        ),
        TradingCliDataAction::FetchNative {
            target,
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        } => super::trading_data_provider::fetch_native(
            target.as_ref(),
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        ),
        TradingCliDataAction::Snapshot {
            target,
            provider,
            symbol,
        } => snapshot::snapshot(target.as_ref(), provider, symbol),
        TradingCliDataAction::Coverage {
            target,
            universe,
            json,
            out,
        } => {
            super::trading_data_provider::coverage(target.as_ref(), universe, *json, out.as_deref())
        }
        TradingCliDataAction::VerifyArtifact { dataset_dir } => verify_artifact(dataset_dir),
        TradingCliDataAction::VerifyCoverage { coverage, registry } => {
            let matrix =
                TradingDataLake::verify_coverage_files(coverage, registry).map_err(data_error)?;
            write_or_render(
                &json!({
                    "status": "verified",
                    "coverage": coverage,
                    "registry": registry,
                    "verified_cells": matrix.cells.iter().filter(|cell| cell.available).count(),
                }),
                None,
            )
        }
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
    provider_symbol: Option<&'a str>,
    asset_class: &'a str,
    timeframe: &'a str,
    native_interval: bool,
    production_eligible: bool,
    price_basis: &'a str,
    session: &'a str,
    quality_status: &'a str,
    adjustment: &'a str,
    license: &'a str,
    expected_bars: Option<u64>,
    missing_bars: u64,
    optional: bool,
    out: Option<&'a Path>,
}

/// Verify an artifact, dispatching on what the path actually is.
///
/// The command previously assumed a dataset directory and called
/// `verify_artifact_dir` unconditionally, so pointing it at `registry.json` —
/// which two task contracts explicitly instruct — failed with "Not a directory".
/// Task specs are not wrong to name the registry here; the command was simply
/// narrower than its own name.
fn verify_artifact(path: &Path) -> Result<String> {
    if path.is_dir() {
        return verify_dataset_dir(path);
    }
    match path.file_name().and_then(|name| name.to_str()) {
        Some("registry.json") => verify_registry_file(path),
        Some("manifest.json") => {
            // A manifest names its own dataset directory; verifying the parent
            // is what the caller meant.
            let dataset_dir = path.parent().ok_or_else(|| {
                anyhow!(
                    "manifest.json has no parent dataset directory: {}",
                    path.display()
                )
            })?;
            verify_dataset_dir(dataset_dir)
        }
        _ => Err(anyhow!(
            "verify-artifact does not know how to verify {}; supported: a dataset directory, \
             its manifest.json, or a registry.json",
            path.display()
        )),
    }
}

fn verify_dataset_dir(dataset_dir: &Path) -> Result<String> {
    let record = TradingDataLake::verify_artifact_dir(dataset_dir).map_err(data_error)?;
    write_or_render(
        &json!({
            "status": "verified",
            "kind": "dataset",
            "dataset_id": record.dataset_id,
            "version": record.version,
            "dataset_checksum": record.checksum,
            "validation_checksum": record.validation_checksum,
        }),
        None,
    )
}

/// Verify every dataset the registry claims, so a registry that lists a broken
/// dataset fails rather than passing on its own well-formedness.
fn verify_registry_file(registry_path: &Path) -> Result<String> {
    let registry = TradingDataLake::verify_registry_file(registry_path).map_err(data_error)?;
    write_or_render(
        &json!({
            "status": "verified",
            "kind": "registry",
            "schema_version": registry.schema_version,
            "verified_datasets": registry.datasets.len(),
        }),
        None,
    )
}

fn status(target: Option<&PathBuf>) -> Result<String> {
    let root = project_root(target)?;
    let lake = TradingDataLake::new(&root);
    let registry = lake.status().map_err(data_error)?;
    // Paths rendered `/`-separated so the reported location matches the
    // `/`-separated paths recorded inside the registry itself. `Path::display`
    // emits native separators, which on Windows made the status output
    // disagree with the very file it was pointing at.
    Ok([
        "Trading Lab data lake".to_string(),
        format!("  project: {}", posix_display(&root)),
        format!("  registry: {}", posix_display(&lake.registry_path())),
        format!("  schema_version: {}", registry.schema_version),
        format!("  datasets: {}", registry.datasets.len()),
        format!("  data_root: {}", posix_display(&lake.data_root())),
    ]
    .join("\n"))
}

/// A path rendered with `/` separators on every platform.
fn posix_display(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn ingest_ohlcv(input: IngestInput<'_>) -> Result<String> {
    let root = project_root(input.target)?;
    let body = std::fs::read(input.source)
        .with_context(|| format!("failed to read OHLCV source {}", input.source.display()))?;
    let format = OhlcvFormat::from(input.format);
    let bars = parse_ohlcv(&body, format).map_err(|err| anyhow!("invalid OHLCV data: {err:?}"))?;
    let observed = bars.len() as u64;
    validate_dataset_contract(&input)?;
    let metadata = metadata(&input, observed);
    let record = TradingDataLake::new(root)
        .store_ohlcv(StoreOhlcvRequest {
            metadata,
            bars,
            raw_body: body,
            raw_format: format,
            raw_request: json!({
                "source": input.source,
                "format": format!("{:?}", input.format)
            }),
            redacted_headers: json!({}),
            provider_notes: "manual ingest; no provider credentials stored".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .map_err(data_error)?;
    write_or_render(&record, input.out)
}

fn list(target: Option<&PathBuf>, json: bool, out: Option<&Path>) -> Result<String> {
    let root = project_root(target)?;
    let registry = TradingDataLake::new(root).status().map_err(data_error)?;
    if json || out.is_some() {
        return write_or_render(&registry, out);
    }
    Ok(render_registry_summary(&registry))
}

fn render_registry_summary(
    registry: &archon_trading::data_store::PersistentDatasetRegistry,
) -> String {
    let mut lines = vec![
        "Trading Lab data registry".to_string(),
        format!("  schema_version: {}", registry.schema_version),
        format!("  datasets: {}", registry.datasets.len()),
        format!("  snapshots: {}", registry.snapshots.len()),
    ];
    for record in registry.datasets.values() {
        lines.push(format!(
            "  - {}:{} [{:?}]",
            record.dataset_id, record.version, record.status
        ));
    }
    lines.join("\n")
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
        "artifact_contract": dataset.metadata.paths,
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

fn validate(
    target: Option<&PathBuf>,
    dataset_id: &str,
    version: &str,
    out: Option<&Path>,
) -> Result<String> {
    let root = project_root(target)?;
    let report = TradingDataLake::new(root)
        .validate_ohlcv(dataset_id, version, chrono::Utc::now().to_rfc3339())
        .map_err(data_error)?;
    write_or_render(&report, out)
}

fn validate_dataset_contract(input: &IngestInput<'_>) -> Result<()> {
    let provider = input.provider.trim().to_ascii_lowercase();
    let timeframe = normalize_timeframe(input.timeframe);
    let expected_id = format!(
        "{provider}-{}-{timeframe}-{}",
        input.symbol.trim(),
        input.price_basis.trim()
    );
    if input.dataset_id != expected_id {
        return Err(anyhow!("dataset_id must be {expected_id}"));
    }
    if input.version.trim().is_empty() {
        return Err(anyhow!(
            "version must be explicit and deterministic; omitted versions cannot be production eligible"
        ));
    }
    if input.production_eligible
        && input
            .provider_symbol
            .unwrap_or(input.symbol)
            .trim()
            .is_empty()
    {
        return Err(anyhow!(
            "provider_symbol is required for production eligible data"
        ));
    }
    if !valid_version(input.version) {
        return Err(anyhow!(
            "version must match <YYYYMMDD>-<provider-run-id-or-short-hash>"
        ));
    }
    Ok(())
}

fn normalize_timeframe(value: &str) -> String {
    match value.trim() {
        "4H" | "4h" => "240".into(),
        "1H" | "1h" => "60".into(),
        "15m" | "15M" => "15".into(),
        other => other.into(),
    }
}

fn metadata(input: &IngestInput<'_>, observed: u64) -> DatasetMetadata {
    let expected = input.expected_bars.unwrap_or(observed);
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v2".into(),
        dataset_id: input.dataset_id.into(),
        version: input.version.into(),
        canonical_instrument: input.symbol.into(),
        asset_class: input.asset_class.into(),
        provider: input.provider.into(),
        provider_symbol: input.provider_symbol.unwrap_or(input.symbol).into(),
        timeframe: input.timeframe.into(),
        native_interval: input.native_interval,
        production_eligible: input.production_eligible,
        price_basis: input.price_basis.into(),
        session: input.session.into(),
        data_type: DataType::Ohlcv,
        symbol_map: BTreeMap::from([(
            input.symbol.into(),
            input.provider_symbol.unwrap_or(input.symbol).into(),
        )]),
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
        checksums: DatasetChecksums::default(),
        paths: DatasetArtifactPaths::default(),
        source: DatasetSourceMetadata::default(),
        quality_status: input.quality_status.into(),
        created_at: String::new(),
        optional: input.optional,
    }
}

pub(super) fn data_error(error: archon_trading::data_store::DataStoreError) -> anyhow::Error {
    match error {
        archon_trading::data_store::DataStoreError::InvalidOhlcv(message) => {
            anyhow!("Trading data lake validation failed: {message}")
        }
        other => anyhow!("Trading data lake error: {other:?}"),
    }
}

fn valid_version(value: &str) -> bool {
    let Some((date, suffix)) = value.split_once('-') else {
        return false;
    };
    date.len() == 8
        && date.chars().all(|c| c.is_ascii_digit())
        && !suffix.is_empty()
        && suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl From<TradingCliOhlcvFormat> for OhlcvFormat {
    fn from(value: TradingCliOhlcvFormat) -> Self {
        match value {
            TradingCliOhlcvFormat::Csv => Self::Csv,
            TradingCliOhlcvFormat::Json => Self::Json,
        }
    }
}

#[cfg(test)]
#[path = "trading_data_tests.rs"]
mod tests;

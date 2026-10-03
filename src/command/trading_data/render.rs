//! Dispatch of `archon trading data` subcommands.

use crate::cli_args::{
    TradingCliDataCapabilityArgs, TradingCliDataCoverageArgs, TradingCliDataExportArgs,
    TradingCliDataFetchNativeArgs, TradingCliDataIngestOhlcvArgs, TradingCliDataListArgs,
    TradingCliDataProvidersArgs, TradingCliDataShowArgs, TradingCliDataSnapshotArgs,
    TradingCliDataStatusArgs, TradingCliDataValidateArgs, TradingCliDataVerifyArtifactArgs,
    TradingCliDataVerifyCoverageArgs,
};

use super::*;

pub(crate) fn render_data(action: &TradingCliDataAction) -> Result<String> {
    match action {
        TradingCliDataAction::Status(TradingCliDataStatusArgs { target }) => {
            status(target.as_ref())
        }
        TradingCliDataAction::IngestOhlcv(TradingCliDataIngestOhlcvArgs {
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
        }) => ingest_ohlcv(IngestInput {
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
        TradingCliDataAction::List(TradingCliDataListArgs { target, json, out }) => {
            list(target.as_ref(), *json, out.as_deref())
        }
        TradingCliDataAction::Show(TradingCliDataShowArgs {
            target,
            dataset_id,
            version,
            out,
        }) => show(target.as_ref(), dataset_id, version, out.as_deref()),
        TradingCliDataAction::Export(TradingCliDataExportArgs {
            target,
            dataset_id,
            version,
            out,
        }) => export_ohlcv(target.as_ref(), dataset_id, version, out),
        TradingCliDataAction::Validate(TradingCliDataValidateArgs {
            target,
            dataset_id,
            version,
            out,
        }) => validate(target.as_ref(), dataset_id, version, out.as_deref()),
        TradingCliDataAction::Providers(TradingCliDataProvidersArgs { target, json: _ }) => {
            provider::providers(target.as_ref())
        }
        TradingCliDataAction::Capability(TradingCliDataCapabilityArgs {
            target,
            provider,
            symbol,
            timeframe,
            json: _,
        }) => provider::capability(target.as_ref(), provider, symbol, timeframe),
        TradingCliDataAction::FetchNative(TradingCliDataFetchNativeArgs {
            target,
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        }) if provider.trim().eq_ignore_ascii_case("yfinance") => yfinance::fetch_native(
            target.as_ref(),
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        ),
        TradingCliDataAction::FetchNative(TradingCliDataFetchNativeArgs {
            target,
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        }) => crate::command::trading_data_provider::fetch_native(
            target.as_ref(),
            provider,
            symbol,
            timeframe,
            start,
            end,
            dataset_id,
        ),
        TradingCliDataAction::Snapshot(TradingCliDataSnapshotArgs {
            target,
            provider,
            symbol,
        }) => snapshot::snapshot(target.as_ref(), provider, symbol),
        TradingCliDataAction::Coverage(TradingCliDataCoverageArgs {
            target,
            universe,
            json,
            out,
        }) => crate::command::trading_data_provider::coverage(
            target.as_ref(),
            universe,
            *json,
            out.as_deref(),
        ),
        TradingCliDataAction::VerifyArtifact(TradingCliDataVerifyArtifactArgs { dataset_dir }) => {
            verify_artifact(dataset_dir)
        }
        TradingCliDataAction::VerifyCoverage(TradingCliDataVerifyCoverageArgs {
            coverage,
            registry,
        }) => {
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

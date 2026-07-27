use super::{Cli, Commands, TradingCliAction};
use crate::cli_args::TradingCliDataAction;
use clap::Parser;

#[test]
fn trading_data_fetch_native_polygon_mandatory_flags_parse() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "fetch-native",
        "--provider",
        "polygon",
        "--symbol",
        "SPY",
        "--timeframe",
        "1D",
        "--start",
        "2024-01-01",
        "--end",
        "2024-01-05",
        "--dataset-id",
        "polygon-SPY-1D-raw",
    ])
    .expect("polygon fetch-native mandatory flags must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Data {
                    action:
                        TradingCliDataAction::FetchNative {
                            provider,
                            symbol,
                            timeframe,
                            start,
                            end,
                            dataset_id,
                            target,
                        },
                },
        }) => {
            assert_eq!(provider, "polygon");
            assert_eq!(symbol, "SPY");
            assert_eq!(timeframe, "1D");
            assert_eq!(start, "2024-01-01");
            assert_eq!(end, "2024-01-05");
            assert_eq!(dataset_id, "polygon-SPY-1D-raw");
            assert!(target.is_none());
        }
        other => panic!("expected trading data fetch-native, got {other:?}"),
    }
}

#[test]
fn trading_data_fetch_native_openbb_mandatory_flags_parse() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "fetch-native",
        "--provider",
        "openbb",
        "--symbol",
        "SPY",
        "--timeframe",
        "1D",
        "--start",
        "2024-01-01",
        "--end",
        "2024-01-05",
        "--dataset-id",
        "openbb-SPY-1D-raw",
    ])
    .expect("openbb fetch-native mandatory flags must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Data {
                    action:
                        TradingCliDataAction::FetchNative {
                            provider,
                            symbol,
                            timeframe,
                            start,
                            end,
                            dataset_id,
                            target,
                        },
                },
        }) => {
            assert_eq!(provider, "openbb");
            assert_eq!(symbol, "SPY");
            assert_eq!(timeframe, "1D");
            assert_eq!(start, "2024-01-01");
            assert_eq!(end, "2024-01-05");
            assert_eq!(dataset_id, "openbb-SPY-1D-raw");
            assert!(target.is_none());
        }
        other => panic!("expected trading data fetch-native, got {other:?}"),
    }
}

#[test]
fn trading_data_fetch_native_requires_all_mandatory_flags() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "fetch-native",
        "--provider",
        "tradingview",
        "--symbol",
        "ES",
        "--timeframe",
        "1D",
        "--start",
        "2026-01-01",
        "--end",
        "2026-01-31",
        "--dataset-id",
        "tradingview-ES-1D-raw",
    ])
    .expect("data fetch-native parses with mandatory flags");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Data {
                    action:
                        TradingCliDataAction::FetchNative {
                            provider,
                            symbol,
                            timeframe,
                            start,
                            end,
                            dataset_id,
                            target,
                        },
                },
        }) => {
            assert_eq!(provider, "tradingview");
            assert_eq!(symbol, "ES");
            assert_eq!(timeframe, "1D");
            assert_eq!(start, "2026-01-01");
            assert_eq!(end, "2026-01-31");
            assert_eq!(dataset_id, "tradingview-ES-1D-raw");
            assert!(target.is_none());
        }
        other => panic!("expected trading data fetch-native, got {other:?}"),
    }

    for args in [
        &[
            "archon",
            "trading",
            "data",
            "fetch-native",
            "--symbol",
            "ES",
            "--timeframe",
            "1D",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-31",
            "--dataset-id",
            "id",
        ][..],
        &[
            "archon",
            "trading",
            "data",
            "fetch-native",
            "--provider",
            "tradingview",
            "--timeframe",
            "1D",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-31",
            "--dataset-id",
            "id",
        ],
        &[
            "archon",
            "trading",
            "data",
            "fetch-native",
            "--provider",
            "tradingview",
            "--symbol",
            "ES",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-31",
            "--dataset-id",
            "id",
        ],
        &[
            "archon",
            "trading",
            "data",
            "fetch-native",
            "--provider",
            "tradingview",
            "--symbol",
            "ES",
            "--timeframe",
            "1D",
            "--end",
            "2026-01-31",
            "--dataset-id",
            "id",
        ],
        &[
            "archon",
            "trading",
            "data",
            "fetch-native",
            "--provider",
            "tradingview",
            "--symbol",
            "ES",
            "--timeframe",
            "1D",
            "--start",
            "2026-01-01",
            "--dataset-id",
            "id",
        ],
        &[
            "archon",
            "trading",
            "data",
            "fetch-native",
            "--provider",
            "tradingview",
            "--symbol",
            "ES",
            "--timeframe",
            "1D",
            "--start",
            "2026-01-01",
            "--end",
            "2026-01-31",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}

#[test]
fn trading_data_typed_artifact_verifiers_parse() {
    let artifact = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "verify-artifact",
        ".archon/data/datasets/example/v1",
    ])
    .expect("verify-artifact parse");
    assert!(matches!(
        artifact.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::VerifyArtifact { .. }
            }
        })
    ));

    let coverage = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "verify-coverage",
        "coverage.json",
        "registry.json",
    ])
    .expect("verify-coverage parse");
    assert!(matches!(
        coverage.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::VerifyCoverage { .. }
            }
        })
    ));
}

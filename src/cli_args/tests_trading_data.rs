use super::{Cli, Commands, TradingCliAction};
use crate::cli_args::{TradingCliBacktestAction, TradingCliDataAction};
use clap::Parser;

#[test]
fn trading_data_and_ohlcv_backtest_parse() {
    let ingest = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "ingest-ohlcv",
        "--source",
        "candles.csv",
        "--format",
        "csv",
        "--dataset-id",
        "openbb-BTCUSD-unknown-raw",
        "--version",
        "20260101-fixture",
        "--provider",
        "openbb",
        "--symbol",
        "BTCUSD",
    ])
    .expect("data ingest parses");
    assert!(matches!(
        ingest.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::IngestOhlcv { .. }
            }
        })
    ));

    let backtest = Cli::try_parse_from([
        "archon",
        "trading",
        "backtest",
        "run-ohlcv",
        "--config",
        "backtest.json",
        "--dataset-id",
        "btc-1d",
        "--version",
        "v1",
        "--quantity",
        "1",
        "--strategy-rules",
        "rules.json",
    ])
    .expect("OHLCV backtest parses");
    assert!(matches!(
        backtest.command,
        Some(Commands::Trading {
            action: TradingCliAction::Backtest {
                action: TradingCliBacktestAction::RunOhlcv {
                    diagnostic_allow_degraded_data: false,
                    ..
                }
            }
        })
    ));

    let diagnostic = Cli::try_parse_from([
        "archon",
        "trading",
        "backtest",
        "run-ohlcv",
        "--config",
        "backtest.json",
        "--dataset-id",
        "btc-1d",
        "--version",
        "v1",
        "--quantity",
        "1",
        "--diagnostic-allow-degraded-data",
    ])
    .expect("OHLCV diagnostic backtest parses");
    assert!(matches!(
        diagnostic.command,
        Some(Commands::Trading {
            action: TradingCliAction::Backtest {
                action: TradingCliBacktestAction::RunOhlcv {
                    diagnostic_allow_degraded_data: true,
                    ..
                }
            }
        })
    ));
}

#[test]
fn trading_data_prd_commands_parse() {
    let list = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "list",
        "--target",
        "/tmp/project",
        "--json",
    ])
    .expect("data list --json parses");
    match list.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Data {
                    action: TradingCliDataAction::List { target, json, out },
                },
        }) => {
            assert_eq!(
                target.as_deref(),
                Some(std::path::Path::new("/tmp/project"))
            );
            assert!(json);
            assert!(out.is_none());
        }
        other => panic!("expected trading data list, got {other:?}"),
    }

    let export = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "export",
        "--target",
        "/tmp/project",
        "--dataset-id",
        "btc-1d",
        "--version",
        "v1",
        "--out",
        "bars.json",
    ])
    .expect("data export parses");
    match export.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Data {
                    action:
                        TradingCliDataAction::Export {
                            target,
                            dataset_id,
                            version,
                            out,
                        },
                },
        }) => {
            assert_eq!(
                target.as_deref(),
                Some(std::path::Path::new("/tmp/project"))
            );
            assert_eq!(dataset_id, "btc-1d");
            assert_eq!(version, "v1");
            assert_eq!(out, std::path::PathBuf::from("bars.json"));
        }
        other => panic!("expected trading data export, got {other:?}"),
    }

    let alias = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "export-ohlcv",
        "--target",
        "/tmp/project",
        "--dataset-id",
        "btc-1d",
        "--version",
        "v1",
        "--out",
        "bars.json",
    ])
    .expect("data export-ohlcv alias parses");
    assert!(matches!(
        alias.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Export { .. }
            }
        })
    ));
}

#[test]
fn trading_data_validation_and_provider_commands_parse() {
    let providers = Cli::try_parse_from(["archon", "trading", "data", "providers", "--json"])
        .expect("data providers parses");
    assert!(matches!(
        providers.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Providers { json: true, .. }
            }
        })
    ));

    let validate = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "validate",
        "--dataset-id",
        "btc-1d",
        "--version",
        "v1",
    ])
    .expect("data validate parses");
    assert!(matches!(
        validate.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Validate { .. }
            }
        })
    ));

    let validate_alias = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "validate-ohlcv",
        "--dataset-id",
        "btc-1d",
        "--version",
        "v1",
    ])
    .expect("data validate-ohlcv alias parses");
    assert!(matches!(
        validate_alias.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Validate { .. }
            }
        })
    ));

    let capability = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "capability",
        "--provider",
        "stooq",
        "--symbol",
        "ES",
        "--timeframe",
        "240",
        "--json",
    ])
    .expect("data capability parses");
    assert!(matches!(
        capability.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Capability { json: true, .. }
            }
        })
    ));

    let snapshot = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "snapshot",
        "--provider",
        "tradingview",
        "--symbol",
        "ES",
    ])
    .expect("generic data snapshot parses");
    assert!(matches!(
        snapshot.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Snapshot { .. }
            }
        })
    ));
    let coverage = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "coverage",
        "--universe",
        "trading-core-v1",
        "--target",
        "/tmp/project",
        "--json",
    ])
    .expect("data coverage parses");
    assert!(matches!(
        coverage.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::Coverage { json: true, .. }
            }
        })
    ));
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

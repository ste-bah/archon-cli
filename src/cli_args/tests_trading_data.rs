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
fn trading_data_coverage_parse() {
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
    match coverage.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Data {
                    action:
                        TradingCliDataAction::Coverage {
                            target,
                            universe,
                            json,
                            out,
                        },
                },
        }) => {
            assert_eq!(
                target.as_deref(),
                Some(std::path::Path::new("/tmp/project"))
            );
            assert_eq!(universe, "trading-core-v1");
            assert!(json);
            assert!(out.is_none());
        }
        other => panic!("expected trading data coverage, got {other:?}"),
    }

    let verify = Cli::try_parse_from([
        "archon",
        "trading",
        "data",
        "verify-coverage",
        "/tmp/project/.archon/trading-lab/data/coverage/latest.json",
        "/tmp/project/.archon/trading-lab/data/registry.json",
    ])
    .expect("data verify-coverage parses");
    assert!(matches!(
        verify.command,
        Some(Commands::Trading {
            action: TradingCliAction::Data {
                action: TradingCliDataAction::VerifyCoverage { .. }
            }
        })
    ));
}

#[path = "tests_trading_data/fetch_native.rs"]
mod fetch_native;

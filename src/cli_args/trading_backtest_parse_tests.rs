use clap::Parser;

use super::{Cli, Commands, TradingCliAction, TradingCliBacktestAction};

#[test]
fn trading_backtest_run_ahdm_native_parses() {
    let parsed = Cli::try_parse_from([
        "archon",
        "trading",
        "backtest",
        "run-ahdm-native",
        "--config",
        "backtest.json",
        "--target",
        "/tmp/project",
        "--run-id",
        "run-1",
        "--dataset-id",
        "openbb-SPY-1D-raw",
        "--version",
        "native-2024-01-02-2024-01-05-1D",
        "--quantity",
        "1",
        "--generated-at",
        "2026-06-16T13:45:00Z",
    ])
    .expect("AHDM native backtest parses");

    assert!(matches!(
        parsed.command,
        Some(Commands::Trading {
            action: TradingCliAction::Backtest {
                action: TradingCliBacktestAction::RunAhdmNative { .. }
            }
        })
    ));
}

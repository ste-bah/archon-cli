use super::{
    Cli, Commands, TradingCliAction, TradingCliCommand, TradingCliPersona, TradingCliVerb,
};
use crate::cli_args::{
    TradingCliOpenBbAction, TradingCliOpenBbMode, TradingCliPineAction, TradingCliToolsAction,
    TradingCliTvAction,
};
use clap::Parser;

#[test]
fn trading_dispatch_parses_fenced_backtest() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "dispatch",
        "backtest",
        "--action",
        "run-backtest",
        "--persona",
        "per05-execution-agent",
    ])
    .expect("trading dispatch must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Dispatch {
                    command,
                    action,
                    persona,
                    maker_checker_approved,
                    live_policy_enabled,
                },
        }) => {
            assert_eq!(command, TradingCliCommand::Backtest);
            assert_eq!(action, TradingCliVerb::RunBacktest);
            assert_eq!(persona, TradingCliPersona::Per05ExecutionAgent);
            assert!(!maker_checker_approved);
            assert!(!live_policy_enabled);
        }
        other => panic!("expected trading dispatch, got {other:?}"),
    }
}

#[test]
fn trading_kill_parses_operator_reason() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "kill",
        "--actor",
        "operator",
        "--reason",
        "manual halt",
        "--working-orders",
        "2",
    ])
    .expect("trading kill must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Kill {
                    actor,
                    reason,
                    working_orders,
                },
        }) => {
            assert_eq!(actor, "operator");
            assert_eq!(reason, "manual halt");
            assert_eq!(working_orders, 2);
        }
        other => panic!("expected trading kill, got {other:?}"),
    }
}

#[test]
fn trading_tools_status_parses_target() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "tools",
        "status",
        "--target",
        "/tmp/project",
    ])
    .expect("trading tools status must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Tools {
                    action: TradingCliToolsAction::Status { target },
                },
        }) => {
            assert_eq!(target.unwrap(), std::path::PathBuf::from("/tmp/project"));
        }
        other => panic!("expected trading tools status, got {other:?}"),
    }
}

#[test]
fn trading_tv_cli_parses_trailing_args() {
    let cli = Cli::try_parse_from([
        "archon", "trading", "tv", "cli", "--", "pine", "analyze", "--file", "x.pine",
    ])
    .expect("trading tv cli must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Tv {
                    action: TradingCliTvAction::Cli { args, .. },
                },
        }) => {
            assert_eq!(args, vec!["pine", "analyze", "--file", "x.pine"]);
        }
        other => panic!("expected trading tv cli, got {other:?}"),
    }
}

#[test]
fn trading_pine_generate_parses_paths() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "pine",
        "generate",
        "--strategy-id",
        "strat-1",
        "--spec",
        "spec.json",
        "--out",
        "pine-out",
    ])
    .expect("trading pine generate must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Pine {
                    action:
                        TradingCliPineAction::Generate {
                            strategy_id,
                            spec,
                            out,
                        },
                },
        }) => {
            assert_eq!(strategy_id, "strat-1");
            assert_eq!(spec, std::path::PathBuf::from("spec.json"));
            assert_eq!(out, std::path::PathBuf::from("pine-out"));
        }
        other => panic!("expected trading pine generate, got {other:?}"),
    }
}

#[test]
fn trading_openbb_status_parses() {
    let cli = Cli::try_parse_from(["archon", "trading", "openbb", "status"])
        .expect("trading openbb status must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Openbb {
                    action: TradingCliOpenBbAction::Status { target },
                },
        }) => {
            assert!(target.is_none());
        }
        other => panic!("expected trading openbb status, got {other:?}"),
    }
}

#[test]
fn trading_openbb_fetch_parses_governed_inputs() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "openbb",
        "fetch",
        "--request",
        "request.json",
        "--metadata",
        "metadata.json",
        "--quality",
        "quality.json",
        "--mode",
        "live-required",
        "--out",
        "dataset.json",
    ])
    .expect("trading openbb fetch must parse");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Openbb {
                    action:
                        TradingCliOpenBbAction::Fetch {
                            request,
                            metadata,
                            quality,
                            mode,
                            out,
                            ..
                        },
                },
        }) => {
            assert_eq!(request, std::path::PathBuf::from("request.json"));
            assert_eq!(metadata, std::path::PathBuf::from("metadata.json"));
            assert_eq!(quality, std::path::PathBuf::from("quality.json"));
            assert_eq!(mode, TradingCliOpenBbMode::LiveRequired);
            assert_eq!(out.unwrap(), std::path::PathBuf::from("dataset.json"));
        }
        other => panic!("expected trading openbb fetch, got {other:?}"),
    }
}

#[test]
fn trading_core_actions_parse() {
    assert!(matches!(
        Cli::try_parse_from([
            "archon",
            "trading",
            "spec",
            "validate",
            "--spec",
            "spec.json",
        ])
        .expect("spec validate parses")
        .command,
        Some(Commands::Trading {
            action: TradingCliAction::Spec { .. }
        })
    ));

    assert!(matches!(
        Cli::try_parse_from([
            "archon",
            "trading",
            "backtest",
            "run",
            "--config",
            "config.json",
            "--fills",
            "fills.json",
        ])
        .expect("backtest run parses")
        .command,
        Some(Commands::Trading {
            action: TradingCliAction::Backtest { .. }
        })
    ));

    assert!(matches!(
        Cli::try_parse_from([
            "archon",
            "trading",
            "paper",
            "sample",
            "--sample",
            "paper-sample.json",
        ])
        .expect("paper sample parses")
        .command,
        Some(Commands::Trading {
            action: TradingCliAction::Paper { .. }
        })
    ));
}

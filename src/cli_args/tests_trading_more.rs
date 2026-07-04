use super::{Cli, Commands, TradingCliAction};
use crate::cli_args::{
    TradingCliPaperAction, TradingCliPromoteAction, TradingCliPromotionStatus,
    TradingCliWorkflowAction,
};
use clap::Parser;

#[test]
fn trading_paper_tradingview_replay_submit_parses() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "paper",
        "tradingview-replay-submit",
        "--intent",
        "intent.json",
        "--adapter-pin",
        "tradesdontlie@abcdef1",
        "--write-tier-enabled",
        "--sandbox-certified",
        "--approval-id",
        "r1",
        "--maker",
        "alice",
        "--checker",
        "bob",
        "--rationale",
        "approved",
    ])
    .expect("TradingView replay submit parses");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Paper {
                    action:
                        TradingCliPaperAction::TradingviewReplaySubmit {
                            adapter_pin,
                            write_tier_enabled,
                            sandbox_certified,
                            ..
                        },
                },
        }) => {
            assert_eq!(adapter_pin, "tradesdontlie@abcdef1");
            assert!(write_tier_enabled);
            assert!(sandbox_certified);
        }
        other => panic!("expected trading paper replay submit, got {other:?}"),
    }
}

#[test]
fn trading_workflow_plan_parses() {
    let cli = Cli::try_parse_from([
        "archon",
        "trading",
        "workflow",
        "plan",
        "--idea",
        "BTC Elliott Wave strategy",
        "--repository",
        "/tmp/repo",
        "--tasks",
        "/tmp/tasks",
        "--kb",
        "trading-elliott-wave",
        "--tradingview-replay",
        "--out",
        "trading-workflow.yaml",
    ])
    .expect("trading workflow plan parses");

    match cli.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Workflow {
                    action:
                        TradingCliWorkflowAction::Plan {
                            idea,
                            kb,
                            tradingview_replay,
                            ..
                        },
                },
        }) => {
            assert_eq!(idea, "BTC Elliott Wave strategy");
            assert_eq!(kb, vec!["trading-elliott-wave"]);
            assert!(tradingview_replay);
        }
        other => panic!("expected trading workflow plan, got {other:?}"),
    }
}

#[test]
fn trading_promotion_and_live_actions_parse() {
    let promote = Cli::try_parse_from([
        "archon",
        "trading",
        "promote",
        "check",
        "--spec",
        "spec.json",
        "--target",
        "paper",
        "--evidence",
        "evidence.json",
    ])
    .expect("promote check parses");

    match promote.command {
        Some(Commands::Trading {
            action:
                TradingCliAction::Promote {
                    action:
                        TradingCliPromoteAction::Check {
                            target, evidence, ..
                        },
                },
        }) => {
            assert_eq!(target, TradingCliPromotionStatus::Paper);
            assert_eq!(evidence, std::path::PathBuf::from("evidence.json"));
        }
        other => panic!("expected trading promote check, got {other:?}"),
    }

    assert!(matches!(
        Cli::try_parse_from([
            "archon",
            "trading",
            "live",
            "pilot",
            "--strategy-id",
            "strat-1",
            "--account-equity",
            "10000",
            "--requested-capital",
            "500",
        ])
        .expect("live pilot parses")
        .command,
        Some(Commands::Trading {
            action: TradingCliAction::Live { .. }
        })
    ));
}

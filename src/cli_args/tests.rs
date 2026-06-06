use super::{
    AgentAction, AgentEvolveAction, Cli, CognitiveAction, CognitiveDaemonAction, Commands,
    GametheoryAction, ProvidersAction, TradingCliAction, TradingCliCommand, TradingCliPersona,
    TradingCliVerb, WorldAction, WorldGuardAction, WorldGuardPolicyAction,
};

#[cfg(test)]
mod metrics_port_parse_tests {
    //! AGS-OBS-903 Gate 4 coverage — pin `--metrics-port` clap parsing contract.
    //!
    //! Sherlock gate-3 flagged that without explicit parse tests the gate-walk
    //! on OBS-903 rested entirely on the smoke test, which skips CLI parsing.
    //! These pin the contract documented on the `metrics_port` field:
    //!   - absent flag         → `None`
    //!   - `--metrics-port 0`  → `Some(0)` (disables exporter at spawn site)
    //!   - `--metrics-port N`  → `Some(N)` for valid u16
    //!   - non-numeric value   → clap parse error
    //!   - value > u16::MAX    → clap parse error (overflow)
    use super::Cli;
    use clap::Parser;
    use clap::error::ErrorKind;
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }
    #[test]
    fn metrics_port_absent_is_none() {
        let cli = parse(&["archon"]).expect("no flags must parse");
        assert_eq!(cli.metrics_port, None);
    }
    #[test]
    fn metrics_port_zero_disables_but_parses() {
        let cli = parse(&["archon", "--metrics-port", "0"]).expect("zero must parse");
        assert_eq!(cli.metrics_port, Some(0));
    }
    #[test]
    fn metrics_port_valid_u16_parses() {
        let cli = parse(&["archon", "--metrics-port", "9090"]).expect("9090 must parse");
        assert_eq!(cli.metrics_port, Some(9090));
    }
    #[test]
    fn metrics_port_max_u16_parses() {
        let cli = parse(&["archon", "--metrics-port", "65535"]).expect("u16::MAX must parse");
        assert_eq!(cli.metrics_port, Some(65535));
    }
    #[test]
    fn metrics_port_non_numeric_rejected() {
        let err = parse(&["archon", "--metrics-port", "foo"]).expect_err("foo must fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }
    #[test]
    fn metrics_port_overflow_rejected() {
        let err = parse(&["archon", "--metrics-port", "70000"]).expect_err("70000 must fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }
    #[test]
    fn metrics_port_negative_rejected() {
        // clap sees a leading `-` as a flag prefix, so `-1` surfaces as
        // `UnknownArgument` rather than `ValueValidation`. Either way the
        // contract we care about is: a negative value never becomes a bound
        // port. We pin both kinds so a future clap behaviour change forces us
        // to reread this note rather than silently accepting `-1`.
        let err = parse(&["archon", "--metrics-port", "-1"]).expect_err("negative must fail");
        assert!(
            matches!(
                err.kind(),
                ErrorKind::UnknownArgument | ErrorKind::ValueValidation
            ),
            "unexpected clap error kind for -1: {:?}",
            err.kind()
        );
    }
}

#[cfg(test)]
mod remote_url_parse_tests {
    //! TASK-TUI-625-FOLLOWUP Gate 4 coverage — pin `--remote-url` clap parsing
    //! contract. These tests guarantee that the long flag spelling stays
    //! `--remote-url` (hyphen, not underscore) and does NOT collide with the
    //! existing `Commands::Remote { action }` subcommand.
    use super::Cli;
    use clap::Parser;

    #[test]
    fn remote_url_parses_from_long_flag() {
        let cli =
            Cli::try_parse_from(["archon", "--remote-url", "https://archon.example/sess/xyz"])
                .expect("--remote-url <URL> must parse");
        assert_eq!(
            cli.remote_url.as_deref(),
            Some("https://archon.example/sess/xyz")
        );
    }

    #[test]
    fn remote_url_absent_when_not_supplied() {
        let cli = Cli::try_parse_from(["archon"]).expect("archon with no flags must parse");
        assert!(cli.remote_url.is_none());
    }
}

#[cfg(test)]
mod cognitive_parse_tests {
    use super::{Cli, CognitiveAction, CognitiveDaemonAction, Commands};
    use clap::Parser;

    #[test]
    fn cognitive_status_json_parses() {
        let cli = Cli::try_parse_from(["archon", "cognitive", "status", "--json"])
            .expect("cognitive status must parse");

        match cli.command {
            Some(Commands::Cognitive {
                action: CognitiveAction::Status { json },
            }) => assert!(json),
            other => panic!("expected cognitive status, got {other:?}"),
        }
    }

    #[test]
    fn cognitive_inspect_session_parses() {
        let cli = Cli::try_parse_from([
            "archon",
            "cognitive",
            "inspect",
            "--session",
            "session-1",
            "--limit",
            "3",
        ])
        .expect("cognitive inspect session must parse");

        match cli.command {
            Some(Commands::Cognitive {
                action:
                    CognitiveAction::Inspect {
                        decision_id,
                        session,
                        limit,
                        ..
                    },
            }) => {
                assert!(decision_id.is_none());
                assert_eq!(session.as_deref(), Some("session-1"));
                assert_eq!(limit, 3);
            }
            other => panic!("expected cognitive inspect, got {other:?}"),
        }
    }

    #[test]
    fn cognitive_daemon_start_parses() {
        let cli = Cli::try_parse_from([
            "archon",
            "cognitive",
            "daemon",
            "start",
            "--interval-ms",
            "10000",
            "--json",
        ])
        .expect("cognitive daemon start must parse");
        match cli.command {
            Some(Commands::Cognitive {
                action:
                    CognitiveAction::Daemon {
                        action: CognitiveDaemonAction::Start { interval_ms, json },
                    },
            }) => {
                assert_eq!(interval_ms, Some(10000));
                assert!(json);
            }
            other => panic!("expected cognitive daemon start, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod world_guard_parse_tests {
    use super::{Cli, Commands, WorldAction, WorldGuardAction, WorldGuardPolicyAction};
    use clap::Parser;

    #[test]
    fn world_guard_status_parses() {
        let cli = Cli::try_parse_from(["archon", "world", "guard", "status"])
            .expect("world guard status must parse");

        match cli.command {
            Some(Commands::World {
                action:
                    WorldAction::Guard {
                        action: WorldGuardAction::Status,
                    },
            }) => {}
            other => panic!("expected world guard status, got {other:?}"),
        }
    }

    #[test]
    fn world_guard_policy_set_parses_modes() {
        let cli = Cli::try_parse_from([
            "archon",
            "world",
            "guard",
            "policy",
            "set",
            "--interactive-mode",
            "guarded",
            "--pipeline-mode",
            "strict",
        ])
        .expect("world guard policy set must parse");

        match cli.command {
            Some(Commands::World {
                action:
                    WorldAction::Guard {
                        action:
                            WorldGuardAction::Policy {
                                action:
                                    WorldGuardPolicyAction::Set {
                                        interactive_mode,
                                        pipeline_mode,
                                    },
                            },
                    },
            }) => {
                assert_eq!(interactive_mode.as_deref(), Some("guarded"));
                assert_eq!(pipeline_mode.as_deref(), Some("strict"));
            }
            other => panic!("expected world guard policy set, got {other:?}"),
        }
    }

    #[test]
    fn world_guard_approve_parses_reason() {
        let cli = Cli::try_parse_from([
            "archon",
            "world",
            "guard",
            "approve",
            "world-guard-action-1",
            "--reason",
            "operator accepts the risk",
        ])
        .expect("world guard approve must parse");

        match cli.command {
            Some(Commands::World {
                action:
                    WorldAction::Guard {
                        action: WorldGuardAction::Approve { action_id, reason },
                    },
            }) => {
                assert_eq!(action_id, "world-guard-action-1");
                assert_eq!(reason, "operator accepts the risk");
            }
            other => panic!("expected world guard approve, got {other:?}"),
        }
    }

    #[test]
    fn world_guard_skip_verification_parses_reason() {
        let cli = Cli::try_parse_from([
            "archon",
            "world",
            "guard",
            "skip-verification",
            "world-guard-req-1",
            "--reason",
            "test harness unavailable",
        ])
        .expect("world guard skip-verification must parse");

        match cli.command {
            Some(Commands::World {
                action:
                    WorldAction::Guard {
                        action:
                            WorldGuardAction::SkipVerification {
                                requirement_id,
                                reason,
                            },
                    },
            }) => {
                assert_eq!(requirement_id, "world-guard-req-1");
                assert_eq!(reason, "test harness unavailable");
            }
            other => panic!("expected world guard skip-verification, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod trading_parse_tests {
    use super::{
        Cli, Commands, TradingCliAction, TradingCliCommand, TradingCliPersona, TradingCliVerb,
    };
    use crate::cli_args::{
        TradingCliBacktestAction, TradingCliDataAction, TradingCliOpenBbAction,
        TradingCliOpenBbMode, TradingCliPaperAction, TradingCliPineAction, TradingCliPromoteAction,
        TradingCliPromotionStatus, TradingCliToolsAction, TradingCliTvAction,
        TradingCliWorkflowAction,
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
            "btc-1d",
            "--version",
            "v1",
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
                    action: TradingCliBacktestAction::RunOhlcv { .. }
                }
            })
        ));
    }

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
}

#[cfg(test)]
#[path = "provider_parse_tests.rs"]
mod provider_parse_tests;

#[cfg(test)]
mod gametheory_prd_parse_tests {
    use super::{Cli, Commands, GametheoryAction};
    use clap::Parser;

    #[test]
    fn gametheory_prd_shorthand_parses_situation_and_kb() {
        let cli = Cli::try_parse_from([
            "archon",
            "gametheory",
            "Assess this plugin marketplace",
            "--kb",
            "policy-pack",
        ])
        .expect("PRD shorthand gametheory command must parse");

        match cli.command {
            Some(Commands::Gametheory {
                situation,
                kb,
                action,
                ..
            }) => {
                assert_eq!(situation.as_deref(), Some("Assess this plugin marketplace"));
                assert_eq!(kb.as_deref(), Some("policy-pack"));
                assert!(action.is_none());
            }
            other => panic!("expected gametheory command, got {other:?}"),
        }
    }

    #[test]
    fn gametheory_prd_classify_only_shorthand_parses() {
        let cli = Cli::try_parse_from([
            "archon",
            "gametheory",
            "--classify-only",
            "Assess a bargaining situation",
        ])
        .expect("PRD classify-only shorthand must parse");

        match cli.command {
            Some(Commands::Gametheory {
                situation,
                classify_only,
                action,
                ..
            }) => {
                assert_eq!(situation.as_deref(), Some("Assess a bargaining situation"));
                assert!(classify_only);
                assert!(action.is_none());
            }
            other => panic!("expected gametheory command, got {other:?}"),
        }
    }

    #[test]
    fn gametheory_existing_run_subcommand_keeps_kb_flag() {
        let cli = Cli::try_parse_from([
            "archon",
            "gametheory",
            "run",
            "Assess a deterrence game",
            "--kb",
            "policy-pack",
        ])
        .expect("existing run subcommand must still parse");

        match cli.command {
            Some(Commands::Gametheory {
                action: Some(GametheoryAction::Run { situation, kb, .. }),
                ..
            }) => {
                assert_eq!(situation, "Assess a deterrence game");
                assert_eq!(kb.as_deref(), Some("policy-pack"));
            }
            other => panic!("expected gametheory run action, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod agent_evolve_parse_tests {
    use super::{AgentAction, AgentEvolveAction, Cli, Commands};
    use clap::Parser;

    #[test]
    fn agent_evolve_list_parses_filters() {
        let cli = Cli::try_parse_from([
            "archon", "agent", "evolve", "list", "--status", "pending", "--agent", "reviewer",
        ])
        .expect("agent evolve list must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::List { status, agent },
                    },
            }) => {
                assert_eq!(status.as_deref(), Some("pending"));
                assert_eq!(agent.as_deref(), Some("reviewer"));
            }
            other => panic!("expected agent evolve list, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_generate_parses_agent_filter() {
        let cli = Cli::try_parse_from([
            "archon", "agent", "evolve", "generate", "--agent", "reviewer",
        ])
        .expect("agent evolve generate must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Generate { agent },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
            }
            other => panic!("expected agent evolve generate, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_inspect_parses_proposal_and_json() {
        let cli = Cli::try_parse_from([
            "archon",
            "agent",
            "evolve",
            "inspect",
            "agent-evo-prop-1",
            "--json",
        ])
        .expect("agent evolve inspect must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Inspect { proposal_id, json },
                    },
            }) => {
                assert_eq!(proposal_id, "agent-evo-prop-1");
                assert!(json);
            }
            other => panic!("expected agent evolve inspect, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_history_and_status_parse() {
        let history = Cli::try_parse_from([
            "archon", "agent", "evolve", "history", "--agent", "reviewer", "--json",
        ])
        .expect("agent evolve history must parse");
        let status = Cli::try_parse_from([
            "archon", "agent", "evolve", "status", "--agent", "reviewer", "--json",
        ])
        .expect("agent evolve status must parse");

        match history.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::History { agent, json },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
                assert!(json);
            }
            other => panic!("expected agent evolve history, got {other:?}"),
        }
        match status.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Status { agent, json },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
                assert!(json);
            }
            other => panic!("expected agent evolve status, got {other:?}"),
        }

        let digest = Cli::try_parse_from([
            "archon",
            "agent",
            "evolve",
            "digest",
            "--agent",
            "reviewer",
            "--persist",
        ])
        .expect("agent evolve digest must parse");
        match digest.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action:
                            AgentEvolveAction::Digest {
                                agent,
                                persist,
                                json,
                            },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
                assert!(persist);
                assert!(!json);
            }
            other => panic!("expected agent evolve digest, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_review_state_commands_parse() {
        let approve =
            Cli::try_parse_from(["archon", "agent", "evolve", "approve", "agent-evo-prop-1"])
                .expect("agent evolve approve must parse");
        let reject =
            Cli::try_parse_from(["archon", "agent", "evolve", "reject", "agent-evo-prop-1"])
                .expect("agent evolve reject must parse");

        match approve.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Approve { proposal_id },
                    },
            }) => assert_eq!(proposal_id, "agent-evo-prop-1"),
            other => panic!("expected agent evolve approve, got {other:?}"),
        }
        match reject.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Reject { proposal_id },
                    },
            }) => assert_eq!(proposal_id, "agent-evo-prop-1"),
            other => panic!("expected agent evolve reject, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_apply_parses_activation_flag() {
        let cli = Cli::try_parse_from([
            "archon",
            "agent",
            "evolve",
            "apply",
            "agent-evo-prop-1",
            "--activate",
        ])
        .expect("agent evolve apply must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action:
                            AgentEvolveAction::Apply {
                                proposal_id,
                                activate,
                            },
                    },
            }) => {
                assert_eq!(proposal_id, "agent-evo-prop-1");
                assert!(activate);
            }
            other => panic!("expected agent evolve apply, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_active_parses_json_flag() {
        let cli = Cli::try_parse_from([
            "archon", "agent", "evolve", "active", "--agent", "reviewer", "--json",
        ])
        .expect("agent evolve active must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Active { agent, json },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
                assert!(json);
            }
            other => panic!("expected agent evolve active, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_rollback_parses_agent_and_activation() {
        let cli = Cli::try_parse_from([
            "archon",
            "agent",
            "evolve",
            "rollback",
            "--agent",
            "reviewer",
            "agent-profile-1",
            "--activate",
        ])
        .expect("agent evolve rollback must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action:
                            AgentEvolveAction::Rollback {
                                agent,
                                version_id,
                                activate,
                            },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
                assert_eq!(version_id, "agent-profile-1");
                assert!(activate);
            }
            other => panic!("expected agent evolve rollback, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_permissions_parses_proposal_id() {
        let cli = Cli::try_parse_from([
            "archon",
            "agent",
            "evolve",
            "permissions",
            "agent-evo-prop-1",
            "--json",
        ])
        .expect("agent evolve permissions must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Permissions { proposal_id, json },
                    },
            }) => {
                assert_eq!(proposal_id, "agent-evo-prop-1");
                assert!(json);
            }
            other => panic!("expected agent evolve permissions, got {other:?}"),
        }
    }

    #[test]
    fn agent_evolve_report_parses_agent_and_json() {
        let cli = Cli::try_parse_from([
            "archon", "agent", "evolve", "report", "--agent", "reviewer", "--json",
        ])
        .expect("agent evolve report must parse");

        match cli.command {
            Some(Commands::Agent {
                action:
                    AgentAction::Evolve {
                        action: AgentEvolveAction::Report { agent, json },
                    },
            }) => {
                assert_eq!(agent, "reviewer");
                assert!(json);
            }
            other => panic!("expected agent evolve report, got {other:?}"),
        }
    }
}

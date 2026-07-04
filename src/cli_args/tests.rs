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

#[path = "provider_parse_tests.rs"]
mod provider_parse_tests;
#[cfg(test)]
#[path = "tests_trading_core.rs"]
mod tests_trading_core;
#[cfg(test)]
#[path = "tests_trading_data.rs"]
mod tests_trading_data;
#[cfg(test)]
#[path = "tests_trading_more.rs"]
mod tests_trading_more;

#[cfg(test)]
#[path = "tests_agent_evolve.rs"]
mod tests_agent_evolve;
#[cfg(test)]
#[path = "tests_gametheory_prd.rs"]
mod tests_gametheory_prd;

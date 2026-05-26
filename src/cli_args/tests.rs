use super::{
    AgentAction, AgentEvolveAction, Cli, CognitiveAction, CognitiveDaemonAction, Commands,
    GametheoryAction, ProvidersAction, WorldAction, WorldGuardAction, WorldGuardPolicyAction,
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

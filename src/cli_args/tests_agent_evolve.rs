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
    let approve = Cli::try_parse_from(["archon", "agent", "evolve", "approve", "agent-evo-prop-1"])
        .expect("agent evolve approve must parse");
    let reject = Cli::try_parse_from(["archon", "agent", "evolve", "reject", "agent-evo-prop-1"])
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

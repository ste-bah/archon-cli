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

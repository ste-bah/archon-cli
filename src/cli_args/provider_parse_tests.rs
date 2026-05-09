use super::{Cli, Commands, ProvidersAction};
use clap::Parser;

#[test]
fn providers_report_parses_provider_and_json() {
    let cli = Cli::try_parse_from([
        "archon",
        "providers",
        "report",
        "--provider",
        "anthropic",
        "--json",
    ])
    .expect("providers report must parse");

    match cli.command {
        Some(Commands::Providers {
            action: Some(ProvidersAction::Report { provider, json }),
        }) => {
            assert_eq!(provider.as_deref(), Some("anthropic"));
            assert!(json);
        }
        other => panic!("expected providers report, got {other:?}"),
    }
}

#[test]
fn providers_status_parses_provider_and_json() {
    let cli = Cli::try_parse_from([
        "archon",
        "providers",
        "status",
        "--provider",
        "openai-codex",
        "--json",
    ])
    .expect("providers status must parse");

    match cli.command {
        Some(Commands::Providers {
            action: Some(ProvidersAction::Status { provider, json }),
        }) => {
            assert_eq!(provider.as_deref(), Some("openai-codex"));
            assert!(json);
        }
        other => panic!("expected providers status, got {other:?}"),
    }
}

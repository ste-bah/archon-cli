use super::{Cli, Commands, SandboxAction};
use clap::Parser;

#[test]
fn sandbox_sessions_parses_filters_and_json() {
    let cli = Cli::try_parse_from([
        "archon",
        "sandbox",
        "sessions",
        "--status",
        "configured",
        "--agent",
        "reviewer",
        "--limit",
        "5",
        "--json",
    ])
    .expect("sandbox sessions must parse");

    match cli.command {
        Some(Commands::Sandbox {
            action:
                Some(SandboxAction::Sessions {
                    status,
                    agent,
                    limit,
                    json,
                }),
        }) => {
            assert_eq!(status.as_deref(), Some("configured"));
            assert_eq!(agent.as_deref(), Some("reviewer"));
            assert_eq!(limit, 5);
            assert!(json);
        }
        other => panic!("expected sandbox sessions, got {other:?}"),
    }
}

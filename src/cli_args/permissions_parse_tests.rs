use super::{Cli, Commands, PermissionsAction};
use clap::Parser;

#[test]
fn permissions_denials_parses_agent_limit_and_json() {
    let cli = Cli::try_parse_from([
        "archon",
        "permissions",
        "denials",
        "--agent",
        "reviewer",
        "--limit",
        "5",
        "--json",
    ])
    .expect("permissions denials must parse");

    match cli.command {
        Some(Commands::Permissions {
            action: PermissionsAction::Denials { agent, limit, json },
        }) => {
            assert_eq!(agent.as_deref(), Some("reviewer"));
            assert_eq!(limit, 5);
            assert!(json);
        }
        other => panic!("expected permissions denials, got {other:?}"),
    }
}

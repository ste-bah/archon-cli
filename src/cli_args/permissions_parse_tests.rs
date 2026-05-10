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

#[test]
fn permissions_diff_parses_versions() {
    let cli = Cli::try_parse_from([
        "archon",
        "permissions",
        "diff",
        "--agent",
        "reviewer",
        "--from",
        "profile-1",
        "--to",
        "profile-2",
        "--json",
    ])
    .expect("permissions diff must parse");

    match cli.command {
        Some(Commands::Permissions {
            action:
                PermissionsAction::Diff {
                    agent,
                    from_version_id,
                    to_version_id,
                    json,
                },
        }) => {
            assert_eq!(agent, "reviewer");
            assert_eq!(from_version_id, "profile-1");
            assert_eq!(to_version_id, "profile-2");
            assert!(json);
        }
        other => panic!("expected permissions diff, got {other:?}"),
    }
}

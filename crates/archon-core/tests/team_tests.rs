//! Tests for TASK-CLI-312: Agent Teams, wired in #184 M5.
//!
//! The file-mailbox backend these used to cover is gone: every agent shares one
//! process, so member delivery is the message router, and a `FileBasedBackend`
//! nothing instantiated was a second delivery path that only tests exercised.
//!
//! What is covered instead is the thing that was broken — `TeamManager` read
//! `<project>/teams/` while the tools wrote `<project>/.archon/teams/`, so
//! `archon team list` could never see a team `TeamCreate` had made.

use archon_core::team::TeamManager;
use archon_core::team::message::{MessageType, TeamMessage};
use archon_core::team::roster;
use archon_core::team::team_config::{MemberConfig, TeamConfig};

fn team(id: &str, roles: &[&str]) -> TeamConfig {
    TeamConfig {
        id: id.to_string(),
        name: format!("{id} team"),
        members: roles.iter().map(|r| MemberConfig::declared(*r)).collect(),
    }
}

// ---------------------------------------------------------------------------
// TeamManager reads what the tools write
// ---------------------------------------------------------------------------

#[test]
fn the_manager_reads_the_directory_the_roster_writes() {
    let dir = tempfile::tempdir().unwrap();
    roster::save(dir.path(), &team("t1", &["coder"])).unwrap();

    let manager = TeamManager::new(dir.path().to_path_buf());
    assert_eq!(manager.list_teams().unwrap(), vec!["t1".to_string()]);
    assert_eq!(manager.load_team("t1").unwrap().members[0].role, "coder");
    assert!(manager.teams_dir().ends_with("teams"));
}

#[test]
fn listing_a_project_with_no_teams_is_empty_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let manager = TeamManager::new(dir.path().to_path_buf());
    assert!(manager.list_teams().unwrap().is_empty());
}

#[test]
fn loading_a_team_that_is_not_there_reports_which_one() {
    let dir = tempfile::tempdir().unwrap();
    let manager = TeamManager::new(dir.path().to_path_buf());
    let error = manager.load_team("ghost").unwrap_err().to_string();
    assert!(error.contains("ghost"), "{error}");
}

// ---------------------------------------------------------------------------
// TeamConfig serialization
// ---------------------------------------------------------------------------

#[test]
fn team_config_round_trips_json() {
    let mut config = team("abc", &["agent1"]);
    config.members[0].model = Some("claude-sonnet-4-6".to_string());
    config.members[0].agent_id = Some("subagent-7".to_string());

    let restored: TeamConfig =
        serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();

    assert_eq!(restored.id, "abc");
    assert_eq!(restored.members.len(), 1);
    assert_eq!(restored.members[0].agent_id.as_deref(), Some("subagent-7"));
    assert!(restored.members[0].is_filled());
}

/// A vacant seat serializes without the field rather than as an explicit null,
/// so a roster nobody is on reads the same as it did before seats existed.
#[test]
fn a_vacant_seat_omits_the_agent_id() {
    let json = serde_json::to_string(&team("t", &["coder"])).unwrap();
    assert!(!json.contains("agent_id"), "{json}");
}

// ---------------------------------------------------------------------------
// Member envelope
// ---------------------------------------------------------------------------

#[test]
fn a_member_message_carries_its_sender() {
    let rendered = TeamMessage::now("coder", "reviewer", "ready", MessageType::Chat).render();
    assert!(rendered.contains("from=\"coder\""), "{rendered}");
    assert!(rendered.contains("ready"), "{rendered}");
}

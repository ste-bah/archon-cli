//! Roster tests (#184 M5).
//!
//! The active team is process-global, so these serialize on `TEAM_TEST_LOCK`
//! rather than running concurrently against one another. Each still uses its
//! own temp directory, so a lock that is poisoned by a failing test cannot make
//! the rest read each other's rosters.

use super::test_lock::lock;
use super::*;

/// A team on disk with the given declared roles, activated for this test.
fn team_with(roles: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let config = TeamConfig {
        id: "t1".into(),
        name: "test team".into(),
        members: roles.iter().map(|r| MemberConfig::declared(*r)).collect(),
    };
    save(dir.path(), &config).expect("save");
    activate(dir.path().to_path_buf(), "t1".into());
    dir
}

#[test]
fn a_spawn_takes_a_declared_seat_of_its_role() {
    let _guard = lock();
    let _dir = team_with(&["coder", "reviewer"]);

    assert_eq!(join("agent-1", "reviewer").as_deref(), Some("t1"));

    let seats = members();
    assert_eq!(seats.len(), 2, "no seat should be added");
    let reviewer = seats.iter().find(|m| m.role == "reviewer").unwrap();
    assert_eq!(reviewer.agent_id.as_deref(), Some("agent-1"));
    assert!(
        !seats
            .iter()
            .find(|m| m.role == "coder")
            .unwrap()
            .is_filled()
    );

    deactivate();
}

/// Two agents of the same role is the ordinary fan-out. A roster keyed by role
/// could not represent it, which is why seats are a list.
#[test]
fn two_spawns_of_one_role_take_two_seats() {
    let _guard = lock();
    let _dir = team_with(&["coder"]);

    join("agent-1", "coder");
    join("agent-2", "coder");

    let seated: Vec<String> = seated_agent_ids();
    assert_eq!(seated, vec!["agent-1".to_string(), "agent-2".to_string()]);

    deactivate();
}

#[test]
fn an_undeclared_role_joins_as_a_new_seat() {
    let _guard = lock();
    let _dir = team_with(&["coder"]);

    join("agent-9", "documenter");

    let seats = members();
    assert_eq!(seats.len(), 2);
    assert_eq!(
        seats
            .iter()
            .find(|m| m.role == "documenter")
            .unwrap()
            .agent_id
            .as_deref(),
        Some("agent-9")
    );

    deactivate();
}

/// A resume re-registers the same agent under the same id. Seating it twice
/// would show one running agent as two members.
#[test]
fn joining_twice_with_one_id_is_idempotent() {
    let _guard = lock();
    let _dir = team_with(&["coder"]);

    join("agent-1", "coder");
    join("agent-1", "coder");

    assert_eq!(seated_agent_ids().len(), 1);
    assert_eq!(members().len(), 1);

    deactivate();
}

/// The team still wants a reviewer when no reviewer is running, so a declared
/// seat survives its occupant.
#[test]
fn leaving_vacates_a_declared_seat_without_removing_it() {
    let _guard = lock();
    let _dir = team_with(&["coder"]);

    join("agent-1", "coder");
    leave("agent-1");

    let seats = members();
    assert_eq!(seats.len(), 1);
    assert!(!seats[0].is_filled());

    deactivate();
}

/// Nothing declared the appended seat, so nothing keeps it once it empties.
#[test]
fn leaving_removes_a_seat_that_a_join_appended() {
    let _guard = lock();
    let _dir = team_with(&["coder"]);

    join("agent-9", "documenter");
    leave("agent-9");

    let roles: Vec<String> = members().into_iter().map(|m| m.role).collect();
    assert_eq!(roles, vec!["coder".to_string()]);

    deactivate();
}

/// Spawns happen constantly; teams are rare. Joining without a team is the
/// ordinary case, not a failure, and must not write anything.
#[test]
fn joining_without_an_active_team_does_nothing() {
    let _guard = lock();
    deactivate();

    assert_eq!(join("agent-1", "coder"), None);
    assert!(members().is_empty());
    leave("agent-1");
}

#[test]
fn a_team_json_written_before_seats_existed_still_loads() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let legacy = r#"{
        "id": "old",
        "name": "legacy",
        "members": [
            { "role": "coder", "system_prompt": "write code", "model": null, "tools": ["Read"] }
        ]
    }"#;
    let team = team_dir(dir.path(), "old");
    std::fs::create_dir_all(&team).unwrap();
    std::fs::write(team.join("team.json"), legacy).unwrap();

    let config = load(dir.path(), "old").expect("legacy team.json should load");
    assert_eq!(config.members.len(), 1);
    assert!(!config.members[0].is_filled());
    assert!(
        config.members[0].declared,
        "a member written before the field existed was declared by TeamCreate; \
         reading it as undeclared would delete it on the first departure"
    );
}

/// The second agent in a declared role is a seat nobody asked for, so it goes
/// when it finishes — otherwise a fan-out of ten leaves nine empty seats behind.
#[test]
fn the_extra_seat_of_a_declared_role_does_not_outlive_its_agent() {
    let _guard = lock();
    let _dir = team_with(&["coder"]);

    join("agent-1", "coder");
    join("agent-2", "coder");
    assert_eq!(members().len(), 2);

    leave("agent-2");

    let seats = members();
    assert_eq!(seats.len(), 1);
    assert_eq!(seats[0].agent_id.as_deref(), Some("agent-1"));

    deactivate();
}

#[test]
fn listing_finds_every_team_directory() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("temp dir");
    for id in ["beta", "alpha"] {
        save(
            dir.path(),
            &TeamConfig {
                id: id.into(),
                name: id.into(),
                members: Vec::new(),
            },
        )
        .unwrap();
    }

    assert_eq!(
        list(dir.path()),
        vec!["alpha".to_string(), "beta".to_string()]
    );
}

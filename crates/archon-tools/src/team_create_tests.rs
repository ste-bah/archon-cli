//! `TeamCreate` tests (#184 M5).
//!
//! The active team is process-global, so these share a lock with the other team
//! suites in this crate and deactivate on the way out.
//!
//! Holding that guard across an `.await` is the point — the whole test has to be
//! exclusive, not just its synchronous half — and each test runs on its own
//! current-thread runtime, so there is nothing for the guard to deadlock
//! against.
#![allow(clippy::await_holding_lock)]

use crate::tool::ToolContext;

use super::*;

fn ctx() -> ToolContext {
    ToolContext::default()
}

fn create_input(roles: &[&str]) -> serde_json::Value {
    json!({
        "name": "test team",
        "members": roles.iter().map(|r| json!({
            "role": r,
            "system_prompt": format!("you are the {r}"),
        })).collect::<Vec<_>>(),
    })
}

#[tokio::test]
async fn creating_a_team_writes_a_roster_and_makes_it_active() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");

    let result = TeamCreateTool::new(dir.path().to_path_buf())
        .execute(create_input(&["coder", "reviewer"]), &ctx())
        .await;
    assert!(!result.is_error, "{}", result.content);

    let active = team_roster::active().expect("the team should be active");
    assert_eq!(active.project_dir, dir.path());

    let config = team_roster::load(dir.path(), &active.team_id).expect("roster on disk");
    let roles: Vec<String> = config.members.iter().map(|m| m.role.clone()).collect();
    assert_eq!(roles, vec!["coder".to_string(), "reviewer".to_string()]);
    assert!(
        config.members.iter().all(|m| !m.is_filled()),
        "no agent has spawned yet"
    );

    team_roster::deactivate();
}

/// The inbox files were written and never read. Delivery is the in-process
/// router, so creating them again would be re-creating the stub.
#[tokio::test]
async fn creating_a_team_writes_no_mailbox_files() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");

    TeamCreateTool::new(dir.path().to_path_buf())
        .execute(create_input(&["coder"]), &ctx())
        .await;
    let team_id = team_roster::active().expect("active").team_id;

    let entries: Vec<String> = std::fs::read_dir(team_roster::team_dir(dir.path(), &team_id))
        .expect("team dir")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    assert_eq!(entries, vec!["team.json".to_string()], "{entries:?}");

    team_roster::deactivate();
}

/// Switching teams while agents are seated would leave them on a roster nothing
/// reads, and route their departures into the new team's file.
#[tokio::test]
async fn a_second_team_is_refused_while_members_are_running() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let tool = TeamCreateTool::new(dir.path().to_path_buf());

    tool.execute(create_input(&["coder"]), &ctx()).await;
    team_roster::join("agent-1", "coder");

    let second = tool.execute(create_input(&["writer"]), &ctx()).await;
    assert!(second.is_error);
    assert!(second.content.contains("TeamDelete"), "{}", second.content);

    team_roster::deactivate();
}

/// With nobody seated there is nothing to strand, so replacing the team is fine.
#[tokio::test]
async fn a_second_team_is_allowed_once_the_first_is_empty() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let tool = TeamCreateTool::new(dir.path().to_path_buf());

    tool.execute(create_input(&["coder"]), &ctx()).await;
    let first = team_roster::active().expect("active").team_id;

    let second = tool.execute(create_input(&["writer"]), &ctx()).await;
    assert!(!second.is_error, "{}", second.content);
    assert_ne!(team_roster::active().expect("active").team_id, first);

    team_roster::deactivate();
}

#[tokio::test]
async fn a_member_without_a_role_is_refused() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");

    let result = TeamCreateTool::new(dir.path().to_path_buf())
        .execute(
            json!({ "name": "t", "members": [{ "role": "  ", "system_prompt": "x" }] }),
            &ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("empty"), "{}", result.content);

    team_roster::deactivate();
}

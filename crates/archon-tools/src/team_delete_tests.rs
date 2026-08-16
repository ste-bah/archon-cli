//! `TeamDelete` tests (#184 M5).
//!
//! The handshake needs an executor to signal through, so these install a double
//! that records who was asked to stop and, optionally, vacates their seat — the
//! same thing the real terminal hook does.
//!
//! The team lock is held across `.await` deliberately: the active team is
//! process-global, so a test has to own it for its whole body, and each test
//! runs on its own current-thread runtime.
#![allow(clippy::await_holding_lock)]

use std::sync::{Arc, Mutex};

use crate::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use crate::subagent_request::SubagentRequest;
use crate::tool::ToolContext;

use super::*;

/// Records shutdown requests. `obedient` decides whether the agents actually
/// leave, which is what separates a clean shutdown from a straggler.
struct StopRecorder {
    asked: Mutex<Vec<String>>,
    obedient: bool,
}

#[async_trait]
impl SubagentExecutor for StopRecorder {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<String, ExecutorError> {
        Err(ExecutorError::Internal("not used in this test".into()))
    }

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {}

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, _request: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }

    async fn request_shutdown(&self, subagent_id: &str) -> bool {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(subagent_id.to_string());
        }
        if self.obedient {
            // What the real terminal hook does, minus the agent.
            team_roster::leave(subagent_id);
        }
        true
    }
}

fn install(obedient: bool) -> Arc<StopRecorder> {
    let recorder = Arc::new(StopRecorder {
        asked: Mutex::new(Vec::new()),
        obedient,
    });
    install_subagent_executor(recorder.clone());
    recorder
}

/// A team on disk with `roles` declared, activated, with `seated` agents on it.
fn team(dir: &std::path::Path, roles: &[&str], seated: &[(&str, &str)]) -> String {
    let config = crate::team_config::TeamConfig {
        id: "t1".into(),
        name: "test team".into(),
        members: roles
            .iter()
            .map(|r| crate::team_config::MemberConfig::declared(*r))
            .collect(),
    };
    team_roster::save(dir, &config).expect("save");
    team_roster::activate(dir.to_path_buf(), "t1".into());
    for (agent_id, role) in seated {
        team_roster::join(agent_id, role);
    }
    "t1".to_string()
}

fn ctx() -> ToolContext {
    ToolContext::default()
}

#[tokio::test]
async fn deleting_asks_every_member_to_stop_then_removes_the_team() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let recorder = install(true);
    let id = team(
        dir.path(),
        &["coder", "reviewer"],
        &[("agent-1", "coder"), ("agent-2", "reviewer")],
    );

    let result = TeamDeleteTool::new(dir.path().to_path_buf())
        .execute(json!({ "team_id": id }), &ctx())
        .await;

    assert!(!result.is_error, "{}", result.content);
    let asked = recorder.asked.lock().unwrap().clone();
    assert_eq!(asked, vec!["agent-1".to_string(), "agent-2".to_string()]);
    assert!(!team_roster::team_dir(dir.path(), "t1").exists());
    assert!(team_roster::active().is_none());
}

/// The whole point of the handshake: a member that will not stop keeps its
/// team, because a roster deleted out from under a running agent is worse than
/// a team that is still there.
#[tokio::test]
async fn a_member_that_does_not_stop_keeps_the_team_alive() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");
    install(false);
    let id = team(dir.path(), &["coder"], &[("agent-1", "coder")]);

    // A short grace so the test reaches the deadline instead of sitting there
    // for the production minute.
    let result = TeamDeleteTool::with_grace(dir.path().to_path_buf(), Duration::from_millis(300))
        .execute(json!({ "team_id": id }), &ctx())
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("agent-1"), "{}", result.content);
    assert!(team_roster::team_dir(dir.path(), "t1").exists());
    assert!(
        team_roster::active().is_some(),
        "the team must stay active while a member is running"
    );

    team_roster::deactivate();
}

/// An empty roster has nobody to signal, so it is a directory removal.
#[tokio::test]
async fn deleting_an_empty_team_needs_no_handshake() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let recorder = install(true);
    let id = team(dir.path(), &["coder"], &[]);

    let result = TeamDeleteTool::new(dir.path().to_path_buf())
        .execute(json!({ "team_id": id }), &ctx())
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(recorder.asked.lock().unwrap().is_empty());
    assert!(!team_roster::team_dir(dir.path(), "t1").exists());
}

#[tokio::test]
async fn deleting_a_team_that_is_not_there_says_so() {
    let _guard = crate::team_roster::test_lock::lock();
    let dir = tempfile::tempdir().expect("temp dir");
    team_roster::deactivate();

    let result = TeamDeleteTool::new(dir.path().to_path_buf())
        .execute(json!({ "team_id": "nope" }), &ctx())
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("not found"), "{}", result.content);
}

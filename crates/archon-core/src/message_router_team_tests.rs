//! Team attribution on routed messages (#184 M5).
//!
//! Split from `message_router_tests.rs` to keep both under the FileSizeGuard
//! threshold. These are the only tests here that touch the process-global active
//! team, so they serialize on their own lock and deactivate on the way out.
//!
//! The guard is held across `.await` on purpose: the team has to stay this
//! test's for its whole body, and each test runs on its own current-thread
//! runtime.
#![allow(clippy::await_holding_lock)]

use std::sync::{Mutex as StdMutex, MutexGuard};

use archon_tools::team_config::{MemberConfig, TeamConfig};
use archon_tools::team_roster;

use super::tests::{RecordingHost, lead_ctx, req, sample_request};
use super::*;

static TEAM_LOCK: StdMutex<()> = StdMutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEAM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A running agent seated on a live team, plus the manager and the agent's id.
fn team_with_seated_agent(
    dir: &std::path::Path,
    role: &str,
) -> (Arc<Mutex<SubagentManager>>, String) {
    team_roster::save(
        dir,
        &TeamConfig {
            id: "t1".into(),
            name: "test team".into(),
            members: vec![
                MemberConfig::declared("coder"),
                MemberConfig::declared("reviewer"),
            ],
        },
    )
    .expect("save roster");
    team_roster::activate(dir.to_path_buf(), "t1".into());

    let mut mgr = SubagentManager::new(4);
    let id = mgr.register(sample_request()).expect("register");
    mgr.register_name(role.to_string(), id.clone());
    team_roster::join(&id, role);

    (Arc::new(Mutex::new(mgr)), id)
}

/// The gap this closes: a member that cannot tell who wrote to it cannot reply.
#[tokio::test]
async fn a_message_between_members_names_its_sender() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let (manager, target_id) = team_with_seated_agent(dir.path(), "reviewer");

    // The sender is the other seat, so it has to be on the roster too.
    let sender_id = {
        let mut mgr = manager.lock().await;
        let id = mgr.register(sample_request()).expect("register");
        mgr.register_name("coder".into(), id.clone());
        id
    };
    team_roster::join(&sender_id, "coder");

    let ctx = RouterContext::new(
        Arc::clone(&manager),
        SenderIdentity::Subagent {
            id: sender_id,
            lead_id: None,
        },
    );
    let host = RecordingHost::without_resume();
    let out = route(&ctx, &host, &req("reviewer", "text")).await;
    assert!(!out.is_error, "{}", out.content);

    let queued = manager.lock().await.drain_pending_messages(&target_id);
    let delivered = queued.first().expect("one queued message").clone();
    assert!(delivered.contains("from=\"coder\""), "{delivered}");
    assert!(delivered.contains("to=\"reviewer\""), "{delivered}");
    assert!(delivered.contains("hello"), "{delivered}");

    team_roster::deactivate();
}

/// Attribution comes from the sender's seat, never from the message. A sender
/// that holds no seat is not team traffic, so nothing is asserted on its behalf.
#[tokio::test]
async fn an_unseated_sender_gets_no_attribution() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let (manager, target_id) = team_with_seated_agent(dir.path(), "reviewer");

    let ctx = RouterContext::new(
        Arc::clone(&manager),
        SenderIdentity::Subagent {
            id: "stranger".into(),
            lead_id: None,
        },
    );
    let host = RecordingHost::without_resume();
    route(&ctx, &host, &req("reviewer", "text")).await;

    let queued = manager.lock().await.drain_pending_messages(&target_id);
    assert_eq!(queued.first().map(String::as_str), Some("hello"));

    team_roster::deactivate();
}

/// Most sessions have no team. Their messages must be delivered exactly as
/// before, with nothing wrapped around them.
#[tokio::test]
async fn without_a_team_the_message_is_delivered_verbatim() {
    let _guard = lock();
    team_roster::deactivate();

    let mut mgr = SubagentManager::new(4);
    let id = mgr.register(sample_request()).expect("register");
    mgr.register_name("reviewer".into(), id.clone());
    let manager = Arc::new(Mutex::new(mgr));

    let host = RecordingHost::without_resume();
    route(
        &lead_ctx(Arc::clone(&manager)),
        &host,
        &req("reviewer", "text"),
    )
    .await;

    let queued = manager.lock().await.drain_pending_messages(&id);
    assert_eq!(queued.first().map(String::as_str), Some("hello"));
}

/// The lead holds no seat, so its role is the reserved address rather than a
/// lookup that would fail and drop the attribution entirely.
#[tokio::test]
async fn the_lead_is_attributed_as_the_lead() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("temp dir");
    let (manager, target_id) = team_with_seated_agent(dir.path(), "reviewer");

    let host = RecordingHost::without_resume();
    route(
        &lead_ctx(Arc::clone(&manager)),
        &host,
        &req("reviewer", "text"),
    )
    .await;

    let queued = manager.lock().await.drain_pending_messages(&target_id);
    let delivered = queued.first().expect("one queued message").clone();
    assert!(delivered.contains("from=\"lead\""), "{delivered}");

    team_roster::deactivate();
}

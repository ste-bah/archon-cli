//! Module-local unit tests (smoke — full contract tests live in
//! `crates/archon-tools/tests/task_ags_101.rs`).
//!
//! Split out of `background_agents.rs` to keep that file under the
//! FileSizeGuard limit; nothing else about them moved.

use super::*;

fn dummy_handle(status: AgentStatus) -> BackgroundAgentHandle {
    let agent_id = Uuid::new_v4();
    BackgroundAgentHandle {
        agent_id,
        subagent_id: agent_id.to_string(),
        join_handle: None,
        cancel_token: CancellationToken::new(),
        spawned_at: SystemTime::now(),
        status: Arc::new(Mutex::new(status)),
        result_slot: new_result_slot(),
    }
}

#[test]
fn register_get_and_duplicate() {
    let r = BackgroundAgentRegistry::new();
    let h = dummy_handle(AgentStatus::Running);
    let id = h.agent_id;
    r.register(h).unwrap();
    assert_eq!(r.get(&id), Some(AgentStatus::Running));

    let mut dup = dummy_handle(AgentStatus::Running);
    dup.agent_id = id;
    dup.subagent_id = id.to_string();
    match r.register(dup) {
        Err(RegistryError::Duplicate(got)) => assert_eq!(got, id),
        other => panic!("expected Duplicate, got {other:?}"),
    }
}

/// A subagent id that is not UUID-shaped is registered and readable all the
/// same — the case that made liveness a fan-out in the first place.
#[test]
fn a_non_uuid_subagent_id_is_a_first_class_key() {
    let r = BackgroundAgentRegistry::new();
    let mut h = dummy_handle(AgentStatus::Running);
    h.subagent_id = "session-7-2-reviewer".to_string();
    r.register(h).expect("register");

    assert_eq!(
        r.status_of("session-7-2-reviewer"),
        Some(AgentStatus::Running)
    );
    assert!(
        r.iter_running_ids()
            .contains(&"session-7-2-reviewer".to_string())
    );
    assert!(r.mark_terminal("session-7-2-reviewer", AgentStatus::Finished));
    assert_eq!(
        r.status_of("session-7-2-reviewer"),
        Some(AgentStatus::Finished),
        "the entry stays put so a late poller sees Complete, not Unknown"
    );
    assert!(
        !r.mark_terminal("session-7-2-reviewer", AgentStatus::Failed),
        "the first terminal status wins"
    );
}

/// The three defined outcomes of starting a run. `SendMessage` resumes an agent
/// under its original id, so all three are reachable in production depending
/// only on whether the reaper has been past — which must not change the answer.
#[test]
fn register_run_has_one_outcome_per_state_of_the_id() {
    let r = BackgroundAgentRegistry::new();
    let mut first = dummy_handle(AgentStatus::Running);
    first.subagent_id = "resume-me".to_string();
    assert_eq!(r.register_run(first), RunRegistration::Registered);

    let mut concurrent = dummy_handle(AgentStatus::Running);
    concurrent.subagent_id = "resume-me".to_string();
    assert_eq!(
        r.register_run(concurrent),
        RunRegistration::AlreadyRunning,
        "the same run reaching the choke point twice must not disturb the entry"
    );
    assert_eq!(r.status_of("resume-me"), Some(AgentStatus::Running));

    // The run ends but the reaper has not been past yet. A resume now has to
    // revive the entry, or the resumed agent reads as dead while it works.
    r.mark_terminal("resume-me", AgentStatus::Finished);
    let mut resumed = dummy_handle(AgentStatus::Running);
    resumed.subagent_id = "resume-me".to_string();
    assert_eq!(r.register_run(resumed), RunRegistration::Restarted);
    assert_eq!(r.status_of("resume-me"), Some(AgentStatus::Running));

    // And once the reaper has been past, it is an ordinary fresh registration.
    r.mark_terminal("resume-me", AgentStatus::Finished);
    r.reap_finished();
    let mut after_reap = dummy_handle(AgentStatus::Running);
    after_reap.subagent_id = "resume-me".to_string();
    assert_eq!(r.register_run(after_reap), RunRegistration::Registered);
    assert_eq!(r.status_of("resume-me"), Some(AgentStatus::Running));
}

#[test]
fn status_is_terminal_helper() {
    assert!(!AgentStatus::Running.is_terminal());
    assert!(AgentStatus::Finished.is_terminal());
    assert!(AgentStatus::Failed.is_terminal());
    assert!(AgentStatus::Cancelled.is_terminal());
}

// TASK-TUI-402: shim unit test. Running-handle happy-path coverage is
// deferred to TASK-TUI-409 integration tests to avoid contaminating
// the global BACKGROUND_AGENTS singleton across unit-test runs.
#[test]
fn poll_unknown_id_returns_unknown() {
    assert_eq!(poll_background_agent(&Uuid::new_v4()), PollOutcome::Unknown);
}

#[test]
fn reap_removes_terminal() {
    let r = BackgroundAgentRegistry::new();
    let running = dummy_handle(AgentStatus::Running);
    let finished = dummy_handle(AgentStatus::Finished);
    let running_id = running.agent_id;
    let finished_id = finished.agent_id;
    r.register(running).unwrap();
    r.register(finished).unwrap();

    let reaped = r.reap_finished();
    assert!(reaped.contains(&finished_id));
    assert!(!reaped.contains(&running_id));
    assert_eq!(r.get(&running_id), Some(AgentStatus::Running));
    assert_eq!(r.get(&finished_id), None);
}

//! What a dropped run leaves behind.
//!
//! The bug these cover ended a fifteen-task workflow at its third task: a write
//! branch was killed mid-flight, its registration stayed `Running`, and the
//! retry — which re-dispatches under the SAME id — was refused by
//! `register_with_id` with "subagent already exists and is running". The write
//! layer had no classification for that error, so a stranded id was reported as
//! the branch's own unclassified failure.
//!
//! So the assertion that matters is not "the status changed". It is that the
//! id is registrable again, which is what the retry actually needs.

use super::*;

use archon_tools::agent_tool::SubagentRequest;

use crate::subagent::SubagentStatus;

fn probe_request() -> SubagentRequest {
    SubagentRequest {
        prompt: "inspect one thing".to_string(),
        model: None,
        allowed_tools: Vec::new(),
        max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
        timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        write_roots: Vec::new(),
        provider_env: None,
    }
}

fn manager() -> Arc<Mutex<SubagentManager>> {
    Arc::new(Mutex::new(SubagentManager::new(4)))
}

#[tokio::test]
async fn a_dropped_run_frees_its_id_for_the_retry() {
    let manager = manager();
    let id = "wf-run-implement-tdl-020-11-0-attempt-1-coder".to_string();
    manager
        .lock()
        .await
        .register_with_id(id.clone(), probe_request())
        .expect("first registration");

    // The run is abandoned: the guard is dropped without ever being settled,
    // exactly as it is when the enclosing future is cancelled.
    drop(RunRegistration::take(Arc::clone(&manager), id.clone()).await);

    // The behaviour the retry depends on.
    let reregistered = manager
        .lock()
        .await
        .register_with_id(id.clone(), probe_request());
    assert!(
        reregistered.is_ok(),
        "a retry under the same id must be able to register after the run was abandoned: \
         {reregistered:?}"
    );
}

#[tokio::test]
async fn an_abandoned_run_says_why_rather_than_looking_like_an_agent_fault() {
    let manager = manager();
    let id = "abandoned-run".to_string();
    manager
        .lock()
        .await
        .register_with_id(id.clone(), probe_request())
        .expect("registration");

    drop(RunRegistration::take(Arc::clone(&manager), id.clone()).await);

    let guard = manager.lock().await;
    let info = guard.get_status(&id).expect("entry still present");
    let SubagentStatus::Failed(reason) = &info.status else {
        panic!(
            "an abandoned run must hold a terminal status, got {:?}",
            info.status
        );
    };
    assert!(
        reason.contains("without reporting a result"),
        "the reason must say the run vanished, not invent a fault: {reason}"
    );
    assert!(
        reason.contains(&id),
        "the reason must name the run: {reason}"
    );
}

/// A settled guard is inert: the ordinary completion path has already recorded
/// the terminal status, and the drop must not overwrite it with a failure.
#[tokio::test]
async fn a_settled_run_is_left_exactly_as_it_reported_itself() {
    let manager = manager();
    let id = "finished-run".to_string();
    manager
        .lock()
        .await
        .register_with_id(id.clone(), probe_request())
        .expect("registration");

    let mut registration = RunRegistration::take(Arc::clone(&manager), id.clone()).await;
    manager
        .lock()
        .await
        .complete(&id, "the answer".to_string())
        .expect("the run reports its own result");
    registration.settle();
    drop(registration);

    let guard = manager.lock().await;
    let info = guard.get_status(&id).expect("entry present");
    assert_eq!(
        info.status,
        SubagentStatus::Completed,
        "a completed run must not be rewritten to failed by its own guard"
    );
    assert_eq!(info.result.as_deref(), Some("the answer"));
}

/// The contended path: the release is handed to the runtime rather than
/// skipped, so the id still frees up.
#[tokio::test]
async fn a_drop_while_the_manager_is_busy_still_releases_the_id() {
    let manager = manager();
    let id = "contended-run".to_string();
    manager
        .lock()
        .await
        .register_with_id(id.clone(), probe_request())
        .expect("registration");

    let registration = RunRegistration::take(Arc::clone(&manager), id.clone()).await;
    let held = manager.lock().await;
    drop(registration); // try_lock fails here; the release is spawned
    drop(held);

    // Yield until the spawned release has run, bounded so a genuine failure is
    // a failure and not a hang.
    for _ in 0..100 {
        tokio::task::yield_now().await;
        let guard = manager.lock().await;
        if !matches!(
            guard.get_status(&id).map(|info| &info.status),
            Some(SubagentStatus::Running)
        ) {
            return;
        }
        drop(guard);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("a contended drop never released the registration");
}

/// Generation scoping, tested directly rather than through a scheduling race.
///
/// The window this protects against is narrow but real: the guard drops and
/// hands its release to the runtime, something ELSE moves the entry out of
/// `Running` (a timeout sweep, say), a retry takes the id back, and only then is
/// the queued release served. Driving that sequence through the scheduler is not
/// something a test can do deterministically — and an attempt to do so asserted
/// a sequence that cannot happen at all, because on the contended path the
/// release is the only thing that frees the id, so a retry issued before it runs
/// is refused rather than racing it.
///
/// So the property is tested where it lives: a release naming an occupancy that
/// is no longer current must do nothing.
#[tokio::test]
async fn a_release_for_a_superseded_occupancy_does_nothing() {
    let manager = manager();
    let id = "reused-id".to_string();
    let mut guard = manager.lock().await;
    guard
        .register_with_id(id.clone(), probe_request())
        .expect("first registration");
    let first = guard.generation(&id).expect("generation");
    guard
        .mark_failed(&id, "the first run stopped".to_string())
        .expect("stop the first run");
    guard
        .register_with_id(id.clone(), probe_request())
        .expect("the retry takes the id back");

    // The first run's release, arriving late.
    guard
        .mark_failed_at_generation(&id, first, "predecessor's reason".to_string())
        .expect("a superseded release is not an error");

    let info = guard.get_status(&id).expect("entry present");
    assert_eq!(
        info.status,
        SubagentStatus::Running,
        "a predecessor's release must not fail the run that replaced it"
    );
}

/// ...and it must still act when it IS the current occupancy, or the guard
/// would protect nothing.
#[tokio::test]
async fn a_release_for_the_current_occupancy_still_acts() {
    let manager = manager();
    let id = "current".to_string();
    let mut guard = manager.lock().await;
    guard
        .register_with_id(id.clone(), probe_request())
        .expect("registration");
    let generation = guard.generation(&id).expect("generation");

    guard
        .mark_failed_at_generation(&id, generation, "stopped".to_string())
        .expect("release");

    let info = guard.get_status(&id).expect("entry present");
    assert!(
        matches!(info.status, SubagentStatus::Failed(_)),
        "the current occupancy must actually be released, got {:?}",
        info.status
    );
}

/// Each occupancy of an id is distinguishable, which is what makes the check
/// above possible at all.
#[tokio::test]
async fn re_registering_an_id_advances_its_generation() {
    let manager = manager();
    let id = "generational".to_string();
    let mut guard = manager.lock().await;
    guard
        .register_with_id(id.clone(), probe_request())
        .expect("first");
    let first = guard.generation(&id).expect("generation");
    guard.mark_failed(&id, "stopped".to_string()).expect("stop");
    guard
        .register_with_id(id.clone(), probe_request())
        .expect("second");
    let second = guard.generation(&id).expect("generation");

    assert!(
        second > first,
        "a reused id must not present the same generation twice: {first} then {second}"
    );
}

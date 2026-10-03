//! Issue-267: a restart-stage of a generated V2 run rewinds the state and
//! invalidates the V2 cache as one commit under the run lock. The
//! invalidation comes first: when the state commit cannot happen, the old
//! state stands over an invalidated cache (the call runs again), never a
//! rewound state over a valid cache. Every check reads the files back.

#[path = "support/restart_run.rs"]
mod restart_run;

use archon_workflow::{LifecycleAction, LifecycleController};
use restart_run::{accepted, generated_run_with_stages, slot, v2_store};

#[test]
fn restart_stage_commits_the_rewind_and_the_invalidation_together() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run_with_stages(&temp, &["author-a"], &["author-a"]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("author-a", "T-A")).unwrap();
    let before = store.load_state(&run.id).unwrap().generation;

    let (state, invalidated) = LifecycleController::new(store.clone())
        .apply_restart(&run.id, LifecycleAction::RestartStage("author-a".into()))
        .unwrap();

    assert!(
        invalidated.contains(&"author-a".to_string()),
        "{invalidated:?}"
    );
    assert_eq!(state.generation, before + 1);
    assert_eq!(store.load_state(&run.id).unwrap().generation, before + 1);
    assert_eq!(
        slot(&v2, "author-a").unwrap().invalidated_by.as_deref(),
        Some("author-a")
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(events.contains("v2_cache_invalidated"), "{events}");
}

#[test]
fn a_failed_state_commit_leaves_the_cache_invalidated_under_the_old_state() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run_with_stages(&temp, &["author-a"], &["author-a"]);
    let v2 = v2_store(&store, &run);
    v2.save_call_record(&accepted("author-a", "T-A")).unwrap();
    let before = store.load_state(&run.id).unwrap();
    // The state commit (temporary file, then rename) cannot happen.
    let tmp = store.state_path(&run.id).with_extension("json.tmp");
    std::fs::create_dir(&tmp).unwrap();

    let result = LifecycleController::new(store.clone())
        .apply_restart(&run.id, LifecycleAction::RestartStage("author-a".into()));

    assert!(result.is_err(), "{result:?}");
    std::fs::remove_dir(&tmp).unwrap();
    let after = store.load_state(&run.id).unwrap();
    assert_eq!(after.generation, before.generation, "nothing committed");
    assert_eq!(after.stages, before.stages, "nothing rewound");
    assert_eq!(
        slot(&v2, "author-a").unwrap().invalidated_by.as_deref(),
        Some("author-a"),
        "the invalidation is already durable, so the call runs again"
    );
}

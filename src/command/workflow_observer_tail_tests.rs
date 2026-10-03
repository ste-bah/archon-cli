use super::super::workflow_run_finalizer_tests::spec;
use super::*;

#[test]
fn observer_lookup_skips_damage_and_matches_acceptance_identity() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    std::fs::write(store.events_path(&run.id), "{\"seq\":").unwrap();
    emit_shadow_event(&store, &run.id, "acceptance").unwrap();
    assert!(
        observer_event_exists(
            &store,
            &run.id,
            "run_end_acceptance_shadow_observed",
            Some("acceptance")
        )
        .unwrap()
    );
    assert!(
        !observer_event_exists(
            &store,
            &run.id,
            "run_end_acceptance_shadow_observed",
            Some("different")
        )
        .unwrap()
    );
}

#[test]
fn observer_lookup_skips_split_utf8_tail() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    std::fs::write(store.events_path(&run.id), b"{\"detail\":\"\xc3").unwrap();
    emit_shadow_event(&store, &run.id, "acceptance").unwrap();
    assert!(
        observer_event_exists(
            &store,
            &run.id,
            "run_end_acceptance_shadow_observed",
            Some("acceptance")
        )
        .unwrap()
    );
}

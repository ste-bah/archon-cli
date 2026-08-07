//! What the store promises about staleness, contention and schema races.
//!
//! Split out under `#[path]` like the indexer's own test modules, so
//! `index_storage.rs` stays inside the 500-line guard.

use super::*;

fn store_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

#[test]
fn guarding_the_read_leaves_the_answer_unchanged() {
    // The guard wraps the query; it must not alter what the query means.
    let db = store_db();
    let guard = archon_cozo::CozoGuardConfig::default();
    let store = FileStore::new(&db, 8, &guard);
    store.ensure_schema().expect("schema");

    assert!(
        matches!(
            store.file_state("src/lib.rs", "abc").expect("read"),
            FileState::Stale
        ),
        "an unindexed file has no stored hash to match"
    );

    store
        .replace_file_with_cancel("src/lib.rs", "abc", &[], || false)
        .expect("write");

    assert!(
        matches!(
            store.file_state("src/lib.rs", "abc").expect("read"),
            FileState::Current
        ),
        "the stored hash matches"
    );
    assert!(
        matches!(
            store.file_state("src/lib.rs", "def").expect("read"),
            FileState::Stale
        ),
        "a changed hash does not match"
    );
}

#[test]
fn the_observed_busy_error_routes_to_skip_not_failure() {
    // Verbatim from the issue #140 report. If Cozo ever reworded this, the
    // skip branch would go unreachable and the walk would start aborting
    // again on contention -- silently, because the symptom is a log line in
    // a session file. Pin the strings that have to keep classifying: the
    // read's SQLITE_BUSY, and the write's expired wait for the file lock,
    // which is what actually ended the pass in #140.
    assert!(archon_cozo::is_store_contention(
        "file state hash check query: database is locked (code 5)"
    ));
    assert!(archon_cozo::is_store_contention(
        "leann index: replace indexed file: Cozo write lock unavailable at \
         /repo/.archon/leann.db.archon-cozo-write.lock: operation would block"
    ));
    assert!(archon_cozo::is_store_contention(
        "leann index: replace indexed file: Cozo write lock at \
         /repo/.archon/leann.db.archon-cozo-write.lock was still held after waiting 60000ms"
    ));
    // A schema fault is not contention, and must never be skipped past.
    assert!(!archon_cozo::is_store_contention(
        "leann index: replace indexed file: relation code_chunks not found"
    ));
}

#[test]
fn a_racing_creates_conflict_is_recognised_through_the_error_chain() {
    // Issue #144. Cozo reports the racing conflict as `when executing
    // against relation 'X'` with the cause one link down, so the flattened
    // rendering that `run_script_guarded` produces cannot classify it.
    // These are the two shapes `run_idempotent` has to forgive; the bare
    // wrapper, which a malformed schema change also produces, must not be.
    assert!(is_benign_schema_conflict(
        "Stored relation code_chunks conflicts with an existing one"
    ));
    assert!(is_benign_schema_conflict(
        "when executing against relation 'code_chunks': \
         Cannot create relation code_chunks as one with the same name already exists"
    ));
    assert!(!is_benign_schema_conflict(
        "when executing against relation 'code_chunks'"
    ));
}

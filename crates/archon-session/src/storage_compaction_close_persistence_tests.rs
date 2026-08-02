use std::sync::{Arc, Barrier};

use super::storage_compaction_tests::open_store;
use super::*;

#[test]
fn persisted_close_evidence_covers_source_duplicates_delete_race_and_rollback() {
    let (temp, store, session_id) = open_store();
    let store = Arc::new(store);
    let successes = concurrent_duplicate_successes(&store, &session_id);
    assert_eq!(successes, 1);
    let segment = only_segment(&store, &session_id);
    let source = store.load_compaction_segment_body(&segment.id).unwrap();
    assert!(source == vec!["first"] || source == vec!["second"]);
    let rollback_ids = assert_full_record_rollback_and_deletion_race(&store, &session_id);
    let deleted_id = deleted_session_id(&store);
    verify_reopen(
        temp.path(),
        &session_id,
        &segment.id,
        &source,
        &rollback_ids,
        &deleted_id,
    );
}

fn concurrent_duplicate_successes(store: &Arc<SessionStore>, session_id: &str) -> usize {
    let barrier = Arc::new(Barrier::new(3));
    let writers = ["first", "second"].map(|source| {
        let store = Arc::clone(store);
        let barrier = Arc::clone(&barrier);
        let session_id = session_id.to_owned();
        std::thread::spawn(move || {
            barrier.wait();
            store.close_compaction_segment(&session_id, 0, 0, &[source.into()])
        })
    });
    barrier.wait();
    writers
        .map(|writer| writer.join().unwrap())
        .iter()
        .filter(|result| result.is_ok())
        .count()
}

fn only_segment(store: &SessionStore, session_id: &str) -> CompactionSegment {
    let mut segments = store.list_compaction_segments(session_id).unwrap();
    assert_eq!(segments.len(), 1);
    segments.remove(0)
}

fn assert_full_record_rollback_and_deletion_race(
    store: &SessionStore,
    session_id: &str,
) -> (String, String) {
    let ledger = CompactionLedgerRecord {
        id: "ledger-rollback-close".into(),
        session_id: session_id.into(),
        kind: "user_directive".into(),
        payload: "keep directive".into(),
        source_start_index: 1,
        source_end_index: 1,
        created_at: "2026-07-30T00:00:00Z".into(),
    };
    let telemetry = CompactionTelemetryRecord {
        id: format!("telemetry:segment:{session_id}:1:1:closed"),
        session_id: session_id.into(),
        action: "segment_closed".into(),
        payload: "{}".into(),
        created_at: "2026-07-30T00:00:00Z".into(),
    };
    let ledger_id = ledger.id.clone();
    let telemetry_id = telemetry.id.clone();
    store.fail_next_compaction_close_after_records();
    assert!(
        store
            .close_compaction_segment_with_records(
                session_id,
                1,
                1,
                &["rollback".into()],
                &[ledger],
                Some(&telemetry),
            )
            .is_err()
    );
    (ledger_id, telemetry_id)
}

fn deleted_session_id(store: &SessionStore) -> String {
    let deleted = store.create_session("/tmp/deleted", None, "test").unwrap();
    store.delete_before_next_compaction_close_transaction();
    assert!(
        store
            .close_compaction_segment(&deleted.id, 0, 0, &["race".into()])
            .is_err()
    );
    deleted.id
}

fn verify_reopen(
    path: &std::path::Path,
    session_id: &str,
    segment_id: &str,
    source: &[String],
    rollback_ids: &(String, String),
    deleted_id: &str,
) {
    let reopened = SessionStore::open(&path.join("sessions.db")).unwrap();
    let segments = reopened.list_compaction_segments(session_id).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(
        reopened.load_compaction_segment_body(segment_id).unwrap(),
        source
    );
    let failed_id = format!("segment:{session_id}:1:1");
    assert!(
        reopened
            .get_compaction_segment(&failed_id)
            .unwrap()
            .is_none()
    );
    assert!(reopened.load_compaction_segment_body(&failed_id).is_err());
    let ledger = reopened.list_compaction_ledger_records(session_id).unwrap();
    assert!(!ledger.iter().any(|record| record.id == rollback_ids.0));
    let telemetry = reopened.list_compaction_telemetry(session_id).unwrap();
    assert!(!telemetry.iter().any(|record| record.id == rollback_ids.1));
    let deleted_segment_id = format!("segment:{deleted_id}:0:0");
    assert!(
        reopened
            .get_compaction_segment(&deleted_segment_id)
            .unwrap()
            .is_none()
    );
    assert!(
        reopened
            .load_compaction_segment_body(&deleted_segment_id)
            .is_err()
    );
    assert!(reopened.get_session(deleted_id).is_err());
    println!(
        "EVIDENCE session_close_persisted source_rows={} duplicate_writers=2 duplicate_successes=1 deletion_race=session_deleted,segment_absent,body_absent full_record_rollback=segment_absent,body_absent,ledger_id_absent,telemetry_id_absent reopened_segments={}",
        source.len(),
        segments.len()
    );
}

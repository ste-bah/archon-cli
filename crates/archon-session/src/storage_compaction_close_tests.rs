use super::storage_compaction_tests::{body, open_store};
use super::*;

#[test]
fn duplicate_close_with_different_body_is_rejected_without_overwrite() {
    let (_temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 0, &["canonical".into()])
        .unwrap();

    assert!(
        store
            .close_compaction_segment(&session_id, 0, 0, &["replacement".into()])
            .is_err()
    );
    assert_eq!(
        store.load_compaction_segment_body(&segment.id).unwrap(),
        vec!["canonical"]
    );
}

#[test]
fn concurrent_duplicate_closes_never_overwrite_source_body() {
    let (_temp, store, session_id) = open_store();
    let store = std::sync::Arc::new(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|content| {
            let store = std::sync::Arc::clone(&store);
            let barrier = std::sync::Arc::clone(&barrier);
            let session_id = session_id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.close_compaction_segment(&session_id, 0, 0, &[content.into()])
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let segment = store
        .list_compaction_segments(&session_id)
        .unwrap()
        .into_iter()
        .next()
        .expect("one segment");
    let body = store.load_compaction_segment_body(&segment.id).unwrap();
    assert!(body == vec!["first"] || body == vec!["second"]);
}

#[test]
fn close_fails_if_session_is_deleted_before_close_transaction() {
    let (_temp, store, session_id) = open_store();
    store.delete_before_next_compaction_close_transaction();

    assert!(
        store
            .close_compaction_segment(&session_id, 0, 0, &["source".into()])
            .is_err()
    );
    assert!(
        store
            .list_compaction_segments(&session_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn failed_atomic_close_leaves_no_partial_compaction_records() {
    let (_temp, store, session_id) = open_store();
    let ledger = CompactionLedgerRecord {
        id: "ledger-atomic-close".into(),
        session_id: session_id.clone(),
        kind: "user_directive".into(),
        payload: "keep directive".into(),
        source_start_index: 0,
        source_end_index: 0,
        created_at: "2026-07-30T00:00:00Z".into(),
    };
    let telemetry = CompactionTelemetryRecord {
        id: format!("telemetry:segment:{session_id}:0:0:closed"),
        session_id: session_id.clone(),
        action: "segment_closed".into(),
        payload: "{}".into(),
        created_at: "2026-07-30T00:00:00Z".into(),
    };

    store.fail_next_compaction_close_after_body();
    assert!(
        store
            .close_compaction_segment_with_records(
                &session_id,
                0,
                0,
                &body(),
                &[ledger],
                Some(&telemetry),
            )
            .is_err()
    );

    let segment_id = format!("segment:{session_id}:0:0");
    assert!(store.get_compaction_segment(&segment_id).unwrap().is_none());
    assert!(store.load_compaction_segment_body(&segment_id).is_err());
    assert!(
        store
            .list_compaction_ledger_records(&session_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_compaction_telemetry(&session_id)
            .unwrap()
            .is_empty()
    );
}

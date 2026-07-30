use super::*;

pub(super) fn open_store() -> (tempfile::TempDir, SessionStore, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&temp.path().join("sessions.db")).expect("open store");
    let session = store
        .create_session("/tmp/project", Some("audit"), "claude-sonnet-4-6")
        .expect("create session");
    (temp, store, session.id)
}

pub(super) fn body() -> Vec<String> {
    vec![
        serde_json::json!({"role":"user","content":"keep directive"}).to_string(),
        serde_json::json!({"role":"assistant","content":[{
            "type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"src/lib.rs"}
        }]})
        .to_string(),
        serde_json::json!({"role":"user","content":[{
            "type":"tool_result","tool_use_id":"tool-1","content":"file body","is_error":false
        }]})
        .to_string(),
    ]
}

#[test]
fn duplicate_segment_close_is_idempotent_and_preserves_one_source_body() {
    let (_temp, store, session_id) = open_store();

    let first = store
        .close_compaction_segment(&session_id, 0, 2, &body())
        .expect("close segment");
    let duplicate = store
        .close_compaction_segment(&session_id, 0, 2, &body())
        .expect("repeat close");

    assert_eq!(first.id, duplicate.id);
    assert_eq!(
        store.list_compaction_segments(&session_id).unwrap().len(),
        1
    );
    assert_eq!(
        store.load_compaction_segment_body(&first.id).unwrap(),
        body()
    );
    assert_eq!(first.summary_status, CompactionSummaryStatus::Pending);
}

#[test]
fn segment_identity_is_stable_across_store_restart() {
    let (temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 3, 5, &body())
        .expect("close segment");
    drop(store);

    let reopened = SessionStore::open(&temp.path().join("sessions.db")).expect("reopen store");
    let loaded = reopened
        .get_compaction_segment(&segment.id)
        .expect("load segment")
        .expect("segment exists");

    assert_eq!(loaded.id, segment.id);
    assert_eq!(loaded.start_index, 3);
    assert_eq!(loaded.end_index, 5);
    assert_eq!(
        reopened.load_compaction_segment_body(&segment.id).unwrap(),
        body()
    );
}

#[test]
fn summary_claim_and_completion_are_once_per_segment() {
    let (_temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 2, &body())
        .expect("close segment");

    let claim = store
        .claim_compaction_segment_summary(
            &segment.id,
            "claude-haiku-4-5-20251001",
            "{\"scope\":\"main\"}",
        )
        .expect("claim summary")
        .expect("claim token");
    assert!(
        store
            .claim_compaction_segment_summary(&segment.id, "claude-haiku-4-5-20251001", "{}")
            .expect("duplicate claim")
            .is_none()
    );
    store
        .complete_compaction_segment_summary(&segment.id, &claim, "summary text", 120, 30, 0.001)
        .expect("complete summary");
    assert!(
        store
            .claim_compaction_segment_summary(&segment.id, "claude-haiku-4-5-20251001", "{}")
            .expect("claim completed")
            .is_none()
    );

    let loaded = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert_eq!(loaded.summary_status, CompactionSummaryStatus::Succeeded);
    assert_eq!(loaded.summary.as_deref(), Some("summary text"));
    assert_eq!(
        loaded.summary_model.as_deref(),
        Some("claude-haiku-4-5-20251001")
    );
    assert_eq!(loaded.summary_input_tokens, Some(120));
    assert_eq!(loaded.summary_output_tokens, Some(30));
    assert_eq!(loaded.summary_cost, Some(0.001));
}

#[test]
fn failed_summary_keeps_source_body_and_can_be_retried_after_restart() {
    let (temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 2, &body())
        .expect("close segment");
    let claim = store
        .claim_compaction_segment_summary(&segment.id, "missing-model", "{}")
        .unwrap()
        .expect("claim token");
    store
        .fail_compaction_segment_summary(&segment.id, &claim, "model unavailable")
        .expect("record failure");
    drop(store);

    let reopened = SessionStore::open(&temp.path().join("sessions.db")).expect("reopen store");
    let failed = reopened
        .get_compaction_segment(&segment.id)
        .unwrap()
        .unwrap();
    assert_eq!(failed.summary_status, CompactionSummaryStatus::Failed);
    assert_eq!(failed.summary_failure.as_deref(), Some("model unavailable"));
    assert_eq!(
        reopened.load_compaction_segment_body(&segment.id).unwrap(),
        body()
    );
    assert!(
        reopened
            .claim_compaction_segment_summary(&segment.id, "fallback-model", "{}")
            .expect("retry failed summary")
            .is_some()
    );
}

#[test]
fn ledger_records_are_idempotent_and_retain_source_provenance() {
    let (temp, store, session_id) = open_store();
    let record = CompactionLedgerRecord {
        id: "ledger-command-1".into(),
        session_id: session_id.clone(),
        kind: "command".into(),
        payload: serde_json::json!({"command":"cargo test","exit_code":0}).to_string(),
        source_start_index: 4,
        source_end_index: 5,
        created_at: "2026-07-30T00:00:00Z".into(),
    };

    store.put_compaction_ledger_record(&record).unwrap();
    store.put_compaction_ledger_record(&record).unwrap();
    drop(store);

    let reopened = SessionStore::open(&temp.path().join("sessions.db")).expect("reopen store");
    let records = reopened
        .list_compaction_ledger_records(&session_id)
        .unwrap();
    assert_eq!(records, vec![record]);
}

#[test]
fn segment_telemetry_is_durable_and_idempotent_by_event_id() {
    let (temp, store, session_id) = open_store();
    let event = CompactionTelemetryRecord {
        id: "compaction-event-1".into(),
        session_id: session_id.clone(),
        action: "segment_closed".into(),
        payload: serde_json::json!({"before_bytes":900000,"after_bytes":400000}).to_string(),
        created_at: "2026-07-30T00:00:00Z".into(),
    };

    store.put_compaction_telemetry(&event).unwrap();
    store.put_compaction_telemetry(&event).unwrap();
    drop(store);

    let reopened = SessionStore::open(&temp.path().join("sessions.db")).expect("reopen store");
    assert_eq!(
        reopened.list_compaction_telemetry(&session_id).unwrap(),
        vec![event]
    );
}

#[test]
fn recalled_segment_requires_session_authorization_and_respects_redaction() {
    let (_temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 0, &["secret directive".into()])
        .unwrap();

    assert!(
        store
            .load_authorized_compaction_segment_body(&session_id, &segment.id)
            .is_ok()
    );
    assert!(
        store
            .load_authorized_compaction_segment_body("other-session", &segment.id)
            .is_err()
    );

    store
        .put_compaction_ledger_record(&CompactionLedgerRecord {
            id: "ledger-redact".into(),
            session_id: session_id.clone(),
            kind: "user_directive".into(),
            payload: "secret directive".into(),
            source_start_index: 0,
            source_end_index: 0,
            created_at: "2026-07-30T00:00:00Z".into(),
        })
        .unwrap();
    store
        .redact_compaction_segment(&session_id, &segment.id, "policy deletion")
        .unwrap();
    assert!(
        store
            .load_authorized_compaction_segment_body(&session_id, &segment.id)
            .is_err()
    );
    let stored = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert_eq!(stored.summary_status, CompactionSummaryStatus::Redacted);
    assert_eq!(
        stored.summary_failure.as_deref(),
        Some("redacted: policy deletion")
    );
    assert!(
        store
            .list_compaction_ledger_records(&session_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn provider_failed_segments_are_recoverable_but_invalid_sources_are_not() {
    let (_temp, store, session_id) = open_store();
    let provider_failed = store
        .close_compaction_segment(&session_id, 0, 0, &["provider source".into()])
        .unwrap();
    let claim = store
        .claim_compaction_segment_summary(&provider_failed.id, "model", "{}")
        .unwrap()
        .expect("claim token");
    store
        .fail_compaction_segment_summary(
            &provider_failed.id,
            &claim,
            "provider summary failed: provider unavailable",
        )
        .unwrap();
    let invalid_source = store
        .close_compaction_segment(&session_id, 1, 1, &["invalid source".into()])
        .unwrap();
    store
        .mark_compaction_segment_source_invalid(
            &invalid_source.id,
            "invalid persisted source message: missing role",
        )
        .unwrap();
    let invalid_summary = store
        .close_compaction_segment(&session_id, 2, 2, &["summary source".into()])
        .unwrap();
    let claim = store
        .claim_compaction_segment_summary(&invalid_summary.id, "model", "{}")
        .unwrap()
        .expect("summary claim token");
    store
        .fail_compaction_segment_summary(
            &invalid_summary.id,
            &claim,
            "invalid compaction summary: provider returned empty summary",
        )
        .unwrap();

    let recovered = store.recoverable_compaction_segments(&session_id).unwrap();
    let ids: Vec<_> = recovered
        .iter()
        .map(|segment| segment.id.as_str())
        .collect();

    assert_eq!(ids, vec![provider_failed.id.as_str()]);
    assert_eq!(recovered[0].summary_status, CompactionSummaryStatus::Failed);
}

#[test]
fn pending_or_running_segments_are_recoverable_after_restart() {
    let (temp, store, session_id) = open_store();
    let pending = store
        .close_compaction_segment(&session_id, 0, 0, &["pending".into()])
        .unwrap();
    let running = store
        .close_compaction_segment(&session_id, 1, 1, &["running".into()])
        .unwrap();
    assert!(
        store
            .claim_compaction_segment_summary(&running.id, "model", "{}")
            .unwrap()
            .is_some()
    );
    drop(store);

    let reopened = SessionStore::open(&temp.path().join("sessions.db")).unwrap();
    let recovered = reopened
        .recoverable_compaction_segments(&session_id)
        .unwrap();
    let ids: Vec<_> = recovered
        .iter()
        .map(|segment| segment.id.as_str())
        .collect();
    assert_eq!(ids, vec![pending.id.as_str(), running.id.as_str()]);
    assert!(
        recovered
            .iter()
            .all(|segment| segment.summary_status == CompactionSummaryStatus::Pending)
    );
}

#[test]
fn stale_worker_cannot_complete_redacted_segment() {
    let (_temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 0, &["sensitive".into()])
        .unwrap();
    let claim = store
        .claim_compaction_segment_summary(&segment.id, "model", "{}")
        .unwrap()
        .expect("claim token");

    store
        .redact_compaction_segment(&session_id, &segment.id, "policy deletion")
        .unwrap();
    assert!(
        !store
            .complete_compaction_segment_summary(
                &segment.id,
                &claim,
                "leaked sensitive summary",
                1,
                1,
                0.0,
            )
            .unwrap()
    );

    let stored = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert_eq!(stored.summary_status, CompactionSummaryStatus::Redacted);
    assert!(stored.summary.is_none());
    assert!(stored.summary_failure.unwrap().starts_with("redacted:"));
}

#[test]
fn stale_recovery_snapshot_cannot_overwrite_completed_summary() {
    let (_temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 0, &["source".into()])
        .unwrap();
    let claim = store
        .claim_compaction_segment_summary(&segment.id, "model", "{}")
        .unwrap()
        .expect("claim token");
    let running_snapshot = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert!(
        store
            .complete_compaction_segment_summary(&segment.id, &claim, "complete", 1, 1, 0.0)
            .unwrap()
    );

    assert!(
        store
            .recover_interrupted_compaction_segment(&running_snapshot)
            .unwrap()
            .is_none()
    );
    let stored = store.get_compaction_segment(&segment.id).unwrap().unwrap();
    assert_eq!(stored.summary_status, CompactionSummaryStatus::Succeeded);
    assert_eq!(stored.summary.as_deref(), Some("complete"));
}

#[test]
fn deleting_session_removes_compaction_source_records() {
    let (_temp, store, session_id) = open_store();
    let segment = store
        .close_compaction_segment(&session_id, 0, 0, &["source".into()])
        .unwrap();
    store
        .put_compaction_ledger_record(&CompactionLedgerRecord {
            id: "ledger-delete".into(),
            session_id: session_id.clone(),
            kind: "user_directive".into(),
            payload: "source".into(),
            source_start_index: 0,
            source_end_index: 0,
            created_at: "2026-07-30T00:00:00Z".into(),
        })
        .unwrap();
    store
        .put_compaction_telemetry(&CompactionTelemetryRecord {
            id: "telemetry-delete".into(),
            session_id: session_id.clone(),
            action: "segment_closed".into(),
            payload: "{}".into(),
            created_at: "2026-07-30T00:00:00Z".into(),
        })
        .unwrap();

    store.delete_session(&session_id).unwrap();

    assert!(store.get_compaction_segment(&segment.id).unwrap().is_none());
    assert!(store.load_compaction_segment_body(&segment.id).is_err());
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

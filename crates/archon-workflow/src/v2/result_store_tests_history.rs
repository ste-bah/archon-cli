// Issue-250: an interrupted re-run must never cost a call its last accepted
// record. Every test reads the files back from a real directory.

fn timed_record(
    id: &str,
    attempt: u32,
    input: &str,
    result: WorkflowV2Result,
    at: &str,
) -> WorkflowV2CallRecord {
    let mut record = WorkflowV2CallRecord::new(
        "wf-test",
        call(id),
        attempt,
        input.to_string(),
        result,
        Vec::new(),
    );
    record.started_at = at.to_string();
    record.finished_at = at.to_string();
    record
}

fn accepted_at(id: &str, attempt: u32, input: &str, at: &str) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result::accepted("done");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Other,
        "criterion checked",
    ));
    timed_record(id, attempt, input, result, at)
}

/// The record a fixed run writes into the slot when it dispatches a call.
fn running_at(id: &str, attempt: u32, input: &str, at: &str) -> WorkflowV2CallRecord {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Running,
        summary: "call in flight".to_string(),
        ..WorkflowV2Result::default()
    };
    timed_record(id, attempt, input, result, at)
}

/// The record a pause, a cancel or a dead host leaves for the attempt.
fn interrupted_at(
    id: &str,
    attempt: u32,
    input: &str,
    reason: &str,
    at: &str,
) -> WorkflowV2CallRecord {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!("call was {reason} and produced no result"),
        data: serde_json::json!({ "call_id": id, "interrupted": reason }),
        ..WorkflowV2Result::default()
    };
    timed_record(id, attempt, input, result, at)
}

fn slot_json(store: &WorkflowV2ResultStore, id: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(store.result_path(id)).expect("slot file");
    serde_json::from_str(&raw).expect("slot json")
}

/// Every archived record of every call (Issue-254: one directory per call).
fn archived_json(temp: &tempfile::TempDir) -> Vec<serde_json::Value> {
    let root = temp.path().join("results").join("history");
    std::fs::read_dir(root)
        .map(|calls| {
            calls
                .flatten()
                .flat_map(|call| std::fs::read_dir(call.path()).into_iter().flatten())
                .flatten()
                .map(|entry| {
                    let raw = std::fs::read_to_string(entry.path()).expect("archived file");
                    serde_json::from_str(&raw).expect("archived json")
                })
                .collect()
        })
        .unwrap_or_default()
}

const T1: &str = "2026-10-02T11:48:49.722657+00:00";
const T2: &str = "2026-10-03T04:27:18.9+00:00";
const T2_END: &str = "2026-10-03T04:31:04.989156+00:00";
const T3: &str = "2026-10-03T04:32:49.606399+00:00";
const T4: &str = "2026-10-03T05:16:09+00:00";

#[test]
fn accepted_then_interrupted_attempt_resume_reuses_and_restores_accepted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "acceptance-author-req-1";
    let accepted = accepted_at(id, 1, "input-x", T1);
    store.save_call_record(&accepted).expect("save accepted");
    // A resume with drifted input starts attempt 2, then a pause stops it.
    store
        .save_call_record(&running_at(id, 2, "input-y", T2))
        .expect("running");
    assert_eq!(slot_json(&store, id)["status"], "running");
    store
        .save_call_record(&interrupted_at(id, 2, "input-y", "paused", T2_END))
        .expect("interrupted");
    assert_eq!(slot_json(&store, id)["status"], "needs_review");
    assert_eq!(slot_json(&store, id)["attempt"], 2);

    // The next resume, with the input the accepted record answered.
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .expect("candidate")
        .expect("some record");
    assert!(candidate.from_history);
    assert_eq!(candidate.record, accepted);
    store
        .restore_call_record(&candidate.record)
        .expect("restore");

    let slot = slot_json(&store, id);
    assert_eq!(slot["status"], "accepted");
    assert_eq!(slot["attempt"], 1);
    assert_eq!(slot["input_hash"], "input-x");
    assert_eq!(store.load_call_record(id).unwrap().unwrap(), accepted);
    let archived = archived_json(&temp);
    assert!(
        archived
            .iter()
            .any(|record| record["attempt"] == 2
                && record["result"]["data"]["interrupted"] == "paused"),
        "the interrupted attempt stays in the history: {archived:?}"
    );
    assert_eq!(store.next_attempt(id).expect("next attempt"), 3);
    // Restored: the slot answers by itself now.
    let again = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!again.from_history);
    assert_eq!(again.record, accepted);
    assert_eq!(store.load_call_records().expect("records").len(), 1);
}

#[test]
fn drifted_input_never_reuses_the_last_accepted_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-drift";
    store
        .save_call_record(&accepted_at(id, 1, "input-x", T1))
        .unwrap();
    store
        .save_call_record(&interrupted_at(id, 2, "input-y", "paused", T2_END))
        .unwrap();
    let candidate = store
        .call_record_for_reuse(&call(id), "input-y")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert_eq!(candidate.record.attempt, 2);
    assert!(!candidate.record.is_reusable_for("input-y"));
}

#[test]
fn interrupted_retries_with_the_same_input_still_reuse_accepted() {
    // The wf-913e62ae pattern: attempt 2 drifted, attempts 3 and 4 asked the
    // accepted input again and were interrupted (a pause, a dead host).
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-retries";
    let accepted = accepted_at(id, 1, "input-x", T1);
    store.save_call_record(&accepted).unwrap();
    store
        .save_call_record(&interrupted_at(id, 2, "input-y", "paused", T2_END))
        .unwrap();
    store
        .save_call_record(&interrupted_at(id, 3, "input-x", "paused", T3))
        .unwrap();
    store
        .save_call_record(&running_at(id, 4, "input-x", T4))
        .unwrap();
    store
        .save_call_record(&interrupted_at(id, 4, "input-x", "host_process_ended", T4))
        .unwrap();
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(candidate.from_history);
    assert_eq!(candidate.record, accepted);
    assert_eq!(store.next_attempt(id).unwrap(), 5);
}

#[test]
fn accepted_then_new_attempt_accepted_replaces_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-replace";
    store
        .save_call_record(&accepted_at(id, 1, "input-x", T1))
        .unwrap();
    let newer = accepted_at(id, 2, "input-y", T2_END);
    store.save_call_record(&newer).unwrap();
    assert_eq!(slot_json(&store, id)["input_hash"], "input-y");
    // Accepted in the slot: nothing older is consulted, for any input.
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert_eq!(candidate.record, newer);

    // A third attempt is interrupted: the newer accepted record is the last
    // accepted one; the older one was replaced and never comes back.
    store
        .save_call_record(&interrupted_at(id, 3, "input-z", "cancelled", T3))
        .unwrap();
    let old_input = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!old_input.from_history);
    assert_eq!(old_input.record.attempt, 3);
    let new_input = store
        .call_record_for_reuse(&call(id), "input-y")
        .unwrap()
        .unwrap();
    assert!(new_input.from_history);
    assert_eq!(new_input.record, newer);
}

#[test]
fn crash_mid_write_leaves_the_accepted_record_intact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-crash";
    let accepted = accepted_at(id, 1, "input-x", T1);
    store.save_call_record(&accepted).unwrap();
    let slot = store.result_path(id);
    let torn = slot.with_extension("json.tmp");

    // Killed while the new attempt's record was being written: the torn
    // temporary file never replaced the slot.
    std::fs::write(&torn, br#"{"call": {"id": "author-crash""#).unwrap();
    assert_eq!(store.load_call_record(id).unwrap().unwrap(), accepted);
    assert_eq!(store.load_call_records().unwrap().len(), 1);
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert_eq!(candidate.record, accepted);

    // Killed between the archive and the write of the new record: the slot
    // is empty, and the accepted record is found in the history.
    archive_superseded_json(&slot, false, |_: &WorkflowV2CallRecord| false).unwrap();
    assert!(!slot.exists());
    assert!(store.load_call_record(id).unwrap().is_none());
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(candidate.from_history);
    assert_eq!(candidate.record, accepted);
    store.restore_call_record(&candidate.record).unwrap();
    assert_eq!(slot_json(&store, id)["status"], "accepted");
    assert_eq!(store.load_call_record(id).unwrap().unwrap(), accepted);
}

#[test]
fn invalidation_is_never_undone_by_the_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-invalidated";
    store
        .save_call_record(&accepted_at(id, 1, "input-x", T1))
        .unwrap();
    let mut interrupted = interrupted_at(id, 2, "input-y", "paused", T2_END);
    store.save_call_record(&interrupted).unwrap();
    // `workflow restart` invalidates whatever holds the slot.
    interrupted.invalidated_by = Some("restart".to_string());
    store.save_call_record(&interrupted).unwrap();
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert!(
        store
            .last_accepted_call_record(id, "input-x")
            .unwrap()
            .is_none()
    );

    // An accepted record restored and then invalidated stays invalidated,
    // although its clean copy is still in the archive.
    let other = "author-restored-invalidated";
    let accepted = accepted_at(other, 1, "input-x", T1);
    store.save_call_record(&accepted).unwrap();
    store
        .save_call_record(&interrupted_at(other, 2, "input-y", "paused", T2_END))
        .unwrap();
    store.restore_call_record(&accepted).unwrap();
    let mut stamped = accepted.clone();
    stamped.invalidated_by = Some("restart".to_string());
    store.save_call_record(&stamped).unwrap();
    store
        .save_call_record(&interrupted_at(other, 3, "input-z", "paused", T3))
        .unwrap();
    assert!(
        store
            .last_accepted_call_record(other, "input-x")
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_later_finished_answer_to_the_same_input_wins() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-answered-again";
    store
        .save_call_record(&accepted_at(id, 1, "input-x", T1))
        .unwrap();
    let failed = timed_record(
        id,
        2,
        "input-x",
        WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary: "broke".to_string(),
            ..WorkflowV2Result::default()
        },
        T2_END,
    );
    store.save_call_record(&failed).unwrap();
    store
        .save_call_record(&interrupted_at(id, 3, "input-y", "paused", T3))
        .unwrap();
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert_eq!(candidate.record.attempt, 3);
}

#[test]
fn records_archived_before_the_fix_are_found_and_reused() {
    // A pre-fix run: the accepted record was archived by D79 when the
    // interrupted attempt took the slot, in the older on-disk shape (no
    // schema_version, output_hash or optional fields).
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "acceptance-author-legacy";
    store
        .save_call_record(&interrupted_at(
            id,
            2,
            "input-y",
            "host_process_ended",
            T2_END,
        ))
        .unwrap();
    let stem = store
        .result_path(id)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let dir = temp.path().join("results").join("superseded");
    std::fs::create_dir_all(&dir).unwrap();
    let legacy = serde_json::json!({
        "run_id": "wf-old",
        "call": { "id": id, "method": "agent", "write_mode": null, "options": {} },
        "attempt": 1,
        "started_at": T1,
        "finished_at": T1,
        "input_hash": "input-x",
        "status": "accepted",
        "result": {
            "status": "accepted",
            "summary": "done",
            "evidence": [{ "kind": "other", "summary": "criterion checked" }]
        },
        "depends_on": []
    });
    std::fs::write(
        dir.join(format!("{stem}-1791001638926510000-19285-0.json")),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(candidate.from_history);
    assert_eq!(candidate.record.attempt, 1);
    assert_eq!(candidate.record.run_id, "wf-old");
    store.restore_call_record(&candidate.record).unwrap();
    assert_eq!(slot_json(&store, id)["status"], "accepted");
    assert_eq!(store.next_attempt(id).unwrap(), 3);

    // Without a finish time the order is unknown: no answer from history.
    let undated = "acceptance-author-undated";
    store
        .save_call_record(&interrupted_at(undated, 2, "input-y", "paused", T2_END))
        .unwrap();
    let stem = store
        .result_path(undated)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let mut old = legacy.clone();
    old["call"]["id"] = serde_json::json!(undated);
    old.as_object_mut().unwrap().remove("started_at");
    old.as_object_mut().unwrap().remove("finished_at");
    // A flat file that appears after the migration is moved on next touch.
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{stem}-1-1-1.json")),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    assert!(
        store
            .last_accepted_call_record(undated, "input-x")
            .unwrap()
            .is_none()
    );
    assert_eq!(store.next_attempt(undated).unwrap(), 3);
}

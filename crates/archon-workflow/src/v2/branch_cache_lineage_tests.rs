//! A replayed fix's lineage names the execution whose answer was replayed.

use super::*;

fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

/// An earlier session as the host leaves it: fix 31's branch outcome, then
/// its call record, then a verdict that finished after it.
fn earlier_fix_and_verdict(store: &WorkflowV2ResultStore, fix: &WorkflowV2FanoutItem) {
    let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
    let call_id = fanout_call_id(fix);
    let data = serde_json::json!({ "patch_landed": true, "branch_id": fix.id, "answer": "A" });
    earlier
        .save_branch_outcome(
            &call_id,
            &outcome_for(fix, WorkflowV2Status::Accepted, None, data),
        )
        .expect("outcome");
    pause();
    let mut call = fix.call.clone();
    call.id = call_id;
    call.method = WorkflowV2HostMethod::Fanout;
    let record = |call| {
        WorkflowV2CallRecord::new(
            "run",
            call,
            1,
            "input".to_string(),
            result(WorkflowV2Status::Accepted, serde_json::json!({})),
            Vec::new(),
        )
    };
    earlier.save_call_record(&record(call)).expect("fix record");
    pause();
    let mut verdict = verdict_call();
    verdict.id = "verification-wave-review-verify-task-b-1-32".to_string();
    earlier.save_call_record(&record(verdict)).expect("verdict");
    pause();
}

fn verdict_call() -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        serde_json::json!({ "version": 1, "stage": "verify", "taskId": "TASK-B", "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    WorkflowV2HostCall {
        id: String::new(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    }
}

fn verdict_replays(store: &WorkflowV2ResultStore) -> bool {
    let records = store.load_call_records().expect("records");
    let verdict = records
        .iter()
        .find(|record| record.call.id.starts_with("verification-wave-"))
        .expect("verdict");
    crate::v2::script::resume_verdict::verdict_vouches_for_session_fix(verdict, &records, store)
}

/// A session killed after the fix re-ran and saved its branch outcome, but
/// before its call record: the outcome on disk is a new answer beside the
/// old record. Replaying it must not let the old verdict stand for it; an
/// untouched outcome still carries its verdict.
#[test]
fn an_answer_saved_after_its_record_carries_no_verdict() {
    for killed in [false, true] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let fix = remediation_item("TASK-B", 1, 31, "[f1]");
        earlier_fix_and_verdict(&store, &fix);
        if killed {
            let crashed = WorkflowV2ResultStore::new(store.root().to_path_buf());
            let data =
                serde_json::json!({ "patch_landed": true, "branch_id": fix.id, "answer": "B" });
            crashed
                .save_branch_outcome(
                    &fanout_call_id(&fix),
                    &outcome_for(&fix, WorkflowV2Status::Accepted, None, data),
                )
                .expect("the re-run's outcome");
        }
        let (reused, pending) =
            split_reusable_branch_outcomes(&store, &fanout_call_id(&fix), vec![fix.clone()])
                .expect("split");
        assert_eq!((reused.len(), pending.len()), (1, 0), "killed: {killed}");
        let key = crate::v2::script::resume_verdict::remediation_round_key(&fix.call).unwrap();
        assert_eq!(
            store.fix_replayed_from(&key).is_some(),
            !killed,
            "killed: {killed}"
        );
        assert_eq!(verdict_replays(&store), !killed, "killed: {killed}");
    }
}

/// Another ordinal of the same label that ran after the replayed record --
/// with no record of its own -- means the replayed record is not the
/// label's latest execution: no verdict follows.
#[test]
fn a_later_record_less_run_of_the_label_carries_no_verdict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let fix = remediation_item("TASK-B", 1, 31, "[f1]");
    earlier_fix_and_verdict(&store, &fix);
    let other = remediation_item("TASK-B", 1, 33, "[f1]");
    let data = serde_json::json!({ "patch_landed": true, "branch_id": other.id });
    WorkflowV2ResultStore::new(store.root().to_path_buf())
        .save_branch_outcome(
            &fanout_call_id(&other),
            &outcome_for(&other, WorkflowV2Status::Accepted, None, data),
        )
        .expect("record-less run");
    split_reusable_branch_outcomes(&store, &fanout_call_id(&fix), vec![fix.clone()])
        .expect("split");
    assert!(!verdict_replays(&store));
}

/// Issue-111: a session that replayed the fix re-saved its record as a new
/// attempt. The next session's replay must still pair the verdict with the
/// execution it judged -- whether that re-save named the execution
/// (`answered_by`, written from now on) or predates the field (legacy: the
/// answer on disk bounds when it was made).
#[test]
fn a_fix_record_resaved_by_a_replaying_session_still_carries_its_verdict() {
    for stamped in [true, false] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let fix = remediation_item("TASK-B", 1, 31, "[f1]");
        earlier_fix_and_verdict(&store, &fix);
        let call_id = fanout_call_id(&fix);
        let replaying = WorkflowV2ResultStore::new(store.root().to_path_buf());
        split_reusable_branch_outcomes(&replaying, &call_id, vec![fix.clone()]).expect("split");
        let mut resaved = replaying.load_call_record(&call_id).unwrap().unwrap();
        resaved.attempt = 2;
        resaved.finished_at = chrono::Utc::now().to_rfc3339();
        if stamped {
            replaying.save_call_record(&resaved).expect("re-save");
            let on_disk = replaying.load_call_record(&call_id).unwrap().unwrap();
            assert!(
                on_disk.answered_by.is_some(),
                "the re-save names its execution"
            );
        } else {
            let path = replaying.result_path(&call_id);
            std::fs::write(&path, serde_json::to_vec(&resaved).unwrap()).unwrap();
        }
        pause();
        let next = WorkflowV2ResultStore::new(store.root().to_path_buf());
        split_reusable_branch_outcomes(&next, &call_id, vec![fix.clone()]).expect("split");
        assert!(verdict_replays(&next), "stamped: {stamped}");
    }
}

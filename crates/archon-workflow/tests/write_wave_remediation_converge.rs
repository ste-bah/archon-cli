//! Issue-109: a remediation verdict re-asked on one resume replays on the
//! next, through the production write-wave seam with real Git writes.
//!
//! A fix filed under a shifted ordinal is answered by refiling a drifted
//! sibling's outcome under its own id. When that refile cannot be proven to
//! be the sibling's execution (a copy under the fix's own id postdates the
//! sibling's record), the verdict is asked again -- once. Live on
//! wf-0ddadd81 the same verifiers ran fresh on every resume instead: each
//! session refiled the sibling again and the copy the previous session left
//! kept postdating it. A real re-execution must still be refused.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeMap;

use archon_workflow::v2::call_data::fanout_items_for_call;
use archon_workflow::v2::script::resume_verdict::{
    remediation_round_key, verdict_vouches_for_session_fix,
};
use archon_workflow::*;
use serde_json::json;
use support::{AuditScript, Edits, Fixture, git};

/// The fix and verdict as the earlier decomposition numbered them, and as
/// the resumed prelude numbers them.
const SIBLING_FIX: u64 = 33;
const OWN_FIX: u64 = 31;
const SIBLING_VERDICT: &str = "verification-wave-review-verify-task-001-1-34";
const VERDICT: &str = "verification-wave-review-verify-task-001-1-32";

fn contract(stage: &str) -> serde_json::Value {
    json!({ "version": 1, "stage": stage, "taskId": "TASK-001", "round": 1, "maxRounds": 2,
        "sourceReduceCallIds": ["adversarial-review-reduce"] })
}

fn fix_call(ordinal: u64) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions {
        item_kind: Some("implementation".into()),
        task: Some("Post-review remediation.".into()),
        target_files_from_item: true,
        ..Default::default()
    };
    options
        .extra
        .insert("remediationContract".into(), contract("remediate"));
    WorkflowV2HostCall {
        id: format!("review-remediate-task-001-1-{ordinal}"),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options,
    }
}

fn verdict_call(id: &str) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert("remediationContract".into(), contract("verify"));
    WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: None,
        options,
    }
}

fn edits(content: &'static str) -> Edits {
    Edits {
        files: vec![("owned.txt", content)],
        report: vec!["owned.txt"],
        via_adapter: false,
    }
}

/// The fix's wave alone, as the host builds it: branch outcomes and apply
/// are persisted, the call record is not. The number of agents dispatched.
async fn wave(
    f: &Fixture,
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    content: &'static str,
    replay: bool,
) -> (WorkflowV2Result, usize) {
    let execution = WorkflowV2CallExecution {
        call: call.clone(),
        input: json!({ "source_data": [{
            "item_id": call.id, "canonical_task_ids": ["TASK-001"],
            "task": "Post-review remediation for TASK-001. Findings (verbatim):\n[f1]",
            "target_files": ["owned.txt"], "focused_verification": [], "artifact_requirements": [],
            "work_type": "implementation",
        }] }),
        depends_on: vec![],
    };
    let items = fanout_items_for_call(&execution, &f.v2)
        .unwrap()
        .into_iter()
        .map(|item| (item, edits(content)))
        .collect();
    let audit = Some(AuditScript {
        flagged: vec![],
        dispositions: BTreeMap::new(),
    });
    let (result, prompts) = f
        .wave_on(store, call.clone(), items, audit, &[], &[], replay)
        .await;
    (result, prompts.len())
}

/// Save `result` for `call` as the host does: the record's next attempt.
fn record(
    f: &Fixture,
    store: &WorkflowV2ResultStore,
    call: WorkflowV2HostCall,
    result: WorkflowV2Result,
) {
    let attempt = store
        .load_call_record(&call.id)
        .unwrap()
        .map_or(1, |record| record.attempt + 1);
    let input = format!("input-{}", call.id);
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            f.run.clone(),
            call,
            attempt,
            input,
            result,
            vec![],
        ))
        .unwrap();
}

/// One session's fix: its wave, then its record. Agents dispatched.
async fn fix(f: &Fixture, store: &WorkflowV2ResultStore, ordinal: u64, replay: bool) -> usize {
    let call = fix_call(ordinal);
    let (result, dispatched) = wave(f, store, &call, "remediated\n", replay).await;
    record(f, store, call, result);
    dispatched
}

fn verdict_result() -> WorkflowV2Result {
    let mut verified = WorkflowV2Result::accepted("findings resolved");
    verified.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "inspected owned.txt",
    ));
    verified
}

/// The session's verifier, as the host answers it: a recorded verdict of
/// the round -- under its own id or the drifted sibling's -- replays when
/// it vouches for this session's fix; otherwise the verifier runs and its
/// verdict is recorded. Whether it ran.
fn verify(f: &Fixture, store: &WorkflowV2ResultStore) -> bool {
    let records = store.load_call_records().unwrap();
    let replayable = [VERDICT, SIBLING_VERDICT].into_iter().find(|id| {
        records.iter().any(|record| {
            record.call.id == *id
                && !store.in_session(id)
                && record.status == WorkflowV2Status::Accepted
                && verdict_vouches_for_session_fix(record, &records, store)
        })
    });
    if let Some(id) = replayable {
        store.note_session_call(id);
        return false;
    }
    record(f, store, verdict_call(VERDICT), verdict_result());
    true
}

fn new_session(f: &Fixture) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(f.v2.root().to_path_buf())
}

fn replayed_from(store: &WorkflowV2ResultStore) -> Option<String> {
    store.fix_replayed_from(&remediation_round_key(&fix_call(OWN_FIX)).unwrap())
}

/// The earlier decomposition: the sibling fix ran and landed, and its
/// verdict judged it.
async fn earlier_decomposition(f: &Fixture) {
    assert_eq!(fix(f, &f.v2, SIBLING_FIX, false).await, 1);
    record(f, &f.v2, verdict_call(SIBLING_VERDICT), verdict_result());
}

/// Resume `sessions` times; each replays the fix without dispatch and asks
/// the verifier. The sessions in which the verifier ran.
async fn resumes(f: &Fixture, sessions: usize) -> Vec<usize> {
    let mut ran = Vec::new();
    for session in 0..sessions {
        let store = new_session(f);
        assert_eq!(fix(f, &store, OWN_FIX, true).await, 0, "session {session}");
        if verify(f, &store) {
            ran.push(session);
        } else {
            assert_eq!(
                replayed_from(&store).as_deref(),
                Some(fix_call(OWN_FIX).id.as_str()),
                "session {session}: the verdict follows the fix's own record"
            );
        }
    }
    ran
}

/// The live shape: a drift replay whose record names no execution (as the
/// build before Issue-111 left it), then resumes. The first resume cannot
/// prove the refile is the sibling's execution and asks the verifier again;
/// every later one replays that verdict.
#[tokio::test]
async fn a_verdict_asked_again_after_a_drift_replay_is_asked_once() {
    let f = Fixture::new();
    earlier_decomposition(&f).await;
    let drift = new_session(&f);
    assert_eq!(fix(&f, &drift, OWN_FIX, true).await, 0, "the drift replays");
    assert!(
        !verify(&f, &drift),
        "the sibling's verdict follows the drift"
    );
    let path = f.v2.result_path(&fix_call(OWN_FIX).id);
    let mut on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    on_disk.as_object_mut().unwrap().remove("answered_by");
    std::fs::write(&path, serde_json::to_vec(&on_disk).unwrap()).unwrap();
    assert_eq!(
        resumes(&f, 3).await,
        vec![0],
        "the verifier runs at most once"
    );
}

/// A session killed after refiling the sibling's answer under the fix's own
/// id, before the fix's record: the copy postdates the sibling's record, so
/// the next resume asks the verifier; the resumes after replay its verdict.
#[tokio::test]
async fn a_verdict_asked_again_after_a_killed_drift_replay_is_asked_once() {
    let f = Fixture::new();
    earlier_decomposition(&f).await;
    let killed = new_session(&f);
    let (_, dispatched) = wave(&f, &killed, &fix_call(OWN_FIX), "remediated\n", true).await;
    assert_eq!(dispatched, 0, "the drift replays");
    assert_eq!(
        resumes(&f, 3).await,
        vec![0],
        "the verifier runs at most once"
    );
}

/// The crash guard stands: once converged, the fix really runs again (the
/// tree no longer holds its patch) and the session is killed before its
/// record. The new answer replays on the next resume, and no verdict
/// recorded before it may stand for it.
#[tokio::test]
async fn a_real_re_execution_killed_before_its_record_is_verified_again() {
    let f = Fixture::new();
    earlier_decomposition(&f).await;
    let killed = new_session(&f);
    wave(&f, &killed, &fix_call(OWN_FIX), "remediated\n", true).await;
    assert_eq!(resumes(&f, 2).await, vec![0]);
    std::fs::write(f.repo.join("owned.txt"), "re-delivered by a later stage\n").unwrap();
    git(&f.repo, &["commit", "-qam", "owned.txt re-delivered"]);
    let crashed = new_session(&f);
    let (_, dispatched) = wave(
        &f,
        &crashed,
        &fix_call(OWN_FIX),
        "remediated again\n",
        false,
    )
    .await;
    assert_eq!(dispatched, 1, "the tree no longer holds the replayed patch");
    let resumed = new_session(&f);
    assert_eq!(
        fix(&f, &resumed, OWN_FIX, true).await,
        0,
        "the new answer is in the tree and replays"
    );
    assert_eq!(replayed_from(&resumed), None, "its record was never saved");
    assert!(
        verify(&f, &resumed),
        "the recorded verdict judged the old patch, not the replayed one"
    );
}

/// The crash guard on the re-derived path: the fix's record exists (the
/// verifier ran after it), and later a session wrote the identical refile
/// under the fix's own id again and was killed before its record -- as a
/// build before Issue-109 re-saved every refile. The copy postdates the
/// record, so the record is not provably the execution replayed, and no
/// verdict recorded after that record may stand.
#[tokio::test]
async fn a_refile_written_after_the_fix_record_is_verified_again() {
    let f = Fixture::new();
    earlier_decomposition(&f).await;
    let killed = new_session(&f);
    wave(&f, &killed, &fix_call(OWN_FIX), "remediated\n", true).await;
    assert_eq!(
        resumes(&f, 1).await,
        vec![0],
        "the verifier ran after the record"
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    let rewrote = new_session(&f);
    let (call, branch) = (fix_call(OWN_FIX).id, format!("{}-0", fix_call(OWN_FIX).id));
    let filed = rewrote
        .load_branch_outcome(&call, &branch)
        .unwrap()
        .unwrap();
    rewrote.save_branch_outcome(&call, &filed).unwrap();
    let resumed = new_session(&f);
    assert_eq!(
        fix(&f, &resumed, OWN_FIX, true).await,
        0,
        "the refile replays"
    );
    assert_eq!(replayed_from(&resumed), None, "written after its record");
    assert!(
        verify(&f, &resumed),
        "no verdict may follow an unproven answer"
    );
}

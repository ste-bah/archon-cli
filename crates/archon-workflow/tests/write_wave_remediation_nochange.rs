//! Review remediation replayed under its own id on a resume, through the
//! production write-wave seam with real Git writes.
//!
//! A write that changed nothing stands with no apply manifest only on the
//! host's positive record of that; a claimed patch with no receipt runs
//! again. A recorded verdict follows its fix only while that fix replayed:
//! the session re-saves the fix's record under a new attempt, as the host
//! does, and the fix's past must survive that.
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

const FIX: &str = "review-remediate-task-001-1-31";
const VERDICT: &str = "verification-wave-review-verify-task-001-1-32";

fn contract_call(id: &str, stage: &str, write: Option<WorkflowV2WriteMode>) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions {
        item_kind: Some("implementation".into()),
        task: Some("Post-review remediation.".into()),
        target_files_from_item: write.is_some(),
        ..Default::default()
    };
    options.extra.insert(
        "remediationContract".into(),
        json!({ "version": 1, "stage": stage, "taskId": "TASK-001", "round": 1,
            "maxRounds": 2, "sourceReduceCallIds": ["adversarial-review-reduce"] }),
    );
    WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: write,
        options,
    }
}

fn fix_call() -> WorkflowV2HostCall {
    contract_call(FIX, "remediate", Some(WorkflowV2WriteMode::Worktree))
}

fn edits(content: &'static str) -> Edits {
    Edits {
        files: vec![("owned.txt", content)],
        report: vec!["owned.txt"],
        via_adapter: false,
    }
}

/// One session's wave for the fix, saved as the host saves it: each run of
/// the call is the record's next attempt. The number of agents dispatched.
async fn run(f: &Fixture, store: &WorkflowV2ResultStore, edits: Edits, replay: bool) -> usize {
    let call = fix_call();
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
        .map(|item| (item, edits.clone()))
        .collect();
    let audit = Some(AuditScript {
        flagged: vec![],
        dispositions: BTreeMap::new(),
    });
    let (result, prompts) = f
        .wave_on(store, call.clone(), items, audit, &[], &[], replay)
        .await;
    let attempt = store
        .load_call_record(&call.id)
        .unwrap()
        .map_or(1, |record| record.attempt + 1);
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            f.run.clone(),
            call,
            attempt,
            format!("input-{FIX}"),
            result,
            vec![],
        ))
        .unwrap();
    prompts.len()
}

/// The earlier session's verifier accepted the fix, after it finished.
fn record_verdict(f: &Fixture) {
    let mut verified = WorkflowV2Result::accepted("findings resolved");
    verified.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "inspected owned.txt",
    ));
    f.v2.save_call_record(&WorkflowV2CallRecord::new(
        f.run.clone(),
        contract_call(VERDICT, "verify", None),
        1,
        "input-verdict".into(),
        verified,
        vec![],
    ))
    .unwrap();
}

fn verdict_replays(session: &WorkflowV2ResultStore) -> bool {
    let records = session.load_call_records().unwrap();
    let verdict = records
        .iter()
        .find(|record| record.call.id == VERDICT)
        .expect("verdict recorded");
    verdict_vouches_for_session_fix(verdict, &records, session)
}

fn manifest_path(f: &Fixture) -> std::path::PathBuf {
    f.store.run_dir(&f.run).join(format!(
        "write-coordination/stages/{FIX}/manifests/{FIX}-0.json"
    ))
}

fn branch_data(store: &WorkflowV2ResultStore) -> serde_json::Value {
    let outcome = store
        .load_branch_outcome(FIX, &format!("{FIX}-0"))
        .unwrap()
        .expect("branch outcome recorded");
    outcome.result.expect("branch result").data
}

fn new_session(f: &Fixture) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(f.v2.root().to_path_buf())
}

fn head(f: &Fixture) -> String {
    git(&f.repo, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn a_remediation_that_changed_nothing_replays_without_its_manifest() {
    let f = Fixture::new();
    assert_eq!(run(&f, &f.v2, edits("baseline\n"), false).await, 1);
    let data = branch_data(&f.v2);
    assert_eq!(data["patch_landed"], json!(false), "{data:#?}");
    assert_eq!(
        data["delivery"]["repository_changed"],
        json!(false),
        "{data:#?}"
    );
    std::fs::remove_file(manifest_path(&f)).unwrap();
    let before = head(&f);
    let session = new_session(&f);
    assert_eq!(run(&f, &session, edits("baseline\n"), true).await, 0);
    assert_eq!(head(&f), before, "a replay writes nothing");
    let key = remediation_round_key(&fix_call()).unwrap();
    assert_eq!(session.fix_replayed_from(&key).as_deref(), Some(FIX));
}

#[tokio::test]
async fn a_claimed_patch_whose_manifest_is_missing_runs_again() {
    let f = Fixture::new();
    assert_eq!(run(&f, &f.v2, edits("remediated\n"), false).await, 1);
    assert_eq!(branch_data(&f.v2)["patch_landed"], json!(true));
    std::fs::remove_file(manifest_path(&f)).unwrap();
    let session = new_session(&f);
    assert_eq!(
        run(&f, &session, edits("remediated\n"), false).await,
        1,
        "no receipt for the claimed patch: ambiguous"
    );
    let key = remediation_round_key(&fix_call()).unwrap();
    assert_eq!(session.fix_replayed_from(&key), None);
}

/// The fix's deliverable was ignored, so its manifest is `skipped_ignored`:
/// nothing entered the tree to check. The fix replays under its own id and
/// the verdict that judged it follows it.
#[tokio::test]
async fn a_verdict_on_a_replayed_skipped_ignored_fix_replays() {
    let f = Fixture::new();
    assert_eq!(run(&f, &f.v2, edits("remediated\n"), false).await, 1);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path(&f)).unwrap()).unwrap();
    manifest["status"] = json!({ "status": "skipped_ignored" });
    std::fs::write(manifest_path(&f), serde_json::to_vec(&manifest).unwrap()).unwrap();
    record_verdict(&f);
    let session = new_session(&f);
    assert_eq!(run(&f, &session, edits("remediated\n"), true).await, 0);
    assert!(
        verdict_replays(&session),
        "the fix it judged replayed unchanged"
    );
}

#[tokio::test]
async fn a_verdict_whose_fix_ran_again_does_not_replay() {
    let f = Fixture::new();
    assert_eq!(run(&f, &f.v2, edits("remediated\n"), false).await, 1);
    record_verdict(&f);
    std::fs::write(f.repo.join("owned.txt"), "re-delivered by a later stage\n").unwrap();
    git(&f.repo, &["commit", "-qam", "owned.txt re-delivered"]);
    let session = new_session(&f);
    assert_eq!(run(&f, &session, edits("remediated\n"), false).await, 1);
    assert!(
        !verdict_replays(&session),
        "a fresh fix is not what the recorded verdict judged"
    );
}

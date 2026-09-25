//! Review remediation replay through the production write-wave seam: the
//! host's item builder (`fanout_items_for_call`), its write-path stamping,
//! the per-item repository audit and the apply manifest, with real Git
//! writes. A dispatch that must not happen panics.
//!
//! The prelude names each remediation call `slug(label)-<global ordinal>`, so
//! a resume in which an earlier task took fewer calls issues the same work
//! under a shifted id. A resume is a new session: a new result-store instance
//! over the same run directory.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeMap;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::call_data::fanout_items_for_call;
use archon_workflow::*;
use serde_json::json;
use support::{AuditScript, Edits, Fixture, git};

const TASK: &str = "TASK-001";

fn call(round: u64, ordinal: u64) -> WorkflowV2HostCall {
    labeled(
        &format!("review-remediate-task-001-{round}"),
        round,
        ordinal,
    )
}

/// A call filed under `label`. Before the prelude kept rounds in its labels,
/// a long unit key was cut at 40 characters and two rounds shared one.
fn labeled(label: &str, round: u64, ordinal: u64) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions {
        item_kind: Some("implementation".into()),
        task: Some("Post-review remediation.".into()),
        target_files_from_item: true,
        ..Default::default()
    };
    options.extra.insert(
        "remediationContract".into(),
        json!({ "version": 1, "stage": "remediate", "taskId": TASK, "round": round, "maxRounds": 2,
            "sourceReduceCallIds": ["adversarial-review-reduce"] }),
    );
    WorkflowV2HostCall {
        id: format!("{label}-{ordinal}"),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options,
    }
}

/// The branch the host builds for the prelude's `agent({write: true})`: the
/// item carries the call id, the prompt carries the findings verbatim.
fn branches(
    f: &Fixture,
    call: &WorkflowV2HostCall,
    findings: &str,
    edits: Edits,
) -> Vec<(WorkflowV2FanoutItem, Edits)> {
    let execution = WorkflowV2CallExecution {
        call: call.clone(),
        input: json!({ "source_data": [{
            "item_id": call.id, "canonical_task_ids": [TASK],
            "task": format!("Post-review remediation for {TASK}. Findings (verbatim):\n{findings}"),
            "target_files": ["owned.txt"], "focused_verification": [], "artifact_requirements": [],
            "work_type": "implementation",
        }] }),
        depends_on: vec![],
    };
    fanout_items_for_call(&execution, &f.v2)
        .unwrap()
        .into_iter()
        .map(|item| (item, edits.clone()))
        .collect()
}

fn fix() -> Edits {
    Edits {
        files: vec![("owned.txt", "remediated\n")],
        report: vec!["owned.txt"],
        via_adapter: false,
    }
}

fn audit() -> Option<AuditScript> {
    Some(AuditScript {
        flagged: vec![],
        dispositions: BTreeMap::new(),
    })
}

/// A session's wave for `call`, and the host's record of the call.
async fn run(
    f: &Fixture,
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    findings: &str,
    edits: Edits,
    replay: bool,
) -> (WorkflowV2Result, usize) {
    run_rejecting(f, store, call, findings, edits, replay, false).await
}

/// `run`; with `reject`, the branch writes and then returns evidence the
/// host refuses -- a round that answered, and was rejected.
async fn run_rejecting(
    f: &Fixture,
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    findings: &str,
    edits: Edits,
    replay: bool,
    reject: bool,
) -> (WorkflowV2Result, usize) {
    let items = branches(f, call, findings, edits);
    let branch = format!("{}-0", call.id);
    let rejecting: Vec<&str> = if reject {
        vec![branch.as_str()]
    } else {
        vec![]
    };
    let (result, prompts) = f
        .wave_on(store, call.clone(), items, audit(), &[], &rejecting, replay)
        .await;
    let record = WorkflowV2CallRecord::new(
        f.run.clone(),
        call.clone(),
        1,
        format!("input-{}", call.id),
        result.clone(),
        vec![],
    );
    store.save_call_record(&record).unwrap();
    (result, prompts.len())
}

fn new_session(f: &Fixture) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(f.v2.root().to_path_buf())
}

fn outcome(store: &WorkflowV2ResultStore, call: &WorkflowV2HostCall) -> WorkflowV2BranchOutcome {
    store
        .load_branch_outcome(&call.id, &format!("{}-0", call.id))
        .unwrap()
        .expect("branch outcome recorded")
}

fn tree(f: &Fixture) -> (String, String) {
    (
        git(&f.repo, &["rev-parse", "HEAD"]),
        std::fs::read_to_string(f.repo.join("owned.txt")).unwrap(),
    )
}

fn set_manifest_status(f: &Fixture, call: &WorkflowV2HostCall, status: serde_json::Value) {
    let branch = format!("{}-0", call.id);
    let path = f.store.run_dir(&f.run).join(format!(
        "write-coordination/stages/{}/manifests/{branch}.json",
        call.id
    ));
    let mut manifest = f.manifest(&call.id, &branch);
    manifest["status"] = status;
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

/// (1) Landed, applied: the shifted call replays with no dispatch, writes
/// nothing, and files the replayed outcome under its own id.
#[tokio::test]
async fn a_landed_remediation_replays_under_a_shifted_ordinal() {
    let f = Fixture::new();
    let (first, dispatched) = run(&f, &f.v2, &call(1, 31), "[f1]", fix(), false).await;
    assert_eq!(first.status, WorkflowV2Status::Accepted, "{first:#?}");
    assert_eq!(dispatched, 1);
    assert_eq!(
        f.manifest(&call(1, 31).id, "review-remediate-task-001-1-31-0")["status"]["status"],
        json!("applied")
    );
    let landed = tree(&f);

    let resumed = new_session(&f);
    let (replayed, dispatched) = run(&f, &resumed, &call(1, 29), "[f1]", fix(), true).await;
    assert_eq!(dispatched, 0);
    assert_eq!(replayed.status, WorkflowV2Status::Accepted, "{replayed:#?}");
    assert_eq!(tree(&f), landed, "a replay writes nothing");
    let refiled = outcome(&resumed, &call(1, 29));
    assert_eq!(refiled.item_id, "review-remediate-task-001-1-29-0");
    let data = &refiled.result.unwrap().data;
    assert_eq!(data["patch_landed"], json!(true), "{data:#?}");
    assert_eq!(data["branch_id"], json!("review-remediate-task-001-1-29-0"));
}

/// (2) A record that claims a patch the tree does not carry never answers a
/// shifted call: `skipped_ignored` (the bytes live only in the sibling's run
/// archive, never in the tree) and a patch that never went through apply
/// are both dispatched again. A record that claims NO patch replays as it
/// is: it says nothing landed, the script counts the round as a no-patch
/// round exactly as it did, and no missing work is hidden behind it.
#[tokio::test]
async fn a_claimed_patch_the_tree_does_not_carry_is_dispatched_again() {
    for status in [
        json!({ "status": "skipped_ignored" }),
        json!({ "status": "pending_apply" }),
    ] {
        let f = Fixture::new();
        run(&f, &f.v2, &call(1, 31), "[f1]", fix(), false).await;
        set_manifest_status(&f, &call(1, 31), status.clone());
        let resumed = new_session(&f);
        let (_, dispatched) = run(&f, &resumed, &call(1, 29), "[f1]", fix(), false).await;
        assert_eq!(
            dispatched, 1,
            "{status}: the claimed patch is not in the tree"
        );
    }
}

#[tokio::test]
async fn a_remediation_that_landed_nothing_replays_as_landing_nothing() {
    let f = Fixture::new();
    let nothing = Edits {
        files: vec![("owned.txt", "baseline\n")],
        report: vec!["owned.txt"],
        via_adapter: false,
    };
    let (first, _) = run(&f, &f.v2, &call(1, 31), "[f1]", nothing.clone(), false).await;
    let recorded = outcome(&f.v2, &call(1, 31));
    assert_eq!(
        recorded.result.as_ref().unwrap().data["patch_landed"],
        json!(false),
        "{first:#?}"
    );
    assert!(
        matches!(
            recorded.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ),
        "{recorded:#?}"
    );
    let before = tree(&f);
    let resumed = new_session(&f);
    let (again, dispatched) = run(&f, &resumed, &call(1, 29), "[f1]", nothing, true).await;
    assert_eq!(dispatched, 0, "{again:#?}");
    assert_eq!(tree(&f), before);
    let refiled = outcome(&resumed, &call(1, 29));
    assert_eq!(
        refiled.result.unwrap().data["patch_landed"],
        json!(false),
        "the replay still says nothing landed"
    );
}

/// (3) Round 1 was rejected and round 2 accepted. On a resume that shifts
/// both, round 1 replays as history (still rejected) and round 2 replays
/// as the landed fix: nothing is dispatched and the tree is untouched.
#[tokio::test]
async fn a_superseded_rejected_round_and_its_accepted_successor_replay() {
    let f = Fixture::new();
    let (round_one, dispatched) =
        run_rejecting(&f, &f.v2, &call(1, 37), "[f1]", fix(), false, true).await;
    assert_eq!(dispatched, 1);
    assert_eq!(
        round_one.status,
        WorkflowV2Status::NeedsReview,
        "{round_one:#?}"
    );
    let rejected = outcome(&f.v2, &call(1, 37));
    assert!(rejected.result.is_some(), "a rejected round still answered");
    let (round_two, _) = run(&f, &f.v2, &call(2, 39), "[f1]", fix(), false).await;
    assert_eq!(
        round_two.status,
        WorkflowV2Status::Accepted,
        "{round_two:#?}"
    );
    let landed = tree(&f);

    let resumed = new_session(&f);
    let (history, dispatched) =
        run_rejecting(&f, &resumed, &call(1, 35), "[f1]", fix(), true, true).await;
    assert_eq!(dispatched, 0);
    assert_eq!(history.status, round_one.status, "{history:#?}");
    assert_eq!(outcome(&resumed, &call(1, 35)).status, rejected.status);
    let (accepted, dispatched) = run(&f, &resumed, &call(2, 37), "[f1]", fix(), true).await;
    assert_eq!(dispatched, 0);
    assert_eq!(accepted.status, WorkflowV2Status::Accepted, "{accepted:#?}");
    assert_eq!(tree(&f), landed);
}

/// (4) A record this session wrote, or already replayed for another call,
/// never answers a call: the same question asked twice in one run runs.
#[tokio::test]
async fn a_record_of_the_current_session_answers_no_other_call() {
    let f = Fixture::new();
    run(&f, &f.v2, &call(1, 31), "[f1]", fix(), false).await;
    let (_, dispatched) = run(&f, &f.v2, &call(1, 33), "[f1]", fix(), false).await;
    assert_eq!(dispatched, 1, "written this session: not an answer");

    let g = Fixture::new();
    run(&g, &g.v2, &call(1, 31), "[f1]", fix(), false).await;
    let resumed = new_session(&g);
    let (_, dispatched) = run(&g, &resumed, &call(1, 29), "[f1]", fix(), true).await;
    assert_eq!(dispatched, 0, "the earlier session's record answers once");
    let (_, dispatched) = run(&g, &resumed, &call(1, 33), "[f1]", fix(), false).await;
    assert_eq!(dispatched, 1, "and only once");
}

/// (5) Two rounds under one cut label: the item identity holds no round (the
/// prompt is the same every round), so only the contract tells them apart.
/// Round 1 under a shifted ordinal is never answered by round 2's landed fix.
#[tokio::test]
async fn one_round_is_never_answered_by_another_under_a_cut_label() {
    const CUT: &str = "review-remediate-cross-task-001-task-00";
    let f = Fixture::new();
    let (round_two, _) = run(&f, &f.v2, &labeled(CUT, 2, 31), "[f1]", fix(), false).await;
    assert_eq!(
        round_two.status,
        WorkflowV2Status::Accepted,
        "{round_two:#?}"
    );
    let resumed = new_session(&f);
    let (_, dispatched) = run(&f, &resumed, &labeled(CUT, 1, 29), "[f1]", fix(), false).await;
    assert_eq!(
        dispatched, 1,
        "round 1 must run: round 2's fix is not its answer"
    );
}

/// A task universe whose task declares one more file than the item does:
/// the write path stamps it into the item's `target_files` before reuse is
/// decided, so the stamped input differs from the authored one the sibling
/// was recorded under (the live wf-0ddadd81 shape: a contract target
/// appended to every review-remediation write).
fn declaring_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: TASK.into(),
            source_path: format!("tasks/{TASK}.md"),
            files_expected_to_change: vec!["owned.txt".into(), "other.txt".into()],
            ..Default::default()
        }],
    }
}

/// (6) The host stamps non-volatile content into the item before the reuse
/// split. Drift still replays -- a shifted round, a later-shifted round and a
/// cross-task unit -- because the sibling is matched on the AUTHORED input
/// rebased to its ordinal, never on the stamped one.
#[tokio::test]
async fn a_shifted_remediation_replays_when_the_host_stamps_extra_targets() {
    for (label, ordinals) in [
        ("review-remediate-task-001-1", (31, 29)),
        ("review-remediate-task-001-2", (40, 33)),
        (
            "review-remediate-cross-task-001-task-00-8a1b2c3d-1",
            (57, 51),
        ),
    ] {
        let mut f = Fixture::new();
        f.universe = Some(declaring_universe());
        let round = if label.ends_with("-2") { 2 } else { 1 };
        let (first, dispatched) = run(
            &f,
            &f.v2,
            &labeled(label, round, ordinals.0),
            "[f1]",
            fix(),
            false,
        )
        .await;
        assert_eq!(
            first.status,
            WorkflowV2Status::Accepted,
            "{label}: {first:#?}"
        );
        assert_eq!(dispatched, 1);
        let targets = &outcome(&f.v2, &labeled(label, round, ordinals.0));
        assert!(targets.item_input_hash.is_some());
        let landed = tree(&f);
        let resumed = new_session(&f);
        let (replayed, dispatched) = run(
            &f,
            &resumed,
            &labeled(label, round, ordinals.1),
            "[f1]",
            fix(),
            true,
        )
        .await;
        assert_eq!(dispatched, 0, "{label}");
        assert_eq!(
            replayed.status,
            WorkflowV2Status::Accepted,
            "{label}: {replayed:#?}"
        );
        assert_eq!(tree(&f), landed);
    }
}

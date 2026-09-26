//! Issue-113 through the production write-wave seam: a review-remediation fix
//! regenerates a declared GITIGNORED project artifact in its worktree; the
//! landing must put it where the verifier and acceptance read it -- under the
//! project root, which is not the repository -- and a resume must replay the
//! landing on that copy, as the run left it.
//!
//! Live on wf-0ddadd81: the fix's manifest said `skipped_ignored`, the bytes
//! were archived as a run artifact, and the verifier judged the stale copy
//! under the project root. No round could ever pass.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use archon_workflow::task_universe::{
    WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
};
use archon_workflow::v2::call_data::fanout_items_for_call;
use archon_workflow::v2::deliverable_contract::ContractRoots;
use archon_workflow::v2::project_artifact_stamping::stamp_project_artifact_paths;
use archon_workflow::*;
use serde_json::json;
use support::{AuditScript, Edits, Fixture, git};

const TASK: &str = "TASK-001";
const PINE: &str = ".archon/lab/strategies/S1/pine/S1-indicator.pine";

fn call(round: u64, ordinal: u64) -> WorkflowV2HostCall {
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
        id: format!("review-remediate-task-001-{round}-{ordinal}"),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options,
    }
}

fn regenerate(content: &'static str) -> Edits {
    Edits {
        files: vec![(PINE, content)],
        report: vec![PINE],
        via_adapter: false,
    }
}

/// The fixture repository, ignoring project state exactly as the live
/// target repository does, under a task universe that declares the
/// artifact as the task's deliverable.
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: TASK.into(),
            source_path: format!("tasks/{TASK}.md"),
            deliverable_contracts: vec![WorkflowV2DeliverableContract {
                kind: "artifact".into(),
                artifact_path: PINE.into(),
                min_instances: 1,
                ..Default::default()
            }],
            ..Default::default()
        }],
    });
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-qm", "ignore project state"]);
    f
}

fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
}

/// The path the host stamps into a verifier's prompt for the deliverable.
fn verifier_path(f: &Fixture) -> String {
    let mut object = json!({ "artifact_requirements": [PINE] })
        .as_object()
        .unwrap()
        .clone();
    let stamped = stamp_project_artifact_paths(&mut object, &project_root(f).display().to_string());
    stamped[0]["absolute_path"].as_str().unwrap().to_string()
}

async fn run(
    f: &Fixture,
    store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
    edits: Edits,
    replay: bool,
) -> (WorkflowV2Result, usize) {
    let execution = WorkflowV2CallExecution {
        call: call.clone(),
        input: json!({ "source_data": [{
            "item_id": call.id, "canonical_task_ids": [TASK],
            "task": format!("Post-review remediation for {TASK}: regenerate the stale artifact."),
            "target_files": [PINE], "focused_verification": [], "artifact_requirements": [],
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

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[tokio::test]
async fn a_regenerated_ignored_deliverable_lands_where_it_is_verified_and_replays() {
    let f = fixture();
    let verified = PathBuf::from(verifier_path(&f));
    assert_eq!(verified, project_root(&f).join(PINE));
    std::fs::create_dir_all(verified.parent().unwrap()).unwrap();
    std::fs::write(&verified, "legacy render").unwrap();
    let head = git(&f.repo, &["rev-parse", "HEAD"]);

    let fix = call(2, 55);
    let (landed, dispatched) = run(&f, &f.v2, &fix, regenerate("regenerated"), false).await;
    assert_eq!(landed.status, WorkflowV2Status::Accepted, "{landed:#?}");
    assert_eq!(dispatched, 1);

    // The verifier's path, and acceptance's (project first), carry the fix.
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "regenerated");
    let roots = ContractRoots::new(
        project_root(&f).display().to_string(),
        Some(f.repo.to_str().unwrap()),
    );
    assert_eq!(roots.resolve(PINE), verified);
    // The repository never sees it: no commit, no copy.
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), head);
    assert!(!f.repo.join(PINE).exists());

    let branch = format!("{}-0", fix.id);
    let manifest = f.manifest(&fix.id, &branch);
    assert_eq!(manifest["status"]["status"], json!("skipped_ignored"));
    let receipt = &manifest["materialized"][PINE];
    assert_eq!(
        receipt["destination"],
        json!(verified.display().to_string())
    );
    assert_eq!(receipt["pre_hash"], json!(blake3_hex(b"legacy render")));
    assert_eq!(receipt["post_hash"], json!(blake3_hex(b"regenerated")));
    assert_eq!(receipt["sequence"], json!(1));

    // Resume: the landing stands on the copy it made; nothing is dispatched.
    let resumed = new_session(&f);
    let (replayed, dispatched) = run(&f, &resumed, &fix, regenerate("regenerated"), true).await;
    assert_eq!(dispatched, 0);
    assert_eq!(replayed.status, WorkflowV2Status::Accepted, "{replayed:#?}");
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "regenerated");

    // A copy changed outside the run is not what the fix's verdict judged:
    // the fix runs again and lands again, as the run's next copy.
    std::fs::write(&verified, "edited by hand").unwrap();
    let resumed = new_session(&f);
    let (rerun, dispatched) = run(&f, &resumed, &fix, regenerate("regenerated"), false).await;
    assert_eq!(dispatched, 1, "{rerun:#?}");
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "regenerated");
    let receipt = &f.manifest(&fix.id, &branch)["materialized"][PINE];
    assert_eq!(receipt["pre_hash"], json!(blake3_hex(b"edited by hand")));
    let resumed = new_session(&f);
    let (_, dispatched) = run(&f, &resumed, &fix, regenerate("regenerated"), true).await;
    assert_eq!(dispatched, 0, "the re-landed copy is the run's last");
}

#[tokio::test]
async fn an_earlier_round_replays_on_the_later_rounds_copy() {
    let f = fixture();
    let verified = project_root(&f).join(PINE);
    let first = call(1, 51);
    let second = call(2, 55);
    let (one, _) = run(&f, &f.v2, &first, regenerate("round one"), false).await;
    let (two, _) = run(&f, &f.v2, &second, regenerate("round two"), false).await;
    assert_eq!(one.status, WorkflowV2Status::Accepted, "{one:#?}");
    assert_eq!(two.status, WorkflowV2Status::Accepted, "{two:#?}");
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "round two");

    // Round two's copy is the run's own later landing, so round one still
    // stands on it -- a resume replays both rounds, as the run left them.
    let resumed = new_session(&f);
    let (replayed, dispatched) = run(&f, &resumed, &first, regenerate("round one"), true).await;
    assert_eq!(dispatched, 0);
    assert_eq!(replayed.status, WorkflowV2Status::Accepted, "{replayed:#?}");
    let (_, dispatched) = run(&f, &resumed, &second, regenerate("round two"), true).await;
    assert_eq!(dispatched, 0);
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "round two");
}

/// A declared ignored target the task universe does not name as a
/// deliverable is never placed: it stays the run artifact it always was.
#[tokio::test]
async fn an_ignored_target_no_task_declares_as_a_deliverable_stays_a_run_artifact() {
    let mut f = fixture();
    f.universe = None;
    let fix = call(1, 51);
    let (landed, _) = run(&f, &f.v2, &fix, regenerate("regenerated"), false).await;
    assert_eq!(landed.status, WorkflowV2Status::Accepted, "{landed:#?}");
    assert!(!project_root(&f).join(PINE).exists());
    let manifest = f.manifest(&fix.id, &format!("{}-0", fix.id));
    assert_eq!(manifest["status"]["status"], json!("skipped_ignored"));
    assert!(manifest.get("materialized").is_none(), "{manifest:#}");
}

/// Defect 2: an ORDINARY write -- no remediation contract -- that placed a
/// copy is reused only while the copy stands, like a remediation write.
#[tokio::test]
async fn an_ordinary_write_is_reused_only_while_its_copy_stands() {
    let f = fixture();
    let verified = project_root(&f).join(PINE);
    let implement = WorkflowV2HostCall {
        id: "implement-task-001-1".into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".into()),
            task: Some("Implement the task.".into()),
            target_files_from_item: true,
            ..Default::default()
        },
    };
    let (landed, dispatched) = run(&f, &f.v2, &implement, regenerate("first"), false).await;
    assert_eq!(landed.status, WorkflowV2Status::Accepted, "{landed:#?}");
    assert_eq!(dispatched, 1);
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "first");

    let (_, dispatched) = run(&f, &new_session(&f), &implement, regenerate("first"), true).await;
    assert_eq!(dispatched, 0, "the copy stands, the write is reused");

    std::fs::write(&verified, "edited by hand").unwrap();
    let (_, dispatched) = run(&f, &new_session(&f), &implement, regenerate("first"), false).await;
    assert_eq!(
        dispatched, 1,
        "a copy changed outside the run is not reused"
    );
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "first");
}

/// A later persist of the same item -- a no-op replay's manifest, which
/// carries no copies -- must not erase that the item ever placed one: with
/// the copy then deleted, a resume must refuse the landing, never credit it.
#[tokio::test]
async fn a_no_op_repersist_cannot_hide_a_deleted_copy() {
    let f = fixture();
    let verified = project_root(&f).join(PINE);
    let fix = call(2, 55);
    let (landed, _) = run(&f, &f.v2, &fix, regenerate("regenerated"), false).await;
    assert_eq!(landed.status, WorkflowV2Status::Accepted, "{landed:#?}");

    // What persist_manifest writes when the item is captured again with
    // nothing to place: a no-op manifest with no ignored bytes, no hashes for
    // the deliverable and no `materialized` receipts.
    let branch = format!("{}-0", fix.id);
    let path = f.store.run_dir(&f.run).join(format!(
        "write-coordination/stages/{}/manifests/{branch}.json",
        fix.id
    ));
    let mut manifest = f.manifest(&fix.id, &branch);
    for field in ["materialized", "skipped_ignored", "destination_baselines"] {
        manifest.as_object_mut().unwrap().remove(field);
    }
    for field in ["pre_hashes", "post_hashes"] {
        manifest[field].as_object_mut().unwrap().remove(PINE);
    }
    manifest["status"] = json!({ "status": "idempotent_noop" });
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let (_, dispatched) = run(&f, &new_session(&f), &fix, regenerate("regenerated"), true).await;
    assert_eq!(dispatched, 0, "the copy still stands");

    std::fs::remove_file(&verified).unwrap();
    let (_, dispatched) = run(&f, &new_session(&f), &fix, regenerate("regenerated"), false).await;
    assert_eq!(
        dispatched, 1,
        "the ledger still knows the copy; its loss refuses"
    );
    assert_eq!(std::fs::read_to_string(&verified).unwrap(), "regenerated");
}

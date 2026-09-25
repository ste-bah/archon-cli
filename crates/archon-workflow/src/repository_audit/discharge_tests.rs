use super::*;

use crate::repository_audit::contract::{AuditContract, AuditRecord, RequiredAction};
use crate::repository_audit::ledger::AuditLedger;
use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind};
use crate::v2::result_store::WorkflowV2DispatchedItem;
use crate::v2::scheduler::WorkflowV2BranchOutcome;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

const PATH: &str = "report.json";

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}

/// A repository with one commit holding `report.json` and one without it.
fn init_repository(root: &Path) -> (String, String) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    std::fs::write(root.join(PATH), "{}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "with the report"]);
    let with = git(root, &["rev-parse", "HEAD"]);
    git(root, &["rm", "-q", PATH]);
    git(root, &["commit", "-qm", "report deleted"]);
    let without = git(root, &["rev-parse", "HEAD"]);
    (with, without)
}

fn result(status: WorkflowV2Status, data: Value) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("answered");
    result.status = status;
    result.data = data;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "read the task",
    ));
    result
}

fn call(id: &str, stage: Option<&str>) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    if let Some(stage) = stage {
        options.extra.insert(
            "remediationContract".into(),
            serde_json::json!({ "version": 1, "stage": stage, "taskId": "TASK-X", "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
        );
    }
    WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    }
}

/// A landed stage of `task` whose applied manifest deleted the report.
fn deleted_by(run_dir: &Path, stage: &str, task: &str) {
    let store = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let item = format!("{stage}-0");
    store
        .save_branch_outcome(
            stage,
            &WorkflowV2BranchOutcome {
                item_id: item.clone(),
                role: "coder".into(),
                status: WorkflowV2Status::Accepted,
                result: Some(result(
                    WorkflowV2Status::Accepted,
                    serde_json::json!({ "canonical_task_ids": [task], "patch_landed": true }),
                )),
                error: None,
                failure_kind: None,
                item_input_hash: Some("h".into()),
                completion_evidence: Vec::new(),
            },
        )
        .unwrap();
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            call(stage, Some("remediate")),
            1,
            "input".into(),
            result(WorkflowV2Status::Accepted, serde_json::json!({})),
            Vec::new(),
        ))
        .unwrap();
    let manifest = serde_json::json!({
        "schema": "archon.workflow.patch_manifest.v1", "run_id": "run", "stage_id": stage, "item_id": item,
        "baseline_commit": "abc", "patch_path": "x.patch", "declared_target_files": [PATH],
        "changed_files": [], "created_files": [], "deleted_files": [PATH],
        "pre_hashes": {}, "post_hashes": { PATH: "deleted" }, "verify_command": null,
        "agent_artifact_path": null, "status": { "status": "applied" },
    });
    let dir = run_dir
        .join("write-coordination/stages")
        .join(stage)
        .join("manifests");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{item}.json")),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

/// A verification of TASK-X attributed by the host (dispatched items), its
/// branch stamped with the commit it judged when `judged` is given.
fn verified(run_dir: &Path, id: &str, status: WorkflowV2Status, judged: Option<&str>) {
    let label = match status {
        WorkflowV2Status::Accepted => "accepted",
        _ => "needs_review",
    };
    let mut branch = result(status, serde_json::json!({}));
    stamp_judged_commit(&mut branch, judged);
    let data = serde_json::json!({ "outcomes": [{
        "item_id": format!("{id}-0"), "status": label, "error": null, "result": branch,
    }] });
    let mut record = WorkflowV2CallRecord::new(
        "run",
        call(id, Some("verify")),
        1,
        "input".into(),
        result(status, data),
        Vec::new(),
    );
    record.dispatched_items = vec![WorkflowV2DispatchedItem {
        item_id: format!("{id}-0"),
        canonical_task_ids: vec!["TASK-X".into()],
    }];
    WorkflowV2ResultStore::new(run_dir.join("v2"))
        .save_call_record(&record)
        .unwrap();
}

fn absent_report() -> AuditReport {
    AuditReport {
        schema_version: 1,
        snapshot: "snap".into(),
        records: vec![AuditRecord {
            declared_path: PATH.into(),
            verdict: Verdict::Absent,
            equivalents: Vec::new(),
            required_action: RequiredAction::Deliver,
            reason: "absent".into(),
        }],
    }
}

/// What stays open once the ledger holds the absent report and whatever the
/// host's records discharge: the one list reuse, the gates and the finalizer
/// read.
fn open(run_dir: &Path, repository: &Path) -> Vec<String> {
    let mut ledger = AuditLedger::default();
    let report = absent_report();
    let contract = AuditContract {
        schema_version: 1,
        snapshot: "snap".into(),
        declared_paths: vec![PATH.into()],
    };
    ledger.accept(contract, report.clone()).unwrap();
    ledger.record_discharges(verified_absences(run_dir, &report, repository));
    ledger.unresolved("snap").unwrap()
}

struct Case {
    temp: tempfile::TempDir,
    with: String,
    without: String,
}

impl Case {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let (with, without) = init_repository(&temp.path().join("repo"));
        Self {
            temp,
            with,
            without,
        }
    }
    fn run(&self) -> std::path::PathBuf {
        self.temp.path().join("run")
    }
    fn open(&self) -> Vec<String> {
        open(&self.run(), &self.temp.path().join("repo"))
    }
}

#[test]
fn the_owners_deletion_then_its_accepted_verification_discharges() {
    let case = Case::new();
    deleted_by(&case.run(), "review-remediate-task-x-1-31", "TASK-X");
    verified(
        &case.run(),
        "verification-wave-review-verify-task-x-1-32",
        WorkflowV2Status::Accepted,
        Some(&case.without),
    );
    let discharges = verified_absences(
        &case.run(),
        &absent_report(),
        &case.temp.path().join("repo"),
    );
    assert_eq!(discharges.len(), 1, "{discharges:?}");
    assert_eq!(
        discharges[0].verification_call_id,
        "verification-wave-review-verify-task-x-1-32"
    );
    assert!(case.open().is_empty());
}

#[test]
fn a_path_no_task_created_then_deleted_is_never_discharged() {
    // A file still to be delivered: no landed deletion, whatever a verifier said.
    let case = Case::new();
    verified(
        &case.run(),
        "verification-wave-review-verify-task-x-1-32",
        WorkflowV2Status::Accepted,
        Some(&case.without),
    );
    assert_eq!(case.open(), vec![PATH.to_string()]);
}

#[test]
fn another_tasks_deletion_does_not_discharge_this_tasks_verification() {
    let case = Case::new();
    deleted_by(&case.run(), "review-remediate-task-y-1-31", "TASK-Y");
    verified(
        &case.run(),
        "verification-wave-review-verify-task-x-1-32",
        WorkflowV2Status::Accepted,
        Some(&case.without),
    );
    assert_eq!(case.open(), vec![PATH.to_string()]);
}

#[test]
fn a_verification_before_the_deletion_or_rejected_after_it_stays_open() {
    let before = Case::new();
    verified(
        &before.run(),
        "verification-wave-review-verify-task-x-1-30",
        WorkflowV2Status::Accepted,
        Some(&before.without),
    );
    deleted_by(&before.run(), "review-remediate-task-x-1-31", "TASK-X");
    assert_eq!(
        before.open(),
        vec![PATH.to_string()],
        "judged before the delete landed"
    );

    let newer = Case::new();
    deleted_by(&newer.run(), "review-remediate-task-x-1-31", "TASK-X");
    verified(
        &newer.run(),
        "verification-wave-review-verify-task-x-1-32",
        WorkflowV2Status::Accepted,
        Some(&newer.without),
    );
    verified(
        &newer.run(),
        "verification-wave-review-verify-task-x-2-34",
        WorkflowV2Status::NeedsReview,
        Some(&newer.without),
    );
    assert_eq!(
        newer.open(),
        vec![PATH.to_string()],
        "a newer rejected verify blocks"
    );
}

#[test]
fn a_verification_that_judged_the_file_or_recorded_no_commit_stays_open() {
    let judged_with = Case::new();
    deleted_by(&judged_with.run(), "review-remediate-task-x-1-31", "TASK-X");
    verified(
        &judged_with.run(),
        "verification-wave-review-verify-task-x-1-32",
        WorkflowV2Status::Accepted,
        Some(&judged_with.with),
    );
    assert_eq!(judged_with.open(), vec![PATH.to_string()]);

    let unstamped = Case::new();
    deleted_by(&unstamped.run(), "review-remediate-task-x-1-31", "TASK-X");
    verified(
        &unstamped.run(),
        "verification-wave-review-verify-task-x-1-32",
        WorkflowV2Status::Accepted,
        None,
    );
    assert_eq!(
        unstamped.open(),
        vec![PATH.to_string()],
        "a record older than the stamp"
    );
}

#[test]
fn a_discharge_does_not_survive_the_path_existing_again() {
    let mut ledger = AuditLedger::default();
    let contract = |snapshot: &str| AuditContract {
        schema_version: 1,
        snapshot: snapshot.into(),
        declared_paths: vec![PATH.into()],
    };
    ledger.accept(contract("snap"), absent_report()).unwrap();
    ledger.record_discharges(vec![Discharge {
        declared_path: PATH.into(),
        snapshot: "snap".into(),
        verification_call_id: "v".into(),
        base_commit: "abc".into(),
    }]);
    assert!(ledger.unresolved("snap").unwrap().is_empty());
    let mut exists = absent_report();
    exists.snapshot = "later".into();
    exists.records[0].verdict = Verdict::ExistsAsDeclared;
    exists.records[0].required_action = RequiredAction::None;
    ledger.accept(contract("later"), exists).unwrap();
    assert!(!ledger.is_discharged(PATH, "later"));
}

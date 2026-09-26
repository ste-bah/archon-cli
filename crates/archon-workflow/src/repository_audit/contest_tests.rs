//! Issue-112: a path several tasks declare, deleted by one of them in a
//! verified landing, is discharged only when every declarer's verification
//! agrees; otherwise it is contested -- open, named, never re-delivered.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::repository_audit::contract::{AuditContract, AuditRecord, RequiredAction};
use crate::repository_audit::ledger::AuditLedger;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::v2::result_store::{WorkflowV2CallRecord, WorkflowV2DispatchedItem};
use crate::v2::scheduler::WorkflowV2BranchOutcome;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Status};

const PATH: &str = "report.json";
/// Declares the report and keeps it.
const KEEPER: &str = "TASK-KEEP";
/// Granted the report by its wave and deletes it as a stray.
const DELETER: &str = "TASK-DEL";

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

struct Case {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    run: PathBuf,
    with: String,
    without: String,
}

impl Case {
    /// A repository with a commit holding the report and one without it;
    /// the universe declares the report for KEEPER when `declared`.
    fn new(declared: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "user.email", "t@example.invalid"]);
        std::fs::write(repo.join(PATH), "{}\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "with the report"]);
        let with = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["rm", "-q", PATH]);
        git(&repo, &["commit", "-qm", "report deleted"]);
        let without = git(&repo, &["rev-parse", "HEAD"]);
        let run = temp.path().join("run");
        std::fs::create_dir_all(run.join("v2")).unwrap();
        let owns = |id: &str, files: &[&str]| WorkflowV2TaskUniverseTask {
            canonical_task_id: id.into(),
            source_path: format!("{id}.md"),
            files_expected_to_change: files.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        };
        let universe = WorkflowV2TaskUniverse {
            schema_version: "t".into(),
            source_roots: vec![],
            tasks: vec![
                owns(KEEPER, if declared { &[PATH] } else { &[] }),
                owns(DELETER, &["src/snapshot.rs"]),
            ],
        };
        std::fs::write(
            run.join("v2/generated-metadata.json"),
            json!({"task_universe": universe}).to_string(),
        )
        .unwrap();
        Self {
            _temp: temp,
            repo,
            run,
            with,
            without,
        }
    }

    fn store(&self) -> WorkflowV2ResultStore {
        WorkflowV2ResultStore::new(self.run.join("v2"))
    }

    /// A landed single-task stage of `task` whose applied manifest declares
    /// the report and records it `created` or `deleted`.
    fn landed(&self, stage: &str, task: &str, deleted: bool) {
        let item = format!("{stage}-0");
        let store = self.store();
        let result = answered(
            WorkflowV2Status::Accepted,
            json!({ "canonical_task_ids": [task], "patch_landed": true }),
        );
        store
            .save_branch_outcome(
                stage,
                &WorkflowV2BranchOutcome {
                    item_id: item.clone(),
                    role: "coder".into(),
                    status: WorkflowV2Status::Accepted,
                    result: Some(result),
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
                call(stage, "remediate", task),
                1,
                "input".into(),
                answered(WorkflowV2Status::Accepted, json!({})),
                Vec::new(),
            ))
            .unwrap();
        let (created, removed): (Vec<&str>, Vec<&str>) = if deleted {
            (vec![], vec![PATH])
        } else {
            (vec![PATH], vec![])
        };
        let post = if deleted {
            json!({ PATH: "deleted" })
        } else {
            json!({ PATH: "h" })
        };
        let manifest = json!({
            "schema": "archon.workflow.patch_manifest.v1", "run_id": "run", "stage_id": stage,
            "item_id": item, "baseline_commit": "abc", "patch_path": "x.patch",
            "declared_target_files": [PATH], "changed_files": [], "created_files": created,
            "deleted_files": removed, "pre_hashes": {}, "post_hashes": post,
            "verify_command": null, "agent_artifact_path": null, "status": { "status": "applied" },
        });
        let dir = self
            .run
            .join("write-coordination/stages")
            .join(stage)
            .join("manifests");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{item}.json")), manifest.to_string()).unwrap();
    }

    /// A host-attributed verification of `task`, accepted or not, that
    /// judged `commit`.
    fn verified(&self, id: &str, task: &str, status: WorkflowV2Status, commit: &str) {
        let mut branch = answered(status, json!({}));
        super::super::discharge::stamp_judged_commit(&mut branch, Some(commit));
        let label = if status == WorkflowV2Status::Accepted {
            "accepted"
        } else {
            "needs_review"
        };
        let data = json!({ "outcomes": [{
            "item_id": format!("{id}-0"), "status": label, "error": null, "result": branch,
        }] });
        let mut record = WorkflowV2CallRecord::new(
            "run",
            call(id, "verify", task),
            1,
            "input".into(),
            answered(status, data),
            Vec::new(),
        );
        record.dispatched_items = vec![WorkflowV2DispatchedItem {
            item_id: format!("{id}-0"),
            canonical_task_ids: vec![task.into()],
        }];
        self.store().save_call_record(&record).unwrap();
    }

    /// The ledger after an assessment of `verdict` and the host's judgment.
    fn ledger(&self, verdict: Verdict) -> AuditLedger {
        let mut ledger = AuditLedger::default();
        let report = report(verdict);
        ledger
            .accept(
                AuditContract {
                    schema_version: 1,
                    snapshot: "snap".into(),
                    declared_paths: vec![PATH.into()],
                },
                report.clone(),
            )
            .unwrap();
        let (discharges, contests) = judge(&self.run, &report, &self.repo);
        ledger.record_discharges(discharges);
        ledger.record_contests(contests);
        ledger
    }
}

fn answered(status: WorkflowV2Status, data: Value) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("answered");
    result.status = status;
    result.data = data;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "read the task",
    ));
    result
}

fn call(id: &str, stage: &str, task: &str) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        json!({ "version": 1, "stage": stage, "taskId": task, "round": 1, "maxRounds": 2,
            "sourceReduceCallIds": ["r"] }),
    );
    WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    }
}

fn report(verdict: Verdict) -> AuditReport {
    let action = if verdict == Verdict::Absent {
        RequiredAction::Deliver
    } else {
        RequiredAction::None
    };
    AuditReport {
        schema_version: 1,
        snapshot: "snap".into(),
        records: vec![AuditRecord {
            declared_path: PATH.into(),
            verdict,
            equivalents: Vec::new(),
            required_action: action,
            reason: "scripted".into(),
        }],
    }
}

/// The live shape: the keeper declared and landed the report, the deleter
/// was granted it and deleted it, and only the deleter's verifier judged
/// the tree without it. Neither wins: the path is contested, open at the
/// gate with both tasks named -- and not re-delivered.
#[test]
fn a_verified_deletion_another_declarer_never_confirmed_is_contested_not_discharged() {
    let case = Case::new(true);
    case.landed("agents-3", KEEPER, false);
    case.verified(
        "verification-wave-verify-keep-4",
        KEEPER,
        WorkflowV2Status::Accepted,
        &case.with,
    );
    case.landed("review-remediate-del-1-45", DELETER, true);
    case.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &case.without,
    );
    let ledger = case.ledger(Verdict::Absent);
    assert!(
        !ledger.is_discharged(PATH, "snap"),
        "{:?}",
        ledger.discharges
    );
    assert!(ledger.is_contested(PATH, "snap"));
    assert_eq!(ledger.unresolved("snap").unwrap(), [PATH]);
    let why = ledger.describe_unresolved("snap").unwrap().join("");
    assert!(why.contains(DELETER) && why.contains(KEEPER), "{why}");
    assert!(why.contains("review-remediate-del-1-45"), "{why}");
    let contest = &ledger.contested("snap")[0];
    assert_eq!(contest.unconfirmed, [KEEPER]);
    assert_eq!(contest.state, "absent");
    // The same holds when the keeper only ever declared it in the universe.
    let unwritten = Case::new(true);
    unwritten.landed("review-remediate-del-1-45", DELETER, true);
    unwritten.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &unwritten.without,
    );
    assert!(unwritten.ledger(Verdict::Absent).is_contested(PATH, "snap"));
}

/// When every declarer's own later verification accepted the tree without
/// it, the declarers agree and the absence is discharged.
#[test]
fn every_declarer_verifying_the_tree_without_it_discharges() {
    let case = Case::new(true);
    case.landed("agents-3", KEEPER, false);
    case.landed("review-remediate-del-1-45", DELETER, true);
    case.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &case.without,
    );
    case.verified(
        "verification-wave-verify-keep-9",
        KEEPER,
        WorkflowV2Status::Accepted,
        &case.without,
    );
    let ledger = case.ledger(Verdict::Absent);
    assert!(ledger.is_discharged(PATH, "snap"));
    assert!(ledger.contested("snap").is_empty());
    assert!(ledger.unresolved("snap").unwrap().is_empty());
    // A keeper verification that judged the old tree confirms nothing.
    let stale = Case::new(true);
    stale.landed("agents-3", KEEPER, false);
    stale.landed("review-remediate-del-1-45", DELETER, true);
    stale.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &stale.without,
    );
    stale.verified(
        "verification-wave-verify-keep-9",
        KEEPER,
        WorkflowV2Status::Accepted,
        &stale.with,
    );
    assert!(stale.ledger(Verdict::Absent).is_contested(PATH, "snap"));
}

/// The sole declarer deleting its own path is Issue-104 unchanged.
#[test]
fn a_sole_declarers_verified_deletion_still_discharges() {
    let case = Case::new(false);
    case.landed("review-remediate-del-1-45", DELETER, true);
    case.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &case.without,
    );
    let ledger = case.ledger(Verdict::Absent);
    assert!(ledger.is_discharged(PATH, "snap"));
    assert!(ledger.contested("snap").is_empty());
}

/// An unverified deletion decides nothing: the path is an ordinary open
/// obligation, which re-delivery may close.
#[test]
fn an_unverified_deletion_is_neither_discharged_nor_contested() {
    let case = Case::new(true);
    case.landed("agents-3", KEEPER, false);
    case.landed("review-remediate-del-1-45", DELETER, true);
    case.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::NeedsReview,
        &case.without,
    );
    let ledger = case.ledger(Verdict::Absent);
    assert!(!ledger.is_discharged(PATH, "snap"));
    assert!(ledger.contested("snap").is_empty());
    assert_eq!(ledger.unresolved("snap").unwrap(), [PATH]);
}

/// A re-delivery after a verified deletion does not win by landing last:
/// until the deleting task's own verification accepts the re-delivered
/// tree, the path is contested though it exists.
#[test]
fn a_redelivery_after_a_verified_deletion_is_contested_until_the_deleter_accepts_it() {
    let case = Case::new(true);
    case.landed("agents-3", KEEPER, false);
    case.landed("review-remediate-del-1-45", DELETER, true);
    case.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &case.without,
    );
    case.landed("agents-3-again", KEEPER, false);
    let ledger = case.ledger(Verdict::ExistsAsDeclared);
    assert!(ledger.is_contested(PATH, "snap"));
    assert_eq!(ledger.contested("snap")[0].state, "present");
    assert_eq!(
        ledger.unresolved("snap").unwrap(),
        [PATH],
        "open with no obligation"
    );
    case.verified(
        "verification-wave-review-verify-del-2-60",
        DELETER,
        WorkflowV2Status::Accepted,
        &case.with,
    );
    let agreed = case.ledger(Verdict::ExistsAsDeclared);
    assert!(agreed.contested("snap").is_empty());
    assert!(agreed.unresolved("snap").unwrap().is_empty());
}

/// Fail closed: a declaration the host cannot read as one repository path,
/// or a task universe it cannot read at all, may be the path's; its silence
/// discharges nothing.
#[test]
fn an_unreadable_declaration_is_a_declarer_that_must_confirm() {
    let case = Case::new(false);
    let metadata = json!({"task_universe": {"schema_version": "t", "source_roots": [], "tasks": [
        {"canonical_task_id": KEEPER, "source_path": "k.md",
         "files_expected_to_change": ["<PROJECT_ROOT>/report.json"]},
        {"canonical_task_id": DELETER, "source_path": "d.md",
         "files_expected_to_change": ["src/snapshot.rs"]}]}});
    let written =
        serde_json::from_value::<WorkflowV2TaskUniverse>(metadata["task_universe"].clone());
    assert!(written.is_ok(), "{written:?}");
    std::fs::write(
        case.run.join("v2/generated-metadata.json"),
        metadata.to_string(),
    )
    .unwrap();
    case.landed("review-remediate-del-1-45", DELETER, true);
    case.verified(
        "verification-wave-review-verify-del-1-46",
        DELETER,
        WorkflowV2Status::Accepted,
        &case.without,
    );
    let ledger = case.ledger(Verdict::Absent);
    assert_eq!(ledger.contested("snap")[0].unconfirmed, [KEEPER]);
    std::fs::write(
        case.run.join("v2/generated-metadata.json"),
        "{\"task_universe\": 7}",
    )
    .unwrap();
    let unreadable = case.ledger(Verdict::Absent);
    assert!(unreadable.is_contested(PATH, "snap"));
    assert!(!unreadable.is_discharged(PATH, "snap"));
}

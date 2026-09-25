use super::*;

use crate::repository_audit::contract::{AuditContract, AuditRecord, RequiredAction};
use crate::repository_audit::ledger::AuditLedger;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

const PATH: &str = "report.json";
const VERIFY: &str = "verification-wave-review-verify-task-x-1-32";

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

/// A repository with one commit lacking `report.json` and one holding it.
fn init_repository(root: &Path) -> (String, String) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    std::fs::write(root.join("lib.rs"), "// x\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "without the report"]);
    let without = git(root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join(PATH), "{}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "with the report"]);
    let with = git(root, &["rev-parse", "HEAD"]);
    (without, with)
}

/// A run whose TASK-X declares the report, and (optionally) its verifier.
fn run(run_dir: &Path, verification: Option<(WorkflowV2Status, &str)>) {
    let v2 = run_dir.join("v2");
    std::fs::create_dir_all(&v2).unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-X".into(),
            source_path: "tasks/TASK-X.md".into(),
            files_expected_to_change: vec![format!("`{PATH}` - absent; must never exist")],
            ..Default::default()
        }],
    };
    std::fs::write(
        v2.join("generated-metadata.json"),
        serde_json::to_vec(&serde_json::json!({ "task_universe": universe })).unwrap(),
    )
    .unwrap();
    let Some((status, commit)) = verification else {
        return;
    };
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        serde_json::json!({ "version": 1, "stage": "verify", "taskId": "TASK-X", "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    let mut result = WorkflowV2Result::accepted("verified");
    result.status = status;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "read the task",
    ));
    let record = WorkflowV2CallRecord::new(
        "run",
        WorkflowV2HostCall {
            id: VERIFY.into(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        1,
        "input".into(),
        result,
        Vec::new(),
    );
    WorkflowV2ResultStore::new(&v2)
        .save_call_record(&record)
        .unwrap();
    let baseline = v2.join("baseline-tests").join(VERIFY);
    std::fs::create_dir_all(&baseline).unwrap();
    std::fs::write(
        baseline.join(format!("{VERIFY}-0.json")),
        serde_json::to_vec(&serde_json::json!({ "base_commit": commit })).unwrap(),
    )
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

/// The ledger after accepting the absent report with whatever the host's
/// records discharge: the one list reuse, the gates and the finalizer read.
fn open_after(run_dir: &Path, repository: &Path) -> Vec<String> {
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

#[test]
fn an_absence_the_owning_tasks_accepted_verifier_judged_is_discharged() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repo");
    let (without, _) = init_repository(&repository);
    run(
        &temp.path().join("run"),
        Some((WorkflowV2Status::Accepted, &without)),
    );
    let discharges = verified_absences(&temp.path().join("run"), &absent_report(), &repository);
    assert_eq!(discharges.len(), 1, "{discharges:?}");
    assert_eq!(discharges[0].verification_call_id, VERIFY);
    assert!(open_after(&temp.path().join("run"), &repository).is_empty());
}

#[test]
fn an_absence_no_accepted_verification_judged_stays_open() {
    for verification in [None, Some(WorkflowV2Status::NeedsReview)] {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("repo");
        let (without, _) = init_repository(&repository);
        run(
            &temp.path().join("run"),
            verification.map(|status| (status, without.as_str())),
        );
        assert_eq!(
            open_after(&temp.path().join("run"), &repository),
            vec![PATH.to_string()],
            "{verification:?}"
        );
    }
}

#[test]
fn an_absence_the_verifier_judged_while_the_file_existed_stays_open() {
    let temp = tempfile::tempdir().unwrap();
    let repository = temp.path().join("repo");
    let (_, with) = init_repository(&repository);
    run(
        &temp.path().join("run"),
        Some((WorkflowV2Status::Accepted, &with)),
    );
    assert_eq!(
        open_after(&temp.path().join("run"), &repository),
        vec![PATH.to_string()]
    );
}

#[test]
fn a_discharge_does_not_survive_the_path_existing_again() {
    let mut ledger = AuditLedger::default();
    let contract = AuditContract {
        schema_version: 1,
        snapshot: "snap".into(),
        declared_paths: vec![PATH.into()],
    };
    ledger.accept(contract, absent_report()).unwrap();
    ledger.record_discharges(vec![Discharge {
        declared_path: PATH.into(),
        snapshot: "snap".into(),
        verification_call_id: VERIFY.into(),
        base_commit: "abc".into(),
    }]);
    assert!(ledger.unresolved("snap").unwrap().is_empty());
    let mut exists = absent_report();
    exists.snapshot = "later".into();
    exists.records[0].verdict = Verdict::ExistsAsDeclared;
    exists.records[0].required_action = RequiredAction::None;
    let contract = AuditContract {
        schema_version: 1,
        snapshot: "later".into(),
        declared_paths: vec![PATH.into()],
    };
    ledger.accept(contract, exists).unwrap();
    assert!(!ledger.is_discharged(PATH, "later"));
}

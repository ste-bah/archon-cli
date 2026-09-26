//! Issue-112 end to end: a root-level file one task declares and keeps, and
//! another task's review remediation -- granted it because nothing else in
//! its wave claimed it -- deletes as a stray and has verified. Through the
//! production write wave (Git, scope grant, manifests, post-apply audit) and
//! the audit runtime: the path is CONTESTED, the final gate names both tasks,
//! a resume does not re-dispatch the keeper to re-deliver it, and the
//! keeper's own verification of the tree without it discharges it.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeMap;

use archon_workflow::repository_audit::discharge::stamp_judged_commit;
use archon_workflow::repository_audit::reuse;
use archon_workflow::repository_audit::runtime::{AuditRuntime, Snapshot};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::WorkflowV2DispatchedItem;
use archon_workflow::*;
use serde_json::json;
use support::{AuditScript, DELETE, Edits, Fixture, git};

const REPORT: &str = "report.json";
const KEEPER: &str = "TASK-KEEP";
const DELETER: &str = "TASK-DEL";

fn no_findings() -> AuditScript {
    AuditScript {
        flagged: vec![],
        dispositions: BTreeMap::new(),
    }
}

fn edits(files: Vec<(&'static str, &'static str)>) -> Edits {
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

/// The keeper declares the report (as the live task does, among its files);
/// the deleter declares only its own module. The universe is persisted where
/// the host keeps it.
fn fixture() -> Fixture {
    let mut f = Fixture::new();
    let task = |id: &str, owns: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![task(KEEPER, &[REPORT]), task(DELETER, &["other.txt"])],
    };
    std::fs::create_dir_all(f.v2.root()).unwrap();
    std::fs::write(
        f.v2.root().join("generated-metadata.json"),
        json!({"task_universe": universe}).to_string(),
    )
    .unwrap();
    f.universe = Some(universe);
    f
}

/// One single-item wave of `task` through `store`.
async fn wave(
    f: &mut Fixture,
    store: &WorkflowV2ResultStore,
    id: &str,
    task: &str,
    targets: Vec<&str>,
    edits: Edits,
    must_replay: bool,
) -> WorkflowV2Result {
    f.item_task_ids = vec![task.into()];
    let call = WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".into()),
            task: Some("Implement the item now.".into()),
            target_files_from_item: true,
            ..Default::default()
        },
    };
    let branch_id = format!("{id}-0");
    let mut branch = call.clone();
    branch.id = branch_id.clone();
    branch.method = WorkflowV2HostMethod::Implementation;
    branch.options.target_files = targets.iter().map(|t| (*t).to_string()).collect();
    let item = WorkflowV2FanoutItem::read_only(
        branch_id.clone(),
        "coder",
        branch,
        json!({"item": {"item_id": branch_id, "canonical_task_ids": [task],
            "target_files": targets, "work_type": "implementation"}}),
    );
    let (result, _) = f
        .wave_on(
            store,
            call,
            vec![(item, edits)],
            Some(no_findings()),
            &[],
            &[],
            must_replay,
        )
        .await;
    let record = WorkflowV2CallRecord::new(
        f.run.clone(),
        WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions::default(),
        },
        1,
        "input".into(),
        result.clone(),
        vec![],
    );
    store.save_call_record(&record).unwrap();
    result
}

/// A host-attributed verification of `task` that judged HEAD.
fn verified(f: &Fixture, id: &str, task: &str) {
    let head = git(&f.repo, &["rev-parse", "HEAD"]);
    let mut branch = WorkflowV2Result::accepted("every finding resolved");
    branch.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "focused tests pass",
    ));
    stamp_judged_commit(&mut branch, Some(&head));
    let mut result = WorkflowV2Result::accepted("verified");
    result.data = json!({"outcomes": [{"item_id": format!("{id}-0"), "status": "accepted",
        "error": null, "result": branch}]});
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        json!({"version": 1, "stage": "verify", "taskId": task, "round": 1, "maxRounds": 2,
            "sourceReduceCallIds": ["adversarial-review-reduce"]}),
    );
    let mut record = WorkflowV2CallRecord::new(
        f.run.clone(),
        WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        1,
        "input".into(),
        result,
        vec![],
    );
    record.dispatched_items = vec![WorkflowV2DispatchedItem {
        item_id: format!("{id}-0"),
        canonical_task_ids: vec![task.into()],
    }];
    f.v2.save_call_record(&record).unwrap();
}

/// The audit asked again about the tree it last assessed: no assessor
/// runs, but what the host's records say about that report is judged anew.
struct NoAssessor;
#[async_trait::async_trait]
impl WorkflowAgentDispatch for NoAssessor {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        execution: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&task_universe::WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        panic!(
            "{} dispatched; the tree was already assessed",
            execution.call.id
        )
    }
}

async fn reassess(f: &Fixture, audit: &AuditRuntime) -> String {
    let paths: Vec<String> = audit.state().unwrap().declared_paths.into_iter().collect();
    let snapshot = Snapshot::capture(&f.repo, &paths, &f.v2).unwrap();
    audit
        .assess(&snapshot, &paths, "final", &NoAssessor)
        .await
        .unwrap();
    snapshot.identity
}

#[tokio::test]
async fn a_shared_deliverable_one_declarer_deleted_in_a_verified_landing_is_contested() {
    let mut f = fixture();
    let session = f.v2.clone();
    let kept = wave(
        &mut f,
        &session,
        "agents-3",
        KEEPER,
        vec![REPORT],
        edits(vec![(REPORT, "{\"schema\": \"report\"}\n")]),
        false,
    )
    .await;
    assert_eq!(kept.status, WorkflowV2Status::Accepted, "{kept:#?}");
    verified(&f, "verification-wave-verify-keep-4", KEEPER);
    // The deleter's item declares only its module; the scope grant hands it
    // the root-level report nothing else in its wave claims.
    let deleted = wave(
        &mut f,
        &session,
        "review-remediate-del-1-45",
        DELETER,
        vec!["other.txt"],
        edits(vec![("other.txt", "other fixed\n"), (REPORT, DELETE)]),
        false,
    )
    .await;
    assert_eq!(deleted.status, WorkflowV2Status::Accepted, "{deleted:#?}");
    let manifest = f.manifest("review-remediate-del-1-45", "review-remediate-del-1-45-0");
    assert!(
        manifest["deleted_files"]
            .as_array()
            .unwrap()
            .contains(&json!(REPORT)),
        "{manifest:#}"
    );
    assert!(
        manifest["declared_target_files"]
            .as_array()
            .unwrap()
            .contains(&json!(REPORT))
    );
    assert!(!f.repo.join(REPORT).exists());

    let audit = f.audit_runtime();
    let snapshot = audit.state().unwrap().snapshot.unwrap().identity;
    // The post-apply audit ran before any verification of the deletion: an
    // ordinary open obligation, which refuses the keeper's cached writes.
    assert!(audit.require_closed(&snapshot).is_err());
    assert!(!reuse::admits(&audit.state().unwrap(), &[REPORT.to_string()]).unwrap());

    verified(&f, "verification-wave-review-verify-del-1-46", DELETER);
    let identity = reassess(&f, &audit).await;
    assert_eq!(identity, snapshot, "the same tree: no new assessment ran");
    let state = audit.state().unwrap();
    assert!(
        state.ledger.is_contested(REPORT, &snapshot),
        "{:#?}",
        state.ledger.contests
    );
    assert!(!state.ledger.is_discharged(REPORT, &snapshot));
    let error = audit.require_closed(&snapshot).unwrap_err().to_string();
    assert!(error.contains("contested"), "{error}");
    assert!(error.contains(KEEPER) && error.contains(DELETER), "{error}");
    // Contested is open, but no delivery obligation: the keeper's recorded
    // write is admitted from cache, so a resume does not re-deliver it.
    assert!(reuse::admits(&state, &[REPORT.to_string()]).unwrap());
    let resumed = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let replayed = wave(
        &mut f,
        &resumed,
        "agents-3",
        KEEPER,
        vec![REPORT],
        edits(vec![(REPORT, "{\"schema\": \"report\"}\n")]),
        true,
    )
    .await;
    assert_eq!(replayed.status, WorkflowV2Status::Accepted, "{replayed:#?}");
    assert!(
        !f.repo.join(REPORT).exists(),
        "the verified deletion stands"
    );
    assert!(git(&f.repo, &["ls-tree", "HEAD", "--", REPORT]).is_empty());

    // The keeper's own verification of the tree without it: every declarer
    // agrees, and the absence is discharged.
    verified(&f, "verification-wave-review-verify-keep-9", KEEPER);
    reassess(&f, &audit).await;
    let state = audit.state().unwrap();
    assert!(state.ledger.is_discharged(REPORT, &snapshot));
    assert!(state.ledger.contested(&snapshot).is_empty());
    audit.require_closed(&snapshot).unwrap();
}

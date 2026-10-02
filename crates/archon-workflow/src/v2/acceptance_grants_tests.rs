//! Batch O2 (ACC-H3): acceptance grants go through the chained ledger, and a
//! file the ledger records an owner for goes to that owner.

use super::super::route_failures_owned;
use super::*;
use crate::task_scope_amendment::history;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::acceptance_stage::AcceptanceCheckStatus;

struct World {
    _dir: tempfile::TempDir,
    repo: std::path::PathBuf,
    run_root: std::path::PathBuf,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let run_root = dir.path().join("runs/run-1");
    std::fs::create_dir_all(run_root.join("v2")).unwrap();
    for path in [
        "pkg/Cargo.toml",
        "pkg/core/engine.rs",
        "pkg/core/loose.rs",
        "pkg/core/sealed.rs",
        "docs/notes.md",
    ] {
        let target = repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    World {
        _dir: dir,
        repo,
        run_root,
    }
}

fn task(id: &str, owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-IMPL", &["pkg/core/engine.rs"], &[]),
            task("TASK-DOCS", &["pkg/Cargo.toml"], &[]),
        ],
    }
}

fn failing(id: &str, stderr: &str, owners: &[&str]) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: id.into(),
        criterion: "the command succeeds".into(),
        kind: "command".into(),
        status: AcceptanceCheckStatus::Failed,
        exit_code: Some(1),
        operational_error: None,
        owning_tasks: owners.iter().map(|s| s.to_string()).collect(),
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}

fn round(checks: Vec<AcceptanceCheckRecordV1>) -> AcceptanceRoundRecordV1 {
    AcceptanceRoundRecordV1 {
        schema_version: 1,
        run_id: "run".into(),
        call_id: "acceptance-contract-run-1".into(),
        round: 1,
        attempt: 1,
        max_rounds: 3,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    }
}

fn owner_record(w: &World, task: &str, path: &str) {
    amend_task_scope(ScopeAmendmentRequest {
        run_root: &w.run_root,
        universe: &universe(),
        repository_root: &w.repo,
        grants: vec![ScopeAmendment {
            task_id: task.into(),
            path: path.into(),
            kind: ScopeGrantKind::Owner,
            root: ScopeGrantRoot::Repository,
            shared_with: BTreeSet::new(),
            evidence: "the set gate's ownership map".into(),
        }],
        trigger: "set gate ownership map",
    })
    .unwrap();
}

fn routed(w: &World, record: &mut AcceptanceRoundRecordV1) {
    let owned = recorded_ownership(&w.run_root).unwrap();
    route_failures_owned(Some(&universe()), &w.repo, &[], &owned, record);
    record_routed_grants(&w.run_root, Some(&universe()), &w.repo, record);
}

#[test]
fn an_unowned_file_is_granted_through_the_ledger_one_link_per_check_and_only_once() {
    let w = world();
    let panic = "thread 'main' panicked at pkg/core/loose.rs:9:1:\nboom\n";
    let mut record = round(vec![failing("AC-1", panic, &["TASK-IMPL"])]);
    routed(&w, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.granted_files, ["pkg/core/loose.rs"]);
    assert_eq!(
        routing.granted_to["pkg/core/loose.rs"],
        vec!["TASK-IMPL".to_string()]
    );
    let ledger = ScopeAmendmentLedger::load(&w.run_root).unwrap();
    assert_eq!(ledger.lineage.len(), 1);
    assert!(ledger.lineage[0].trigger.contains("acceptance check AC-1"));
    let grant = &ledger.set.grants[0];
    assert_eq!(
        (grant.task_id.as_str(), grant.path.as_str(), grant.kind),
        (
            "TASK-IMPL",
            "pkg/core/loose.rs",
            ScopeGrantKind::OwnerlessAssignment
        )
    );
    ledger.verify(&history(&w.run_root)).unwrap();
    // The host re-runs the round on resume: nothing new is recorded.
    let mut again = round(vec![failing("AC-1", panic, &["TASK-IMPL"])]);
    routed(&w, &mut again);
    // The grant is in force (and the ledger now records TASK-IMPL as
    // answering for the file): the same grant, nothing recorded again.
    let (first, second) = (
        record.checks[0].routing.clone().unwrap(),
        again.checks[0].routing.clone().unwrap(),
    );
    assert_eq!(second.granted_files, first.granted_files);
    assert_eq!(second.granted_to, first.granted_to);
    assert_eq!(
        ScopeAmendmentLedger::load(&w.run_root)
            .unwrap()
            .lineage
            .len(),
        1
    );
}

#[test]
fn a_file_with_a_recorded_owner_goes_to_that_owner_even_under_a_deliverable_root() {
    let w = world();
    // `docs/` lies outside every scope root the tasks declare; the ledger
    // records TASK-DOCS as answering for the note.
    owner_record(&w, "TASK-DOCS", "docs/notes.md");
    let panic = "thread 'main' panicked at docs/notes.md:2:1:\nstale\n";
    let mut record = round(vec![failing("AC-2", panic, &["TASK-IMPL"])]);
    routed(&w, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.writer_tasks, ["TASK-DOCS"]);
    assert_eq!(routing.granted_files, ["docs/notes.md"]);
    assert_eq!(
        routing.granted_to["docs/notes.md"],
        vec!["TASK-DOCS".to_string()]
    );
    let ledger = ScopeAmendmentLedger::load(&w.run_root).unwrap();
    let grant = (ledger.set.grants.iter())
        .find(|g| g.kind.writable())
        .expect("a write grant");
    // A repository file lands through the patch: no root list names it a
    // deliverable root (only stored data under a recorded data root is).
    assert_eq!(
        (grant.task_id.as_str(), grant.kind, grant.root),
        (
            "TASK-DOCS",
            ScopeGrantKind::OwnerlessAssignment,
            ScopeGrantRoot::Repository
        )
    );
    // Without the record the same file is outside the plan's scope roots.
    let bare = world();
    let mut record = round(vec![failing("AC-2", panic, &["TASK-IMPL"])]);
    routed(&bare, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert!(routing.granted_files.is_empty());
    assert!(
        routing.unwritable[0]
            .1
            .contains("outside the plan's scope roots")
    );
}

#[test]
fn a_recorded_owner_that_forbids_the_file_is_never_granted_it() {
    let w = world();
    let mut universe = universe();
    universe.tasks[1]
        .files_forbidden_to_change
        .push("`docs/**`".into());
    let owned = OwnershipMap::from([(
        "docs/notes.md".to_string(),
        BTreeSet::from(["TASK-DOCS".to_string()]),
    )]);
    let panic = "thread 'main' panicked at docs/notes.md:2:1:\nstale\n";
    let mut record = round(vec![failing("AC-3", panic, &["TASK-IMPL"])]);
    route_failures_owned(Some(&universe), &w.repo, &[], &owned, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert!(routing.granted_files.is_empty());
    assert!(
        routing.unwritable[0].1.contains("its recorded owner"),
        "{routing:?}"
    );
}

#[test]
fn stored_project_data_the_failure_names_is_a_project_grant_never_a_script_target() {
    let w = world();
    let project = w
        .run_root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("project");
    let stored = project.join(".archon/store/data/bars.json");
    std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
    std::fs::write(&stored, "{}\n").unwrap();
    let text = format!("AssertionError: {} holds a stale close\n", stored.display());
    let canonical = project.canonicalize().unwrap();
    let roots = |inputs: &[&str]| {
        let policy = crate::write_coordinator::project_inputs::ProjectInputPolicy {
            project: canonical.clone(),
            inputs: inputs.iter().map(std::path::PathBuf::from).collect(),
            excludes: Vec::new(),
            task_root: canonical.join("tasks"),
            limit: 1 << 20,
            combined: true,
        };
        let universe = WorkflowV2TaskUniverse {
            schema_version: "test".into(),
            source_roots: Vec::new(),
            tasks: Vec::new(),
        };
        DeclaredDataRoots::read(&policy, &universe, &w.repo)
    };
    // Only under a root the run's records declare.
    assert_eq!(
        named_stored_data(&text, &roots(&[".archon/store"])),
        BTreeMap::from([(
            ".archon/store/data/bars.json".to_string(),
            ScopeGrantRoot::Project
        )])
    );
    assert!(named_stored_data(&text, &roots(&[])).is_empty());
    // Engine state is never project data, whatever root covers it.
    std::fs::create_dir_all(project.join(".archon/workflows/r")).unwrap();
    std::fs::write(project.join(".archon/workflows/r/state.json"), "{}").unwrap();
    let engine = format!(
        "see {}",
        project.join(".archon/workflows/r/state.json").display()
    );
    assert!(named_stored_data(&engine, &roots(&[".archon"])).is_empty());
}

/// M6: a file the blamed landing deleted keeps its (ledger-less) grant only
/// when it is that landing's change AND every grantee is one of that
/// landing's tasks; granted to anyone else -- here its recorded owner, who
/// did not land the deletion -- it is withdrawn, with why.
#[test]
fn a_deleted_file_is_restored_only_by_the_landing_that_deleted_it() {
    use crate::v2::acceptance_regression::AcceptanceRegressionV1;
    let regressed = |tasks: &[&str]| AcceptanceRegressionV1 {
        held_at: "base".into(),
        landing_commit: "landing".into(),
        landing_stage: "implementation".into(),
        tasks: tasks.iter().map(|t| t.to_string()).collect(),
        changed_files: vec!["pkg/core/loose.rs".into()],
        probed_as: None,
    };
    let panic = "thread 'main' panicked at pkg/core/engine.rs:1:1:\nboom\n";
    // The landing's own authors restore what they deleted.
    let w = world();
    std::fs::remove_file(w.repo.join("pkg/core/loose.rs")).unwrap();
    let mut check = failing("AC-5", panic, &["TASK-IMPL"]);
    check.regressed_by = Some(regressed(&["TASK-IMPL"]));
    let mut record = round(vec![check]);
    routed(&w, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert!(
        routing
            .granted_files
            .contains(&"pkg/core/loose.rs".to_string()),
        "{routing:?}"
    );
    // Recorded as owned by a task that did not land the deletion: withdrawn.
    let w = world();
    owner_record(&w, "TASK-DOCS", "pkg/core/loose.rs");
    std::fs::remove_file(w.repo.join("pkg/core/loose.rs")).unwrap();
    let mut check = failing("AC-6", panic, &["TASK-IMPL"]);
    check.regressed_by = Some(regressed(&["TASK-IMPL"]));
    let mut record = round(vec![check]);
    routed(&w, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert!(
        !routing
            .granted_files
            .contains(&"pkg/core/loose.rs".to_string()),
        "{routing:?}"
    );
    assert!(
        (routing.unwritable.iter())
            .any(|(file, why)| file == "pkg/core/loose.rs" && why.contains("could not grant")),
        "{routing:?}"
    );
}

/// Fail closed (7-9): a ledger that cannot be read is a round error, and
/// nothing is granted without it.
#[test]
fn an_unreadable_ledger_grants_nothing_and_says_so() {
    let w = world();
    std::fs::write(
        crate::task_scope_amendment::ledger_path(&w.run_root),
        b"not a ledger",
    )
    .unwrap();
    let panic = "thread 'main' panicked at pkg/core/loose.rs:9:1:\nboom\n";
    let mut record = round(vec![failing("AC-7", panic, &["TASK-IMPL"])]);
    route_failures_owned(
        Some(&universe()),
        &w.repo,
        &[],
        &OwnershipMap::new(),
        &mut record,
    );
    record_routed_grants(&w.run_root, Some(&universe()), &w.repo, &mut record);
    assert!(
        record.operational_errors[0].contains("could not record its write grants"),
        "{:?}",
        record.operational_errors
    );
}

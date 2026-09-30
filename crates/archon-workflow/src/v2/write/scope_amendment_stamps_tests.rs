//! Batch O: a run's scope amendments reach every write branch.

use super::*;
use crate::task_scope_amendment::{
    ScopeAmendment, ScopeAmendmentRequest, ScopeGrantKind, ScopeGrantRoot, amend_task_scope,
};
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

fn branch(tasks: &[&str], artifacts: serde_json::Value) -> crate::WorkflowV2FanoutItem {
    let call = WorkflowV2HostCall {
        id: "impl-0".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(crate::v2::WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    crate::WorkflowV2FanoutItem::read_only(
        "impl-0",
        "coder",
        call,
        serde_json::json!({"item": {"item_id": "impl-0", "canonical_task_ids": tasks,
            "target_files": ["src/a.rs"], "project_artifact_requirements": artifacts}}),
    )
}

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, owns: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-A", &["src/a.rs"]), task("TASK-B", &["src/b.rs"])],
    }
}

#[test]
fn without_a_ledger_nothing_changes() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    let mut branches = vec![branch(&["TASK-A"], serde_json::Value::Null)];
    let before = branches.clone();
    assert!(
        apply(&mut branches, &store, Some(&universe()))
            .unwrap()
            .is_none()
    );
    assert_eq!(branches[0].input, before[0].input);
}

/// A repository grant is the grantee's declared file in the universe the
/// fan-out plans with; a project-data grant is a declared project artifact
/// of the grantee's branches only; a tampered ledger stops the call.
#[test]
fn grants_amend_the_universe_and_stamp_project_data_on_the_grantees_branches() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let project = dir.path().join("project");
    let run_root = project.join(".archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    crate::write_coordinator::project_inputs::write_test_policy(&run_root, &project, &[]);
    for (root, rel) in [
        (&repo, "src/loose.rs"),
        (&project, ".archon/lab/data/x.json"),
    ] {
        std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
        std::fs::write(root.join(rel), "{}").unwrap();
    }
    let u = universe();
    let grant = |path: &str| ScopeAmendment {
        task_id: "TASK-A".into(),
        path: path.into(),
        kind: ScopeGrantKind::OwnerlessAssignment,
        root: ScopeGrantRoot::Repository,
        shared_with: Default::default(),
        evidence: "a landing of the task changed it".into(),
    };
    let outcome = amend_task_scope(ScopeAmendmentRequest {
        run_root: &run_root,
        universe: &u,
        repository_root: &repo,
        grants: vec![grant("src/loose.rs"), grant(".archon/lab/data/x.json")],
        trigger: "test",
    })
    .unwrap();
    assert!(outcome.refused.is_empty(), "{outcome:?}");
    let store = WorkflowV2ResultStore::new(run_root.join("v2"));
    let mut branches = vec![
        branch(&["TASK-A"], serde_json::json!("docs/own.md")),
        branch(&["TASK-B"], serde_json::Value::Null),
    ];
    let amended = apply(&mut branches, &store, Some(&u)).unwrap().unwrap();
    assert_eq!(
        branches[0].input["item"]["project_artifact_requirements"],
        serde_json::json!(["docs/own.md", ".archon/lab/data/x.json"])
    );
    assert!(branches[1].input["item"]["project_artifact_requirements"].is_null());
    let owners = crate::v2::script::residual_paths::owners(&amended, "src/loose.rs", &repo);
    assert_eq!(owners.into_iter().collect::<Vec<_>>(), ["TASK-A"]);
    let rels = crate::write_coordinator::project_inputs::declared_rel_paths(
        &branches[0].input,
        &[],
        project.to_str().unwrap(),
    );
    assert!(
        rels.contains(&".archon/lab/data/x.json".to_string()),
        "{rels:?}"
    );

    let ledger = crate::task_scope_amendment::ledger_path(&run_root);
    let text = std::fs::read_to_string(&ledger).unwrap();
    std::fs::write(&ledger, text.replace("src/loose.rs", "src/other.rs")).unwrap();
    let error = apply(&mut branches, &store, Some(&u)).unwrap_err();
    assert!(error.to_string().contains("chain ends at"), "{error}");
}

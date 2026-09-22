//! Issue-71: a branch whose declared focused filter names a test module no
//! task declares is widened to that module's file and directory; a module
//! another task declares stays that task's.
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use tokio::sync::Semaphore;

use super::super::test_baseline::tests::{Host, repository, runs, spec, universe};
use super::super::worktree_scope_grant::ScopeGrant;
use super::super::{WorktreeFanoutSetup, WorktreePlanRunContext};
use super::prepare_worktree_wave;
use crate::agent_dispatch_port::declared_targets;
use crate::v2::{
    WorkflowV2AgentAdapter, WorkflowV2CallExecution, WorkflowV2FanoutItem, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2ResultStore, WorkflowV2WriteAssignment, WorkflowV2WriteMode,
    WorkflowV2WriteWave,
};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::{WorkflowStore, WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status};

#[tokio::test]
async fn a_declared_focused_filter_widens_the_branch_to_its_undeclared_test_module() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, _ws) = repository(temp.path());
    let counter = temp.path().join("runs");
    // Both commands pass on the base commit (`:` is a no-op), so nothing is
    // widened by obligation: whatever widens below is the filter alone.
    // `nobody` is a module no task declares; `theirs` is TASK-B's.
    let mine = format!(
        ": cargo test -p app --lib nobody::tests::round_trips ; echo run >> \"{}\"",
        counter.display()
    );
    let theirs = ": cargo test -p app --lib theirs::tests".to_string();
    let control = WorkflowStore::new(temp.path().join("workflows"));
    let run = control.create_run(spec()).unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let universe = universe();
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "agents-3".into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        input: serde_json::json!({}),
        depends_on: vec![],
    };
    let setup = WorktreeFanoutSetup {
        canonical_root: canonical.clone(),
        cfg: WriteCoordinatorConfig::default(),
        run_root: temp.path().join("run"),
    };
    let ctx = WorktreePlanRunContext {
        task: "implement",
        target_repository_root: canonical.to_str(),
        execution: &execution,
        adapter: WorkflowV2AgentAdapter::new(),
        dispatch: &Host,
        v2_store: &store,
        store_for_control: &control,
        run_id: &run.id,
        setup: &setup,
        semaphore: Arc::new(Semaphore::new(2)),
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
        task_universe: Some(&universe),
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "agents-3-a",
        "coder",
        WorkflowV2HostCall {
            id: "agents-3-agents-3-a".into(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        serde_json::json!({"item": {
            "item_id": "agents-3-a",
            "canonical_task_ids": ["TASK-A"],
            "target_files": ["src/mine.rs"],
            "focused_verification": [mine, theirs],
        }}),
    );
    let wave = WorkflowV2WriteWave {
        assignments: vec![WorkflowV2WriteAssignment {
            item_id: "agents-3-a".into(),
            owned_targets: vec!["src/mine.rs".into()],
            owned_scopes: Vec::new(),
            worktree_path: Some(temp.path().join("iso/agents-3-a").display().to_string()),
            artifact_only: false,
        }],
    };
    let prepared = prepare_worktree_wave(&ctx, &wave, &[branch]).await.unwrap();
    assert_eq!(runs(&counter), 1);
    let branch = &prepared[0];
    let record = branch.test_baseline.as_ref().unwrap();
    assert!(record.obligations.is_empty(), "{record:?}");

    // Gates 2 and 3: the plan.
    let targets: Vec<String> = branch
        .coordinator_plan
        .target_files
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        targets,
        vec!["src/mine.rs".to_string(), "src/nobody.rs".to_string()]
    );
    let scopes: Vec<String> = branch
        .coordinator_plan
        .target_dir_scopes
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(scopes, vec!["src/nobody".to_string()]);
    // Gate 1 and the adapter's claims: the assignment.
    assert_eq!(
        branch.assignment.owned_targets,
        vec!["src/mine.rs".to_string(), "src/nobody.rs".to_string()]
    );
    assert_eq!(
        branch.assignment.owned_scopes,
        vec!["src/nobody".to_string()]
    );
    // The wave claim, so a sibling cannot be granted the same paths.
    assert!(branch.wave_claims[0].owned.contains("src/nobody.rs"));
    assert!(branch.wave_claims[0].owned.contains("src/nobody"));
    // The sealed baseline, so the stale recheck covers the widened file.
    assert!(
        branch
            .baseline
            .declared_target_meta
            .contains_key("src/nobody.rs")
    );
    // Recorded for the preamble and the result.
    assert_eq!(
        branch.focused_test_targets,
        vec!["src/nobody.rs".to_string(), "src/nobody/".to_string()]
    );
    // TASK-B's module is TASK-B's.
    assert!(!targets.contains(&"src/theirs.rs".to_string()));
    assert!(!scopes.contains(&"src/theirs".to_string()));
    assert!(!branch.wave_claims[0].owned.contains("src/theirs.rs"));

    // Issue-64: the stamped target set the tool guard refuses writes
    // outside of carries the widened file and its directory.
    let mut input = branch.branch.input.clone();
    super::super::declared_targets::stamp(&mut input, &branch.coordinator_plan);
    assert_eq!(
        declared_targets(&input),
        vec![
            "src/mine.rs".to_string(),
            "src/nobody.rs".to_string(),
            "src/nobody/".to_string(),
        ]
    );

    // A new test file under the widened directory, and an edit to the
    // widened module file, are declared: neither out of scope nor granted.
    let root = &branch.workspace.plan.isolated_root;
    std::fs::create_dir_all(root.join("src/nobody")).unwrap();
    std::fs::write(
        root.join("src/nobody/round_trips.rs"),
        "#[test]\nfn t() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/nobody.rs"), "mod round_trips;\n").unwrap();
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        ..Default::default()
    };
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/nobody/round_trips.rs"));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/nobody.rs"));
    let grant = ScopeGrant::resolve_unforbidden(
        &branch.coordinator_plan,
        &result,
        Some(&branch.wave_claims),
    );
    assert!(grant.out_of_scope.is_empty(), "{grant:?}");
    assert!(grant.granted.is_empty(), "declared, not granted: {grant:?}");
    assert!(grant.covers("src/nobody/round_trips.rs"));
    assert!(grant.covers("src/nobody.rs"));

    // What the coder is told.
    let text = super::super::focused_test_targets::preamble(&branch.focused_test_targets);
    assert_eq!(
        text,
        "\nFocused-test modules: the files and module directories your declared focused test \
         commands resolve to are added to your declared targets, so you can add or change tests \
         there (src/nobody.rs, src/nobody/). A test file another task declares stays that \
         task's and is not listed.\n"
    );
    let mut stamped = WorkflowV2Result::default();
    super::super::focused_test_targets::stamp_result(&mut stamped, &branch.focused_test_targets);
    assert_eq!(
        stamped.data["focused_test_targets_widened"],
        serde_json::json!(["src/nobody.rs", "src/nobody/"])
    );
}

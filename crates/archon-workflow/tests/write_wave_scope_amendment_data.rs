//! Batch O (group 3) through the production write-wave seam: a finding that
//! names wrong STORED project data is fixed through the existing audited
//! project-input landing. The host grants the data file to the owning task
//! by a scope amendment; the task's next branch -- which declares no
//! artifact of its own -- is seeded with its copy, and its change lands in
//! the project root with a logged decision and a kept copy of what it
//! replaced, never through the repository patch (which would skip a path
//! git ignores).
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::{Path, PathBuf};

use archon_workflow::task_scope_amendment::{
    ScopeAmendment, ScopeAmendmentRequest, ScopeGrantKind, ScopeGrantRoot, amend_task_scope,
};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

/// Stored data outside the acceptance policy's inputs: only a grant can
/// make it a branch's to change.
const STORED: &str = ".archon/store/data/bars.json";

fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
    .canonicalize()
    .unwrap()
}

fn fixture(scratch: &Path) -> Fixture {
    let mut f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-qm", "ignore project state"]);
    let project = project_root(&f);
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::create_dir_all(project.join(".archon/lab/data")).unwrap();
    let stored = project.join(STORED);
    std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
    std::fs::write(&stored, "{\"close\": 0}\n").unwrap();
    let policy = json!({
        "repository": f.repo.canonicalize().unwrap(), "project": project,
        "task_root": project.join("tasks"), "scratch_parent": scratch,
        "project_inputs": [".archon/lab/data"], "project_input_excludes": [],
        "combined": true, "toolchain_path": "/usr/bin:/bin", "environment": {},
        "environment_allowlist": [], "cargo_seed": null, "timeout_secs": 60,
        "output_bytes": 4096, "scratch_bytes": 1u64 << 30,
    });
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy, "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
    let path = f.store.run_dir(&f.run).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-001".into(),
            source_path: "tasks/TASK-001.md".into(),
            files_expected_to_change: vec!["`owned.txt` — exists (1 lines)".into()],
            ..Default::default()
        }],
    });
    f
}

fn landings(f: &Fixture) -> Vec<serde_json::Value> {
    let log = f
        .store
        .run_dir(&f.run)
        .join("write-coordination/project-inputs.jsonl");
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["outcome"] != "intent")
        .collect()
}

#[tokio::test]
async fn a_granted_stored_data_fix_lands_through_the_audited_project_input_landing() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let project = project_root(&f);
    let run_root = f.store.run_dir(&f.run);
    let outcome = amend_task_scope(ScopeAmendmentRequest {
        run_root: &run_root,
        universe: f.universe.as_ref().unwrap(),
        repository_root: &f.repo,
        grants: vec![ScopeAmendment {
            task_id: "TASK-001".into(),
            path: STORED.into(),
            kind: ScopeGrantKind::DeliverableRoot,
            root: ScopeGrantRoot::Repository,
            shared_with: Default::default(),
            evidence: "a review finding names the stored bars as wrong".into(),
        }],
        trigger: "review finding",
    })
    .unwrap();
    assert!(outcome.refused.is_empty(), "{outcome:?}");
    assert_eq!(outcome.applied[0].root, ScopeGrantRoot::Project);

    let call = WorkflowV2HostCall {
        id: "fix".into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".into()),
            task: Some("Fix the stored data now.".into()),
            target_files_from_item: true,
            ..Default::default()
        },
    };
    let mut branch = call.clone();
    branch.id = "fix-0".into();
    branch.method = WorkflowV2HostMethod::Implementation;
    branch.options.target_files = vec!["owned.txt".into()];
    let item = WorkflowV2FanoutItem::read_only(
        "fix-0".to_string(),
        "coder",
        branch,
        json!({"item": {"item_id": "fix-0", "canonical_task_ids": ["TASK-001"],
            "target_files": ["owned.txt"], "work_type": "implementation"}}),
    );
    let edits = Edits {
        files: vec![
            ("owned.txt", "implemented\n"),
            (STORED, "{\"close\": 101}\n"),
        ],
        report: vec!["owned.txt"],
        via_adapter: false,
    };
    let (result, _) = f
        .wave_on(&f.v2, call, vec![(item, edits)], None, &[], &[], false)
        .await;
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(
        std::fs::read_to_string(project.join(STORED)).unwrap(),
        "{\"close\": 101}\n",
        "the fix reached the project's stored data"
    );
    let log = landings(&f);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["outcome"], json!("applied"));
    assert_eq!(log[0]["path"], json!(STORED));
    assert_eq!(log[0]["task_ids"], json!(["TASK-001"]));
    let kept = run_root
        .join("write-coordination/project-inputs-replaced/fix/fix-0")
        .join(STORED);
    assert_eq!(
        std::fs::read_to_string(kept).unwrap(),
        "{\"close\": 0}\n",
        "what it replaced is kept"
    );
    assert_eq!(
        git(&f.repo, &["ls-files", ".archon"]),
        "",
        "never the patch"
    );
}

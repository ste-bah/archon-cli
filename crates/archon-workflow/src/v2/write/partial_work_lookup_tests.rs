use super::*;
use crate::v2::write::partial_work::{capture_partial_work, resume_into_workspace};
use std::process::Command;

fn sh(args: &[&str], cwd: &Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A run directory: `<run>/v2` for the store, `<run>/write-coordination` for
/// the patches, exactly as the worktree wave lays them out.
fn run_store(root: &Path) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(root.join("run").join("v2"))
}

fn patch_on_disk(root: &Path, stage: &str, item: &str, body: &str) -> PartialWork {
    let dir = root
        .join("run")
        .join("write-coordination")
        .join("stages")
        .join(stage)
        .join("partial");
    std::fs::create_dir_all(&dir).unwrap();
    let patch_path = dir.join(format!("{item}.patch"));
    std::fs::write(&patch_path, body).unwrap();
    PartialWork {
        patch_path,
        files: vec!["lib.rs".into()],
        bytes: body.len() as u64,
        baseline_commit: "c".into(),
    }
}

fn outcome(
    item: &str,
    status: WorkflowV2Status,
    task: &str,
    partial: Option<&PartialWork>,
    hash: &str,
) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result {
        status,
        data: serde_json::json!({"canonical_task_ids": [task]}),
        ..WorkflowV2Result::default()
    };
    if let Some(partial) = partial {
        result.data[DATA_KEY] = serde_json::to_value(partial).unwrap();
    }
    WorkflowV2BranchOutcome {
        item_id: item.into(),
        role: "coder".into(),
        status,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: Some(hash.into()),
        completion_evidence: Vec::new(),
    }
}

/// (a) The live shape: the record that captured the partial was superseded
/// by a placeholder without one. No sidecar, as an older binary left it.
#[test]
fn a_partial_on_a_superseded_record_is_still_found() {
    let temp = tempfile::tempdir().unwrap();
    let store = run_store(temp.path());
    let partial = patch_on_disk(temp.path(), "agents-5", "agents-5-0", "diff --git a/lib.rs");
    let task = "TASK-DL-004";
    store
        .save_branch_outcome(
            "agents-5",
            &outcome(
                "agents-5-0",
                WorkflowV2Status::Failed,
                task,
                Some(&partial),
                "h1",
            ),
        )
        .unwrap();
    store
        .save_branch_outcome(
            "agents-5",
            &outcome(
                "agents-5-0",
                WorkflowV2Status::NeedsReview,
                task,
                None,
                "h2",
            ),
        )
        .unwrap();
    let current = store
        .load_branch_outcome("agents-5", "agents-5-0")
        .unwrap()
        .unwrap();
    assert!(
        current.result.unwrap().data.get(DATA_KEY).is_none(),
        "raw store write keeps the placeholder bare"
    );
    assert_eq!(store.load_superseded_branch_outcomes().len(), 1);
    let found =
        latest_partial_for_tasks(&store, &[task.to_string()]).expect("found via superseded");
    assert_eq!(found, partial);
    // Gone from disk: not a candidate, whatever the records say.
    std::fs::remove_file(&partial.patch_path).unwrap();
    assert!(latest_partial_for_tasks(&store, &[task.to_string()]).is_none());
}

/// (b) The sidecar alone resolves a patch: no outcome record at all.
#[test]
fn a_sidecar_resolves_a_partial_without_any_outcome_record() {
    let temp = tempfile::tempdir().unwrap();
    let store = run_store(temp.path());
    let partial = patch_on_disk(temp.path(), "agents-2", "agents-2-0", "diff --git a/lib.rs");
    write_sidecar(
        "agents-2",
        "agents-2-0",
        &["TASK-001".to_string()],
        &partial,
    )
    .unwrap();
    let sidecar: PartialSidecar =
        serde_json::from_str(&std::fs::read_to_string(sidecar_path(&partial.patch_path)).unwrap())
            .unwrap();
    assert_eq!(sidecar.schema_version, SIDECAR_SCHEMA_VERSION);
    assert_eq!(sidecar.canonical_task_ids, vec!["TASK-001".to_string()]);
    assert_eq!(sidecar.partial, partial);
    assert!(chrono::DateTime::parse_from_rfc3339(&sidecar.captured_at).is_ok());
    assert!(store.load_branch_outcomes().unwrap().is_empty());
    assert_eq!(
        latest_partial_for_tasks(&store, &["TASK-001".to_string()]),
        Some(partial.clone())
    );
    assert!(latest_partial_for_tasks(&store, &["TASK-009".to_string()]).is_none());
    // A newer capture for the same task wins, and the sidecar's file list is
    // preferred over a record naming the same patch.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let newer = patch_on_disk(temp.path(), "agents-4", "agents-4-0", "diff --git a/new.rs");
    write_sidecar("agents-4", "agents-4-0", &["TASK-001".to_string()], &newer).unwrap();
    let mut stale = newer.clone();
    stale.files = vec!["stale.rs".into()];
    store
        .save_branch_outcome(
            "agents-4",
            &outcome(
                "agents-4-0",
                WorkflowV2Status::Failed,
                "TASK-001",
                Some(&stale),
                "h",
            ),
        )
        .unwrap();
    assert_eq!(
        latest_partial_for_tasks(&store, &["TASK-001".to_string()]),
        Some(newer.clone())
    );
    // A sidecar with no task ids does not shadow the record naming its patch.
    write_sidecar("agents-4", "agents-4-0", &[], &newer).unwrap();
    assert_eq!(
        latest_partial_for_tasks(&store, &["TASK-001".to_string()]),
        Some(stale)
    );
}

/// (c) Rewriting an item's outcome through the write funnel carries the
/// previous record's partial forward when the new result has none.
#[test]
fn a_rewrite_without_partial_keeps_the_previous_partial() {
    let temp = tempfile::tempdir().unwrap();
    let store = run_store(temp.path());
    let partial = patch_on_disk(temp.path(), "agents-5", "agents-5-0", "diff --git a/lib.rs");
    store
        .save_branch_outcome(
            "agents-5",
            &outcome(
                "agents-5-0",
                WorkflowV2Status::Failed,
                "TASK-004",
                Some(&partial),
                "h1",
            ),
        )
        .unwrap();
    let placeholder = super::super::dependency_gate::blocked_on_dependency_result(
        "agents-5-0",
        &serde_json::json!({"item": {"item_id": "agents-5-0", "canonical_task_ids": ["TASK-004"]}}),
        None,
        &["TASK-002".to_string()],
    );
    assert!(placeholder.data.get(DATA_KEY).is_none());
    super::super::contract::save_write_branch_outcome(
        &store,
        "agents-5",
        "agents-5-0",
        "coder",
        Some("h2".into()),
        &placeholder,
    )
    .unwrap();
    let current = store
        .load_branch_outcome("agents-5", "agents-5-0")
        .unwrap()
        .unwrap();
    let result = current.result.unwrap();
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        serde_json::from_value::<PartialWork>(result.data[DATA_KEY].clone()).unwrap(),
        partial
    );
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("carried forward"))
    );
    // An accepted rewrite is the work landing: nothing to carry.
    let accepted = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        data: serde_json::json!({"canonical_task_ids": ["TASK-004"]}),
        ..Default::default()
    };
    super::super::contract::save_write_branch_outcome(
        &store,
        "agents-5",
        "agents-5-0",
        "coder",
        Some("h3".into()),
        &accepted,
    )
    .unwrap();
    let current = store
        .load_branch_outcome("agents-5", "agents-5-0")
        .unwrap()
        .unwrap();
    assert!(current.result.unwrap().data.get(DATA_KEY).is_none());
}

/// (d) A task that landed through another outcome is never resumed, and a
/// rewrite for it carries nothing forward.
#[test]
fn a_landed_task_is_neither_resumed_nor_carried() {
    let temp = tempfile::tempdir().unwrap();
    let store = run_store(temp.path());
    let canonical = temp.path().join("repo");
    std::fs::create_dir_all(&canonical).unwrap();
    sh(&["init", "-q"], &canonical);
    sh(&["config", "user.email", "t@example.invalid"], &canonical);
    sh(&["config", "user.name", "t"], &canonical);
    std::fs::write(canonical.join("lib.rs"), "fn a() {}\n").unwrap();
    sh(&["add", "."], &canonical);
    sh(&["commit", "-qm", "base"], &canonical);
    let ws = temp.path().join("ws");
    sh(
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            ws.to_str().unwrap(),
            "HEAD",
        ],
        &canonical,
    );
    std::fs::write(ws.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let run_root = temp.path().join("run");
    let partial = capture_partial_work(
        &ws,
        &run_root,
        "agents-2",
        "agents-2-0",
        &["TASK-001".to_string()],
    )
    .unwrap()
    .unwrap();
    assert!(
        sidecar_path(&partial.patch_path).is_file(),
        "capture writes the sidecar"
    );
    sh(&["reset", "--hard", "HEAD", "--quiet"], &ws);
    store
        .save_branch_outcome(
            "agents-2",
            &outcome(
                "agents-2-0",
                WorkflowV2Status::Failed,
                "TASK-001",
                Some(&partial),
                "h1",
            ),
        )
        .unwrap();
    let input =
        serde_json::json!({"item": {"item_id": "agents-3-0", "canonical_task_ids": ["TASK-001"]}});
    assert!(resume_into_workspace(&store, None, &input, &ws).is_some());
    sh(&["reset", "--hard", "HEAD", "--quiet"], &ws);
    store
        .save_branch_outcome(
            "agents-3",
            &outcome(
                "agents-3-0",
                WorkflowV2Status::Accepted,
                "TASK-001",
                None,
                "h2",
            ),
        )
        .unwrap();
    assert!(resume_into_workspace(&store, None, &input, &ws).is_none());
    assert_eq!(
        std::fs::read_to_string(ws.join("lib.rs")).unwrap(),
        "fn a() {}\n"
    );
    let mut rewrite = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        data: serde_json::json!({"canonical_task_ids": ["TASK-001"]}),
        ..Default::default()
    };
    carry_forward_partial_work(&store, "agents-2", "agents-2-0", &mut rewrite);
    assert!(rewrite.data.get(DATA_KEY).is_none());
}

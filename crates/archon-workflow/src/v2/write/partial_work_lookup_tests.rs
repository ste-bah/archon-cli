use super::*;
use crate::v2::write::partial_work::{capture_partial_work, resume_into_workspace};
use std::process::Command;

fn sh(args: &[&str], cwd: &Path) {
    let out = Command::new("git")
        .args(["-c", "core.autocrlf=false", "-c", "core.eol=lf"])
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
        origin: None,
    }
}

/// Issue-20: a partial resolved through a record (no sidecar, or one an
/// older binary wrote) carries an origin derived from that record. The
/// tests below put origin-less partials on `Failed` records, so what comes
/// back is the input plus that derived origin.
fn with_failed_origin(partial: &PartialWork) -> PartialWork {
    PartialWork {
        origin: Some(super::super::partial_work::PartialOrigin {
            status: "failed".into(),
            summary: String::new(),
            residual_gaps: Vec::new(),
        }),
        ..partial.clone()
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
    assert_eq!(found, with_failed_origin(&partial));
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
    // The sidecar's file list wins; its missing origin is filled from the
    // record naming the same patch (Issue-20).
    assert_eq!(
        latest_partial_for_tasks(&store, &["TASK-001".to_string()]),
        Some(with_failed_origin(&newer))
    );
    // A sidecar with no task ids does not shadow the record naming its patch.
    write_sidecar("agents-4", "agents-4-0", &[], &newer).unwrap();
    assert_eq!(
        latest_partial_for_tasks(&store, &["TASK-001".to_string()]),
        Some(with_failed_origin(&stale))
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
        with_failed_origin(&partial)
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
        None,
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

/// Issue-20: a record an older binary wrote carries a partial with no origin.
/// The record itself is the verdict on that attempt, so the origin is read
/// from its status, summary and gaps; a partial that already has one keeps
/// it, and a carry-forward hands it on unchanged.
#[test]
fn partial_from_outcome_derives_the_origin_from_a_legacy_record() {
    let temp = tempfile::tempdir().unwrap();
    let store = run_store(temp.path());
    let partial = patch_on_disk(temp.path(), "agents-2", "agents-2-0", "diff --git a/lib.rs");
    let mut legacy = outcome(
        "agents-2-0",
        WorkflowV2Status::NeedsReview,
        "TASK-001",
        Some(&partial),
        "h1",
    );
    let result = legacy.result.as_mut().unwrap();
    result.summary = "ownership gate rejected the patch".into();
    result.residual_gaps.push(crate::v2::WorkflowV2ResidualGap {
        id: "undeclared_target".into(),
        description: "src/extra.rs is not in target_files".into(),
        severity: Some("blocker".into()),
    });
    assert!(
        result.data[DATA_KEY].get("origin").is_none(),
        "legacy shape has no origin"
    );
    let (task_ids, derived) = partial_from_outcome(&legacy).unwrap();
    assert_eq!(task_ids, vec!["TASK-001".to_string()]);
    let origin = derived.origin.expect("derived from the record");
    assert_eq!(origin.status, "needs_review");
    assert_eq!(origin.summary, "ownership gate rejected the patch");
    assert_eq!(origin.residual_gaps.len(), 1);
    assert_eq!(origin.residual_gaps[0].id, "undeclared_target");
    assert_eq!(origin.residual_gaps[0].severity, "blocker");
    assert!(!origin.is_timeout());
    // A partial recorded WITH an origin keeps it over the record's own text.
    let mut with_origin = partial.clone();
    with_origin.origin = Some(super::super::partial_work::PartialOrigin {
        status: "failed".into(),
        summary: "the capture-time verdict".into(),
        residual_gaps: Vec::new(),
    });
    let mut newer = outcome(
        "agents-2-0",
        WorkflowV2Status::NeedsReview,
        "TASK-001",
        Some(&with_origin),
        "h1",
    );
    newer.result.as_mut().unwrap().summary = "was not dispatched".into();
    let (_, kept) = partial_from_outcome(&newer).unwrap();
    assert_eq!(
        kept.origin.as_ref().map(|o| o.summary.as_str()),
        Some("the capture-time verdict")
    );
    // Carry-forward: the rewrite's partial carries the same origin.
    store.save_branch_outcome("agents-2", &newer).unwrap();
    let mut rewrite = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        data: serde_json::json!({"canonical_task_ids": ["TASK-001"]}),
        ..Default::default()
    };
    carry_forward_partial_work(&store, "agents-2", "agents-2-0", &mut rewrite);
    assert_eq!(
        rewrite.data[DATA_KEY]["origin"]["summary"],
        "the capture-time verdict"
    );
}

/// Issue-20, the on-disk shape of the live run: the rejected branch's
/// sidecar was written by a binary that knew no origin, and the sidecar wins
/// the lookup by patch path. The record naming the same patch is the verdict,
/// and the sidecar's candidate must take its origin from there — keeping the
/// sidecar's own task ids and file list.
#[test]
fn a_legacy_sidecar_takes_its_origin_from_the_outcome_record() {
    let temp = tempfile::tempdir().unwrap();
    let store = run_store(temp.path());
    let partial = patch_on_disk(temp.path(), "agents-7", "agents-7-0", "diff --git a/lib.rs");
    // The pre-Issue-20 sidecar: no `origin` key, its own file list.
    let legacy = serde_json::json!({
        "schema_version": SIDECAR_SCHEMA_VERSION,
        "stage_id": "agents-7",
        "branch_id": "agents-7-0",
        "canonical_task_ids": ["TASK-001"],
        "captured_at": "2026-09-15T07:38:00Z",
        "patch_path": partial.patch_path,
        "files": ["lib.rs", "tests/extra.rs"],
        "bytes": partial.bytes,
        "baseline_commit": "c",
    });
    std::fs::write(sidecar_path(&partial.patch_path), legacy.to_string()).unwrap();
    let mut rejected = outcome(
        "agents-7-0",
        WorkflowV2Status::NeedsReview,
        "TASK-001",
        Some(&partial),
        "h1",
    );
    let result = rejected.result.as_mut().unwrap();
    result.summary =
        "repository audit rejected unexplained or unauthorized changes: tests/extra.rs".into();
    result.residual_gaps.push(crate::v2::WorkflowV2ResidualGap {
        id: "audit_unexplained_change".into(),
        description: "tests/extra.rs has no audit disposition".into(),
        severity: Some("blocker".into()),
    });
    store.save_branch_outcome("agents-7", &rejected).unwrap();
    let found = latest_partial_for_tasks(&store, &["TASK-001".to_string()]).expect("found");
    assert_eq!(
        found.files,
        vec!["lib.rs".to_string(), "tests/extra.rs".to_string()],
        "sidecar's list kept"
    );
    let origin = found.origin.expect("origin taken from the record");
    assert_eq!(origin.status, "needs_review");
    assert_eq!(
        origin.summary,
        "repository audit rejected unexplained or unauthorized changes: tests/extra.rs"
    );
    assert_eq!(origin.residual_gaps.len(), 1);
    assert_eq!(origin.residual_gaps[0].id, "audit_unexplained_change");
    assert!(!origin.is_timeout());
    // A sidecar that already carries an origin keeps it over the record's.
    let mut with_origin = partial.clone();
    with_origin.origin = Some(super::super::partial_work::PartialOrigin {
        status: "failed".into(),
        summary: "the capture-time verdict".into(),
        residual_gaps: Vec::new(),
    });
    write_sidecar(
        "agents-7",
        "agents-7-0",
        &["TASK-001".to_string()],
        &with_origin,
    )
    .unwrap();
    let found = latest_partial_for_tasks(&store, &["TASK-001".to_string()]).expect("found");
    assert_eq!(
        found.origin.map(|o| o.summary),
        Some("the capture-time verdict".to_string())
    );
}

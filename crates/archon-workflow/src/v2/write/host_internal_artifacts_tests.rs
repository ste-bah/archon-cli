//! Issue-76: the host's own bookkeeping is never advertised as a branch
//! artifact, and never lands.
//!
//! The drop is exercised over a real sealed worktree, so the scan, the
//! restore-or-remove and the capture that follows it are the production ones
//! and not an in-memory stand-in.

use std::path::Path;

use archon_write_plan::{TargetFilesSource, WritePlan, normalize_target};

use super::{
    drop_host_internal_changes, is_host_internal_artifact_path, report_host_internal_drops,
};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::patch_manifest::{capture_patch, validate_patch};
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace, run_git,
};
use crate::{WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status};

/// The basename the live incident reproduced inside the worktree.
const HOST_FILE: &str = "patch_manifest.json";

fn git(root: &Path, args: &[&str]) {
    run_git(args, root).expect("git");
}

/// A canonical repository holding `owned.txt`, and a sealed worktree for an
/// item declaring it.
fn sealed(dir: &Path) -> (WritePlan, crate::write_coordinator::ItemWorkspace) {
    let canonical = dir.join("canonical");
    std::fs::create_dir_all(&canonical).unwrap();
    git(&canonical, &["init", "-q"]);
    git(&canonical, &["config", "user.name", "t"]);
    git(&canonical, &["config", "user.email", "t@example.invalid"]);
    std::fs::write(canonical.join("owned.txt"), "baseline\n").unwrap();
    git(&canonical, &["add", "."]);
    git(&canonical, &["commit", "-qm", "baseline"]);
    let plan = WritePlan {
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: "item-a".into(),
        canonical_root: canonical.clone(),
        isolated_root: dir.join("iso").join("item-a"),
        target_files: vec![normalize_target("owned.txt", &canonical).unwrap()],
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    };
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(&canonical, &plan, &[], &cfg).unwrap();
    let workspace = create_item_workspace(&canonical, &plan, &baseline).unwrap();
    (plan, workspace)
}

fn reported(paths: &[&str]) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        files_changed: paths
            .iter()
            .map(|p| WorkflowV2FileRecord::new(*p))
            .collect(),
        // An envelope parsed from an agent always carries an object here; the
        // reporters below stamp their finding into it.
        data: serde_json::json!({}),
        ..Default::default()
    }
}

/// The names are matched by basename, wherever the copy was put, and nothing
/// that merely resembles one is caught.
#[test]
fn host_internal_names_are_matched_by_basename_anywhere_in_the_tree() {
    for path in [
        HOST_FILE,
        "crates/a/patch_manifest.json",
        "gate-envelope.json",
        "nested/dir/gate-envelope.json",
        &format!("{}.jsonl", "a".repeat(64)),
    ] {
        assert!(is_host_internal_artifact_path(path), "{path}");
    }
    for path in [
        "owned.txt",
        "src/manifest.json",
        "patch_manifest.rs",
        "docs/patch_manifest.json.md",
        "notes.jsonl",
        &format!("{}.jsonl", "a".repeat(63)),
    ] {
        assert!(!is_host_internal_artifact_path(path), "{path}");
    }
}

/// The live shape in miniature: the coder copies the host's write manifest to
/// the repository root beside its real work. The copy is removed from the
/// worktree before any gate reads it, struck from the envelope, absent from
/// the captured patch — which still validates, so the branch is NOT failed —
/// and named in a review gap.
#[test]
fn a_host_internal_file_is_dropped_from_the_worktree_and_never_captured() {
    let dir = tempfile::tempdir().unwrap();
    let (plan, workspace) = sealed(dir.path());
    std::fs::write(plan.isolated_root.join("owned.txt"), "implemented\n").unwrap();
    std::fs::write(
        plan.isolated_root.join(HOST_FILE),
        "{\"stage\":\"R09\",\"write_mode\":\"worktree\"}\n",
    )
    .unwrap();
    let mut result = reported(&["owned.txt", HOST_FILE]);

    let dropped = drop_host_internal_changes(&plan, &mut result);

    assert_eq!(dropped, vec![HOST_FILE.to_string()]);
    assert!(!plan.isolated_root.join(HOST_FILE).exists());
    assert_eq!(
        result
            .files_changed
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>(),
        vec!["owned.txt".to_string()],
        "the dropped path must leave the envelope too: {result:#?}"
    );

    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(&plan.canonical_root, &plan, &[], &cfg).unwrap();
    let captured = capture_patch(&workspace, &plan.target_files, &baseline).expect("captured");
    assert_eq!(captured.changed_files, vec!["owned.txt".to_string()]);
    assert!(!String::from_utf8_lossy(&captured.patch_bytes).contains(HOST_FILE));
    validate_patch(&captured, &plan, &cfg, "implemented the declared target").expect("not failed");

    report_host_internal_drops(&mut result, "item-a", &dropped);
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == "host_internal_artifact_dropped_item-a")
        .unwrap_or_else(|| panic!("no host-internal gap: {result:#?}"));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains(HOST_FILE), "{gap:?}");
    assert_eq!(
        result.data["host_internal_dropped"],
        serde_json::json!([HOST_FILE])
    );
}

/// Layer 1: a branch result that went through the drop and the report — the
/// object a rejected-attempt replay stringifies into the next coder's prompt
/// — names no host coordination path at all. The write manifest used to be
/// pushed onto `artifacts` here, and that entry is what the coder reproduced.
#[test]
fn a_branch_result_rendered_for_a_prompt_names_no_host_coordination_path() {
    let dir = tempfile::tempdir().unwrap();
    let (plan, _workspace) = sealed(dir.path());
    std::fs::write(plan.isolated_root.join("owned.txt"), "implemented\n").unwrap();
    std::fs::write(plan.isolated_root.join(HOST_FILE), "{}\n").unwrap();
    let mut result = reported(&["owned.txt", HOST_FILE]);
    let dropped = drop_host_internal_changes(&plan, &mut result);
    report_host_internal_drops(&mut result, "item-a", &dropped);

    let rendered = serde_json::to_string(&result).unwrap();

    assert!(
        !rendered.contains("write-coordination"),
        "the host's own coordination directory must never reach a prompt: {rendered}"
    );
    assert!(
        !rendered.contains("/manifests/"),
        "the host's own manifest path must never reach a prompt: {rendered}"
    );
    assert!(
        result.artifacts.is_empty(),
        "a branch that delivered no artifact must advertise none: {result:#?}"
    );
}

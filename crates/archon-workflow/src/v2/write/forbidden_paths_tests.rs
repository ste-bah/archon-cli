//! Issue-30: the forbidden list resolved per branch, told to the agent,
//! stamped for the guard, and enforced by the grant over a real worktree.
use std::path::Path;

use archon_write_plan::{TargetFilesSource, WritePlan, normalize_target};

use super::{
    FORBIDDEN_DECLARED_CONFLICT_GAP_PREFIX, FORBIDDEN_PATH_CHANGED_GAP_PREFIX, ForbiddenPaths,
    forbidden_paths, forbidden_rejection_result, preamble, report_forbidden_declared_conflict,
    stamp,
};
use crate::agent_dispatch_port::{
    FORBIDDEN_PATHS_INPUT_KEY, declared_forbidden_paths, forbidden_path_roots,
};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::reuse_identity::reuse_input_hash;
use crate::v2::write::worktree_scope_grant::ScopeGrant;
use crate::v2::write_scope_extension::WaveClaim;
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace, run_git,
};
use crate::{WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status};

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, forbidden: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_forbidden_to_change: forbidden.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-001",
                &["`src/gate.rs` and `coverage.rs` (frozen)", "Frozen chain"],
            ),
            task("TASK-002", &["docs/**"]),
            task("TASK-003", &["src/elsewhere.rs"]),
        ],
    }
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

#[test]
fn the_list_is_the_union_over_the_branch_tasks_and_nothing_else() {
    let f = forbidden_paths(&universe(), &ids(&["TASK-001", "TASK-002"]));
    assert!(f.matches("src/gate.rs"));
    assert!(f.matches("crates/x/src/coverage.rs"));
    assert!(f.matches("docs/a.md"));
    assert!(
        !f.matches("src/elsewhere.rs"),
        "another task's list must not apply"
    );
    assert!(forbidden_paths(&universe(), &ids(&["TASK-009"])).is_empty());
    assert!(forbidden_paths(&universe(), &[]).is_empty());
}

#[test]
fn the_preamble_names_the_list_and_is_silent_when_empty() {
    let text = preamble(&forbidden_paths(&universe(), &ids(&["TASK-001"])));
    assert!(
        text.contains(
            "Forbidden paths for this task (never edit; a needed change there is a residual gap \
             to report, not an edit to make; declared targets take precedence): src/gate.rs, \
             **/coverage.rs."
        ),
        "{text}"
    );
    assert_eq!(preamble(&ForbiddenPaths::default()), "");
}

/// The stamp is a top-level host key: read back by the dispatch port, absent
/// when nothing is forbidden, and invisible to the reuse identity.
#[test]
fn the_stamp_round_trips_through_the_dispatch_port_and_never_moves_the_reuse_hash() {
    let mut input = serde_json::json!({"item": {"item_id": "i", "target_repository_root": "/repo"},
        "_workflow_project_artifact_policy": {"project_root": "/project"}});
    let before = reuse_input_hash(&input);
    stamp(&mut input, &ForbiddenPaths::default());
    assert!(input.get(FORBIDDEN_PATHS_INPUT_KEY).is_none());
    stamp(
        &mut input,
        &forbidden_paths(&universe(), &ids(&["TASK-001", "TASK-002"])),
    );
    assert_eq!(
        declared_forbidden_paths(&input),
        vec![
            "src/gate.rs".to_string(),
            "**/coverage.rs".to_string(),
            "docs/".to_string()
        ]
    );
    assert_eq!(
        reuse_input_hash(&input),
        before,
        "a host stamp must not perturb reuse"
    );
    assert_eq!(
        ForbiddenPaths::from_entries(declared_forbidden_paths(&input)),
        forbidden_paths(&universe(), &ids(&["TASK-001", "TASK-002"]))
    );
    assert_eq!(
        forbidden_path_roots(&input, Some("/iso/i")),
        vec![
            "/iso/i".to_string(),
            "/repo".to_string(),
            "/project".to_string()
        ]
    );
    assert_eq!(
        forbidden_path_roots(&input, Some("/repo")),
        vec!["/repo", "/project"]
    );
    assert!(declared_forbidden_paths(&serde_json::json!({})).is_empty());
}

#[test]
fn the_rejection_has_the_semantic_review_shape_and_names_the_paths() {
    let changed = vec!["src/gate.rs".to_string(), "src/coverage.rs".to_string()];
    let result = forbidden_rejection_result("item-a", &ids(&["TASK-001"]), &changed);
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.summary,
        "write item 'item-a' changed 2 path(s) the task forbids: src/gate.rs, src/coverage.rs; \
         the patch was not captured"
    );
    let gap = &result.residual_gaps[0];
    assert_eq!(gap.id, format!("{FORBIDDEN_PATH_CHANGED_GAP_PREFIX}item-a"));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(
        gap.description
            .contains("Restore each of these to the baseline"),
        "{gap:?}"
    );
    assert_eq!(result.data["failure_kind"], serde_json::json!("semantic"));
    assert_eq!(
        result.data["branch_error_from_runtime"],
        serde_json::json!(true)
    );
    assert_eq!(
        result.data["forbidden_paths_changed"],
        serde_json::json!(changed)
    );
    assert_eq!(
        result.data["canonical_task_ids"],
        serde_json::json!(["TASK-001"])
    );
    assert_eq!(result.data["patch_landed"], serde_json::json!(false));
}

/// A canonical repository with one commit holding `files`, and a sealed
/// worktree for an item declaring `targets` — the grant's own harness.
fn sealed(dir: &Path, files: &[(&str, &str)], targets: &[&str]) -> WritePlan {
    let canonical = dir.join("canonical");
    std::fs::create_dir_all(&canonical).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "t"],
        vec!["config", "user.email", "t@example.invalid"],
    ] {
        run_git(&args, &canonical).unwrap();
    }
    for (path, content) in files {
        let target = canonical.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    run_git(&["add", "."], &canonical).unwrap();
    run_git(&["commit", "-qm", "baseline"], &canonical).unwrap();
    let plan = WritePlan {
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: "item-a".into(),
        canonical_root: canonical.clone(),
        isolated_root: dir.join("iso").join("item-a"),
        target_files: targets
            .iter()
            .map(|path| normalize_target(path, &canonical).unwrap())
            .collect(),
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    };
    let baseline =
        capture_canonical_baseline(&canonical, &plan, &[], &WriteCoordinatorConfig::default())
            .unwrap();
    create_item_workspace(&canonical, &plan, &baseline).unwrap();
    plan
}

fn write(plan: &WritePlan, rel: &str, content: &str) {
    let target = plan.isolated_root.join(rel);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, content).unwrap();
}

fn reported(paths: &[&str]) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        files_changed: paths
            .iter()
            .map(|p| WorkflowV2FileRecord::new(*p))
            .collect(),
        ..Default::default()
    }
}

const BASELINE: &[(&str, &str)] = &[
    ("src/owned.rs", "// owned\n"),
    ("src/gate.rs", "// gate\n"),
    ("src/coverage.rs", "// coverage\n"),
    ("src/unrelated.rs", "// unrelated\n"),
    ("docs/a.md", "# a\n"),
];

/// The live shape: an in-scope, unclaimed, real change to a forbidden file.
/// It lands in `forbidden`, never in `granted`; the untouched forbidden
/// sibling is not named; the in-scope unclaimed non-forbidden change is
/// still granted as before.
#[test]
fn a_changed_forbidden_path_is_forbidden_not_granted_and_an_unchanged_one_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), BASELINE, &["src/owned.rs"]);
    write(&plan, "src/owned.rs", "// implemented\n");
    write(&plan, "src/gate.rs", "// gate edited\n");
    write(&plan, "src/unrelated.rs", "// sibling\n");
    let wave = vec![WaveClaim::new("item-a", ["src/owned.rs".to_string()])];
    let forbidden = ForbiddenPaths::from_entries(["src/gate.rs", "coverage.rs"]);
    let grant = ScopeGrant::resolve(
        &plan,
        &reported(&["src/owned.rs", "src/gate.rs", "src/unrelated.rs"]),
        Some(&wave),
        &forbidden,
    );
    assert_eq!(grant.forbidden, vec!["src/gate.rs".to_string()]);
    assert!(
        !grant.granted.contains(&"src/gate.rs".to_string()),
        "{:?}",
        grant.granted
    );
    assert!(
        grant.granted.contains(&"src/unrelated.rs".to_string()),
        "{:?}",
        grant.granted
    );
}

/// Declared AND forbidden: the DECLARATION wins — a declared target edited
/// by its own item is never forbidden, whether or not it changed — and the
/// overlap is recorded in `forbidden_declared`. An undeclared forbidden
/// change is still judged on the worktree scan (an over-reported one with
/// no diff is not a change), and an out-of-scope one is dropped, not judged.
#[test]
fn a_declared_target_is_never_forbidden_and_the_overlap_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), BASELINE, &["src/owned.rs", "src/gate.rs"]);
    let forbidden = ForbiddenPaths::from_entries(["src/gate.rs", "coverage.rs", "docs/"]);
    write(&plan, "src/owned.rs", "// implemented\n");
    let grant = ScopeGrant::resolve(
        &plan,
        &reported(&["src/owned.rs", "src/gate.rs", "src/coverage.rs"]),
        None,
        &forbidden,
    );
    assert!(grant.forbidden.is_empty(), "{:?}", grant.forbidden);
    assert_eq!(grant.forbidden_declared, vec!["src/gate.rs".to_string()]);
    write(&plan, "src/gate.rs", "// gate edited\n");
    write(&plan, "src/coverage.rs", "// coverage edited\n");
    write(&plan, "docs/a.md", "# edited\n");
    let grant = ScopeGrant::resolve(&plan, &reported(&["src/owned.rs"]), None, &forbidden);
    assert_eq!(
        grant.forbidden,
        vec!["src/coverage.rs".to_string()],
        "the declared target must not be judged forbidden; the undeclared one must be"
    );
    assert_eq!(grant.out_of_scope, vec!["docs/a.md".to_string()]);
    assert_eq!(grant.forbidden_declared, vec!["src/gate.rs".to_string()]);
    assert!(
        grant.unreported.contains(&"src/gate.rs".to_string()),
        "an unreported declared change is still unreported: {:?}",
        grant.unreported
    );
}

/// A declared directory scope the list names is a conflict too, and the
/// gap names every overlapping declaration once, whatever the status.
#[test]
fn the_conflict_gap_names_the_declared_paths_and_is_silent_without_overlap() {
    let mut result = WorkflowV2Result {
        data: serde_json::json!({}),
        ..WorkflowV2Result::default()
    };
    report_forbidden_declared_conflict(&mut result, "item-a", &[]);
    assert!(result.residual_gaps.is_empty());
    assert!(result.data.get("forbidden_declared_conflict").is_none());
    let conflicting = vec!["src/cmd".to_string(), "src/gate.rs".to_string()];
    report_forbidden_declared_conflict(&mut result, "item-a", &conflicting);
    let gap = &result.residual_gaps[0];
    assert_eq!(
        gap.id,
        format!("{FORBIDDEN_DECLARED_CONFLICT_GAP_PREFIX}item-a")
    );
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert_eq!(
        gap.description,
        "write item 'item-a' has 2 path(s) declared and forbidden at once by the task text; \
         the declaration was honoured — check the task wording: src/cmd, src/gate.rs"
    );
    assert_eq!(
        result.data["forbidden_declared_conflict"],
        serde_json::json!(conflicting)
    );
    let dir = tempfile::tempdir().unwrap();
    let mut plan = sealed(dir.path(), BASELINE, &["src/owned.rs"]);
    plan.target_dir_scopes = vec![normalize_target("docs", &plan.canonical_root).unwrap()];
    let forbidden = ForbiddenPaths::from_entries(["docs/", "src/owned.rs"]);
    let grant = ScopeGrant::resolve(&plan, &reported(&[]), None, &forbidden);
    assert_eq!(
        grant.forbidden_declared,
        vec!["docs".to_string(), "src/owned.rs".to_string()]
    );
}

//! The grant against a real sealed worktree (Issue-16): candidates are what
//! the agent CHANGED, not what it reported.
//!
//! Live on wf-719ff3b0 `agents-5-0` the worktree held twenty-two changed
//! files, the envelope listed fourteen, nobody else claimed the rest, and
//! gate 3 refused the first unlisted one as an undeclared write. These resolve
//! the grant over a git worktree, so the scan the grant reads is exercised
//! rather than the envelope-only fallback the in-memory fixtures hit.

use std::path::Path;

use archon_write_plan::{TargetFilesSource, WritePlan, normalize_target};

use super::ScopeGrant;
use crate::v2::write_scope_extension::WaveClaim;
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace, run_git,
};
use crate::{WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status};

fn git(root: &Path, args: &[&str]) {
    run_git(args, root).expect("git");
}

/// A canonical repository with one commit holding `files`, and a sealed
/// worktree for an item declaring `targets`.
fn sealed(dir: &Path, item_id: &str, files: &[(&str, &str)], targets: &[&str]) -> WritePlan {
    let canonical = dir.join("canonical");
    std::fs::create_dir_all(&canonical).unwrap();
    git(&canonical, &["init", "-q"]);
    git(&canonical, &["config", "user.name", "t"]);
    git(&canonical, &["config", "user.email", "t@example.invalid"]);
    for (path, content) in files {
        let target = canonical.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&canonical, &["add", "."]);
    git(&canonical, &["commit", "-qm", "baseline"]);
    let plan = WritePlan {
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: item_id.into(),
        canonical_root: canonical.clone(),
        isolated_root: dir.join("iso").join(item_id),
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
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(&canonical, &plan, &[], &cfg).unwrap();
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

fn declared(plan: &WritePlan) -> Vec<String> {
    plan.target_files
        .iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

const BASELINE: &[(&str, &str)] = &[
    ("owned.txt", "baseline\n"),
    ("other.txt", "other baseline\n"),
    ("src/formatted.txt", "fn f() {\n    1\n}\n"),
];

/// (a) The live failure in miniature: a real change to an unclaimed file the
/// envelope never named, inside the scope roots the declared targets reach
/// (`src/`). Granted — declared in the widened plan, so every gate accepts
/// it — and named as unreported for the reviewer.
#[test]
fn an_unreported_unclaimed_real_change_is_granted_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(
        dir.path(),
        "item-a",
        BASELINE,
        &["owned.txt", "src/owned.txt"],
    );
    write(&plan, "owned.txt", "implemented\n");
    write(&plan, "src/forgotten.txt", "also needed\n");
    let wave = vec![WaveClaim::new("item-a", ["owned.txt".to_string()])];
    let grant = ScopeGrant::resolve(&plan, &reported(&["owned.txt"]), Some(&wave));
    assert_eq!(grant.granted, vec!["src/forgotten.txt".to_string()]);
    assert!(declared(&grant.plan).contains(&"src/forgotten.txt".to_string()));
    assert!(grant.is_granted("src/forgotten.txt"));
    assert!(grant.covers("src/forgotten.txt"));
    assert_eq!(grant.unreported, vec!["src/forgotten.txt".to_string()]);
    assert!(grant.whitespace_only.is_empty());
    assert!(grant.out_of_scope.is_empty());
}

/// (f) Issue-27: the same real, unclaimed, unreported change OUTSIDE the
/// scope roots — the plan declares only a root-level file, so `src/` is not
/// its to change. Not granted even with nothing contesting it, not declared,
/// still counted as unreported; the drop restores the modified file and
/// removes the created one, and the manifest-bound plan never names either.
#[test]
fn an_out_of_scope_real_change_is_dropped_not_granted_even_uncontested() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), "item-a", BASELINE, &["owned.txt"]);
    write(&plan, "owned.txt", "implemented\n");
    write(&plan, "src/formatted.txt", "fn f() {\n    2\n}\n");
    write(&plan, "lib/created.txt", "new crate\n");
    write(&plan, "forgotten.txt", "root-level, always in scope\n");
    let wave = vec![WaveClaim::new("item-a", ["owned.txt".to_string()])];
    let grant = ScopeGrant::resolve(&plan, &reported(&["owned.txt"]), Some(&wave));
    assert_eq!(
        grant.out_of_scope,
        vec![
            "lib/created.txt".to_string(),
            "src/formatted.txt".to_string()
        ]
    );
    assert_eq!(grant.granted, vec!["forgotten.txt".to_string()]);
    assert!(!grant.covers("src/formatted.txt"));
    assert!(!grant.covers("lib/created.txt"));
    assert!(grant.is_out_of_scope("src/formatted.txt"));
    assert!(grant.is_out_of_scope(plan.isolated_root.join("lib/created.txt").to_str().unwrap()));
    assert!(!grant.is_out_of_scope("forgotten.txt"));
    assert_eq!(
        grant.unreported,
        vec![
            "forgotten.txt".to_string(),
            "lib/created.txt".to_string(),
            "src/formatted.txt".to_string()
        ]
    );
    assert!(
        !declared(&grant.plan)
            .iter()
            .any(|p| p.starts_with("src/") || p.starts_with("lib/"))
    );
    assert_eq!(grant.roots.describe(), "owned.txt");
    assert_eq!(grant.drop_out_of_scope_changes(), grant.out_of_scope);
    assert_eq!(
        std::fs::read_to_string(plan.isolated_root.join("src/formatted.txt")).unwrap(),
        "fn f() {\n    1\n}\n"
    );
    assert!(!plan.isolated_root.join("lib/created.txt").exists());
    assert_eq!(
        std::fs::read_to_string(plan.isolated_root.join("forgotten.txt")).unwrap(),
        "root-level, always in scope\n"
    );
}

/// An out-of-scope path the envelope names but never changed is ignored by
/// gate 1 like any other out-of-scope entry, and is NOT reported as dropped:
/// there was nothing to restore.
#[test]
fn an_over_reported_out_of_scope_path_is_not_claimed_as_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), "item-a", BASELINE, &["owned.txt"]);
    write(&plan, "owned.txt", "implemented\n");
    let wave = vec![WaveClaim::new("item-a", ["owned.txt".to_string()])];
    let grant = ScopeGrant::resolve(
        &plan,
        &reported(&["owned.txt", "src/formatted.txt"]),
        Some(&wave),
    );
    assert_eq!(grant.out_of_scope, vec!["src/formatted.txt".to_string()]);
    assert!(grant.granted.is_empty());
    assert!(grant.drop_out_of_scope_changes().is_empty());
}

/// (b) The same unreported file, claimed by the other item: contested, so not
/// granted — gate 2 refuses it exactly as today — but still named.
#[test]
fn an_unreported_contested_change_is_not_granted() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), "item-a", BASELINE, &["owned.txt"]);
    write(&plan, "owned.txt", "implemented\n");
    write(&plan, "other.txt", "trespass\n");
    let wave = vec![
        WaveClaim::new("item-a", ["owned.txt".to_string()]),
        WaveClaim::new("item-b", ["other.txt".to_string()]),
    ];
    let grant = ScopeGrant::resolve(&plan, &reported(&["owned.txt"]), Some(&wave));
    assert!(grant.granted.is_empty());
    assert_eq!(declared(&grant.plan), declared(&plan));
    assert!(!grant.covers("other.txt"));
    assert_eq!(grant.unreported, vec!["other.txt".to_string()]);
}

/// (c) An unreported whitespace-only change is dropped (Issue-13), never
/// granted, and not under-reporting either: it is restored, not judged.
#[test]
fn an_unreported_whitespace_only_change_is_dropped_not_granted() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), "item-a", BASELINE, &["owned.txt"]);
    write(&plan, "owned.txt", "implemented\n");
    write(&plan, "src/formatted.txt", "fn f() {\n\t1\n}\n\n");
    let wave = vec![WaveClaim::new("item-a", ["owned.txt".to_string()])];
    let grant = ScopeGrant::resolve(&plan, &reported(&["owned.txt"]), Some(&wave));
    assert!(grant.granted.is_empty());
    assert_eq!(grant.whitespace_only, vec!["src/formatted.txt".to_string()]);
    assert!(grant.unreported.is_empty());
    assert_eq!(
        grant.drop_whitespace_only_changes(),
        vec!["src/formatted.txt".to_string()]
    );
    assert_eq!(
        std::fs::read_to_string(plan.isolated_root.join("src/formatted.txt")).unwrap(),
        "fn f() {\n    1\n}\n"
    );
}

/// A declared file changed but not listed is under-reporting too — named,
/// and nothing to grant. A listed file with no diff (over-reporting) is
/// neither: harmless, as today.
#[test]
fn a_declared_unreported_change_is_named_and_an_undiffed_report_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), "item-a", BASELINE, &["owned.txt"]);
    write(&plan, "owned.txt", "implemented\n");
    let wave = vec![WaveClaim::new("item-a", ["owned.txt".to_string()])];
    let grant = ScopeGrant::resolve(&plan, &reported(&["other.txt"]), Some(&wave));
    assert_eq!(grant.unreported, vec!["owned.txt".to_string()]);
    assert!(grant.whitespace_only.is_empty());
    // The over-reported unclaimed path is a candidate like any other and is
    // granted, as it was before: there is no diff to capture for it.
    assert_eq!(grant.granted, vec!["other.txt".to_string()]);
}

/// The worktree path the envelope names — by the worktree root, the
/// canonical root, or repo-relative — is the same file the scan found, so
/// naming it any of those ways is reporting it.
#[test]
fn a_change_reported_by_either_root_is_not_unreported() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(
        dir.path(),
        "item-a",
        BASELINE,
        &["owned.txt", "src/owned.txt"],
    );
    write(&plan, "owned.txt", "implemented\n");
    write(&plan, "src/forgotten.txt", "also needed\n");
    let by_worktree = plan.isolated_root.join("owned.txt");
    let by_canonical = plan.canonical_root.join("src/forgotten.txt");
    let wave = vec![WaveClaim::new("item-a", ["owned.txt".to_string()])];
    let grant = ScopeGrant::resolve(
        &plan,
        &reported(&[
            by_worktree.to_str().unwrap(),
            by_canonical.to_str().unwrap(),
        ]),
        Some(&wave),
    );
    assert!(grant.unreported.is_empty(), "{:?}", grant.unreported);
    assert_eq!(grant.granted, vec!["src/forgotten.txt".to_string()]);
}

/// (e) The live shape from wf-719ff3b0 `agents-5-0`: twenty-two files
/// changed, fourteen reported, eight unreported and unclaimed. Every
/// unreported path is granted and the widened plan covers all twenty-two.
#[test]
fn the_live_fourteen_of_twenty_two_envelope_is_granted_in_full() {
    let changed: Vec<String> = (0..22)
        .map(|i| format!("crates/t/src/f{i:02}.rs"))
        .collect();
    let baseline: Vec<(&str, &str)> = changed.iter().map(|p| (p.as_str(), "// base\n")).collect();
    let declared_targets: Vec<&str> = changed[..10].iter().map(String::as_str).collect();
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path(), "agents-5-0", &baseline, &declared_targets);
    for path in &changed {
        write(&plan, path, "// implemented\n");
    }
    let listed: Vec<&str> = changed[..14].iter().map(String::as_str).collect();
    let wave = vec![WaveClaim::new(
        "agents-5-0",
        declared_targets.iter().map(|p| (*p).to_string()),
    )];
    let grant = ScopeGrant::resolve(&plan, &reported(&listed), Some(&wave));
    assert_eq!(grant.granted, changed[10..].to_vec());
    assert_eq!(grant.unreported, changed[14..].to_vec());
    assert!(grant.whitespace_only.is_empty());
    for path in &changed {
        assert!(grant.covers(path), "{path}");
    }
}

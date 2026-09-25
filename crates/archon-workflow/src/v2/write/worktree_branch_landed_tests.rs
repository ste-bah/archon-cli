//! `patch_landed` against a real repository: a gitignored declared
//! deliverable that moved off the baseline lands; nothing else new does.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{PatchLanding, mark_patch_landed, workspace_patch_landed};
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace,
};
use crate::write_coordinator::write_plan::normalize_target;
use crate::write_coordinator::{
    CanonicalBaseline, ItemId, ItemWorkspace, TargetFilesSource, WriteCoordinatorConfig, WritePlan,
};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fixture {
    _dir: tempfile::TempDir,
    canonical: PathBuf,
    plan: WritePlan,
    baseline: CanonicalBaseline,
    workspace: ItemWorkspace,
}

impl Fixture {
    /// A repository that ignores `docs/`, holds an existing ignored
    /// `docs/report.md`, and gives the branch `targets` to write.
    fn new(targets: &[&str]) -> Self {
        Self::with_canonical(targets, |_| {})
    }

    /// As `new`, with `seed` run on the canonical root before the baseline.
    fn with_canonical(targets: &[&str], seed: impl FnOnce(&Path)) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical = dir.path().join("canonical");
        std::fs::create_dir_all(canonical.join("docs")).expect("mkdir");
        git(&["init", "-q", "-b", "main"], &canonical);
        git(&["config", "user.name", "t"], &canonical);
        git(&["config", "user.email", "t@local"], &canonical);
        std::fs::write(canonical.join(".gitignore"), "docs/\n").expect("gitignore");
        std::fs::write(canonical.join("keep.rs"), "// keep\n").expect("seed");
        std::fs::write(canonical.join("docs/report.md"), "# v1\n").expect("doc");
        git(&["add", ".gitignore", "keep.rs"], &canonical);
        git(&["commit", "-q", "-m", "init"], &canonical);
        seed(&canonical);
        let plan = WritePlan {
            run_id: "run1".into(),
            stage_id: "impl".into(),
            item_id: ItemId::from("impl-0"),
            canonical_root: canonical.clone(),
            isolated_root: dir.path().join("wt/impl-0"),
            target_files: targets
                .iter()
                .map(|target| normalize_target(target, &canonical).expect("normalize"))
                .collect(),
            target_dir_scopes: Vec::new(),
            target_files_source: TargetFilesSource::Item,
            read_context_files: vec![],
            verify_inputs: vec![],
            baseline_id: "git:HEAD".into(),
            workspace_boundary_required: true,
            resource_keys: BTreeSet::new(),
        };
        let cfg = WriteCoordinatorConfig::default();
        let baseline = capture_canonical_baseline(&canonical, &plan, &[], &cfg).expect("baseline");
        let workspace = create_item_workspace(&canonical, &plan, &baseline).expect("workspace");
        Self {
            _dir: dir,
            canonical,
            plan,
            baseline,
            workspace,
        }
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.plan.isolated_root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    fn landing(&self) -> PatchLanding {
        workspace_patch_landed(&self.workspace, &self.plan.target_files, &self.baseline)
    }

    fn landed(&self) -> bool {
        self.landing().any()
    }
}

#[test]
fn an_edit_to_an_existing_ignored_deliverable_lands() {
    let fixture = Fixture::new(&["docs/report.md"]);
    fixture.write("docs/report.md", "# v2, fixed\n");
    assert_eq!(
        fixture.landing(),
        PatchLanding {
            tracked: false,
            ignored: true
        },
        "the edited ignored deliverable is real work, and not git-visible"
    );
    // The materialised child is shared with canonical, as live.
    assert_eq!(
        std::fs::read_to_string(fixture.canonical.join("docs/report.md")).expect("read"),
        "# v2, fixed\n"
    );
}

#[test]
fn a_new_ignored_deliverable_lands() {
    let fixture = Fixture::new(&["docs/new.md"]);
    fixture.write("docs/new.md", "# new\n");
    assert!(fixture.landed());
}

#[test]
fn no_change_and_no_ignored_deliverable_does_not_land() {
    let fixture = Fixture::new(&["keep.rs"]);
    assert!(!fixture.landed());
}

#[test]
fn an_ignored_deliverable_present_but_unchanged_does_not_land() {
    // Captured into `ignored_files` because it exists; presence is not work.
    let fixture = Fixture::new(&["docs/report.md"]);
    assert!(!fixture.landed());
}

#[test]
fn undeclared_ignored_noise_does_not_land() {
    let fixture = Fixture::new(&["docs/report.md"]);
    fixture.write("docs/scratch.log", "tool output\n");
    assert!(!fixture.landed());
}

#[test]
fn undeclared_untracked_noise_does_not_land() {
    // Capture refuses the undeclared path, and a failed capture is "nothing
    // landed" — fail closed.
    let fixture = Fixture::new(&["docs/report.md"]);
    fixture.write("stray.txt", "noise\n");
    assert!(!fixture.landed());
}

#[test]
fn a_tracked_change_still_lands() {
    let fixture = Fixture::new(&["keep.rs"]);
    fixture.write("keep.rs", "// changed\n");
    assert!(fixture.landing().tracked);
}

/// A path granted into the plan after the baseline was sealed has no baseline
/// entry, so its capture pre-hash reads "absent". An untouched ignored file
/// the envelope merely listed must not land on that.
#[test]
fn a_granted_untouched_ignored_path_does_not_land() {
    let fixture = Fixture::new(&["keep.rs"]);
    let mut plan = fixture.plan.clone();
    plan.target_files
        .push(normalize_target("docs/report.md", &fixture.canonical).expect("normalize"));
    let workspace = ItemWorkspace {
        plan: plan.clone(),
        ..fixture.workspace.clone()
    };
    let landing = workspace_patch_landed(&workspace, &plan.target_files, &fixture.baseline);
    assert!(!landing.any(), "{landing:?}");
}

/// `file_meta` does not follow a symlink, so the baseline holds no hash for
/// one; that is not evidence of a change. `normalize_target` resolves a
/// symlinked target to its real path, so the plan is built from a
/// deserialized path here — the one way a symlink reaches the baseline.
#[test]
fn a_symlinked_ignored_target_left_alone_does_not_land() {
    let fixture = Fixture::with_canonical(&["keep.rs"], |canonical| {
        std::os::unix::fs::symlink("report.md", canonical.join("docs/link.md")).expect("symlink");
    });
    let link: crate::write_coordinator::NormalizedPath =
        serde_json::from_value(serde_json::json!("docs/link.md")).expect("path");
    let mut plan = fixture.plan.clone();
    plan.target_files = vec![link];
    let cfg = WriteCoordinatorConfig::default();
    let baseline =
        capture_canonical_baseline(&fixture.canonical, &plan, &[], &cfg).expect("baseline");
    let meta = &baseline.declared_target_meta["docs/link.md"];
    assert!(meta.exists && meta.blake3_hex.is_empty(), "{meta:?}");
    let workspace = ItemWorkspace {
        plan: plan.clone(),
        ..fixture.workspace.clone()
    };
    let landing = workspace_patch_landed(&workspace, &plan.target_files, &baseline);
    assert!(!landing.any(), "{landing:?}");
}

fn marked(landing: PatchLanding, schema_repair_failed: bool) -> crate::WorkflowV2Result {
    let mut result = crate::WorkflowV2Result::accepted("done");
    result.data = serde_json::json!({});
    mark_patch_landed(&mut result, "branch-1", landing, schema_repair_failed);
    result
}

/// Partial work is a git diff, which never carries an ignored path, so the
/// once-per-task refund must not be spent on an ignored-only landing.
#[test]
fn an_ignored_only_landing_is_landed_but_earns_no_schema_refund() {
    let result = marked(
        PatchLanding {
            tracked: false,
            ignored: true,
        },
        true,
    );
    assert_eq!(result.data["patch_landed"], true);
    assert!(result.data.get("schema_repair_patch_landed").is_none());
    assert!(result.residual_gaps.is_empty());
}

#[test]
fn a_tracked_landing_under_schema_failure_earns_the_refund() {
    let result = marked(
        PatchLanding {
            tracked: true,
            ignored: true,
        },
        true,
    );
    assert_eq!(result.data["patch_landed"], true);
    assert_eq!(result.data["schema_repair_patch_landed"], true);
    assert_eq!(result.residual_gaps.len(), 1);
}

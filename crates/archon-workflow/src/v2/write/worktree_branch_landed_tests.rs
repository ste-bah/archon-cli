//! `patch_landed` against a real repository: a gitignored declared
//! deliverable that moved off the baseline lands; nothing else new does.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::workspace_patch_landed;
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

    fn landed(&self) -> bool {
        workspace_patch_landed(&self.workspace, &self.plan.target_files, &self.baseline)
    }
}

#[test]
fn an_edit_to_an_existing_ignored_deliverable_lands() {
    let fixture = Fixture::new(&["docs/report.md"]);
    fixture.write("docs/report.md", "# v2, fixed\n");
    assert!(
        fixture.landed(),
        "the edited ignored deliverable is real work"
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
    assert!(fixture.landed());
}

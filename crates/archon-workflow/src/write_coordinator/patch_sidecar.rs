//! Deliverables git refuses to see, carried beside the patch.
//!
//! The patch pipeline proves a branch's work with `git add --intent-to-add`
//! followed by `git diff` — and git refuses to stage a path covered by
//! `.gitignore`, erroring rather than skipping. A generated task that declares
//! its deliverable inside an ignored directory (a user-docs report, generated
//! data) therefore used to fail its whole branch at capture, and one failed
//! branch failed its wave: fifteen tasks reported `missing` because one audit
//! file lived under an ignored `docs/` path.
//!
//! Forcing the add (`-f`) would be worse, not better: the directory is ignored
//! *deliberately* — those files must never enter history — and an apply that
//! stages them would eventually commit them.
//!
//! So an ignored declared target bypasses git entirely. Capture reads its
//! bytes; persist writes them in a sidecar directory beside the patch; apply
//! copies them into the canonical tree as plain files, where they remain
//! ignored exactly as intended. Existence and content hash — which the
//! manifest's `post_hashes` already records for every declared target — are
//! the proof, standing in for the diff that cannot exist.
//!
//! Only *declared* targets ever reach this path: the undeclared-write check
//! runs before capture, so the sidecar cannot smuggle scope the plan never
//! granted.

use std::fs;
use std::path::{Path, PathBuf};

use super::patch_manifest::PatchError;

/// Sidecar directory for a patch at `<stage>/patches/<item>.patch`:
/// `<stage>/patches/<item>.ignored/`. Derived from the patch path so the
/// manifest schema — which downstream prompts enumerate field-for-field —
/// does not grow a field.
pub(super) fn sidecar_dir(patch_path: &Path) -> PathBuf {
    patch_path.with_extension("ignored")
}

/// Write each `(relative path, bytes)` under the sidecar directory.
pub(super) fn persist(
    patch_path: &Path,
    ignored_files: &[(String, Vec<u8>)],
) -> Result<(), PatchError> {
    if ignored_files.is_empty() {
        return Ok(());
    }
    let root = sidecar_dir(patch_path);
    for (rel, bytes) in ignored_files {
        let dest = root.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|source| PatchError::PersistFailed { source })?;
        }
        fs::write(&dest, bytes).map_err(|source| PatchError::PersistFailed { source })?;
    }
    Ok(())
}

/// Copy every sidecar file into the canonical tree, returning the relative
/// paths copied. A missing sidecar directory is the normal case — most patches
/// have no ignored deliverables — and returns empty rather than erroring.
pub(super) fn apply(patch_path: &Path, canonical_root: &Path) -> std::io::Result<Vec<String>> {
    let root = sidecar_dir(patch_path);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut copied = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .expect("walked from root")
                .to_string_lossy()
                .into_owned();
            let dest = canonical_root.join(&rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&path, &dest)?;
            copied.push(rel);
        }
    }
    copied.sort();
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_nested_ignored_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let patch = dir.path().join("stage/patches/item-1.patch");
        fs::create_dir_all(patch.parent().unwrap()).unwrap();
        persist(
            &patch,
            &[("docs/generated/report.md".to_string(), b"body".to_vec())],
        )
        .expect("persist");

        let canonical = dir.path().join("canonical");
        fs::create_dir_all(&canonical).unwrap();
        let copied = apply(&patch, &canonical).expect("apply");
        assert_eq!(copied, vec!["docs/generated/report.md".to_string()]);
        assert_eq!(
            fs::read_to_string(canonical.join("docs/generated/report.md")).unwrap(),
            "body"
        );
    }

    /// No sidecar is the overwhelmingly normal case and must cost nothing.
    #[test]
    fn absent_sidecar_applies_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let patch = dir.path().join("item.patch");
        let copied = apply(&patch, dir.path()).expect("apply");
        assert!(copied.is_empty());
    }
}

/// End-to-end: an ignored declared target must survive capture, satisfy the
/// gauntlet, and reach the canonical tree — the exact live failure where one
/// audit doc under an ignored `docs/` path failed its branch and, with it, a
/// fifteen-task wave.
#[cfg(test)]
mod capture_e2e {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::super::patch_manifest::{capture_patch, validate_patch};
    use super::super::worktree_isolation::{capture_canonical_baseline, create_item_workspace};
    use super::super::write_plan::{TargetFilesSource, normalize_target};
    use super::super::{ItemId, WriteCoordinatorConfig, WritePlan};

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

    #[test]
    fn ignored_declared_target_is_captured_and_reaches_canonical() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git(&["init", "-q", "-b", "main"], root);
        git(&["config", "user.name", "t"], root);
        git(&["config", "user.email", "t@local"], root);
        // The repo deliberately ignores the docs directory: user-created
        // content that must never enter history.
        std::fs::write(root.join(".gitignore"), "docs/\n").expect("gitignore");
        std::fs::write(root.join("keep.rs"), "// keep\n").expect("seed");
        git(&["add", ".gitignore", "keep.rs"], root);
        git(&["commit", "-q", "-m", "init"], root);

        let plan = WritePlan {
            run_id: "run1".into(),
            stage_id: "impl".into(),
            item_id: ItemId::from("impl-0"),
            canonical_root: root.to_path_buf(),
            isolated_root: root.join(".archon/wc/run1/impl-0"),
            target_files: vec![normalize_target("docs/report.md", root).expect("normalize")],
            target_dir_scopes: Vec::new(),
            target_files_source: TargetFilesSource::Item,
            read_context_files: vec![],
            verify_inputs: vec![],
            baseline_id: "git:HEAD".into(),
            workspace_boundary_required: true,
            resource_keys: BTreeSet::new(),
        };
        let cfg = WriteCoordinatorConfig::default();
        let baseline = capture_canonical_baseline(root, &plan, &[], &cfg).expect("baseline");
        let ws = create_item_workspace(root, &plan, &baseline).expect("workspace");

        std::fs::create_dir_all(plan.isolated_root.join("docs")).expect("mkdir");
        std::fs::write(plan.isolated_root.join("docs/report.md"), "# audit\n").expect("write");

        // Capture must not fail on the ignored path — this call used to die in
        // `git add --intent-to-add` with "paths are ignored by one of your
        // .gitignore files".
        let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
        assert!(
            captured.patch_bytes.is_empty(),
            "no git-visible diff exists"
        );
        assert_eq!(captured.ignored_files.len(), 1, "carried as bytes instead");
        assert_eq!(captured.ignored_files[0].0, "docs/report.md");

        // The gauntlet must read this as work done, not a missing noop claim.
        validate_patch(&captured, &plan, &cfg, "wrote the audit report")
            .expect("an ignored deliverable is not an empty patch");

        // And the sidecar delivers it to the canonical tree.
        let patch_path = root.join("stage/patches/impl-0.patch");
        std::fs::create_dir_all(patch_path.parent().unwrap()).expect("mkdir");
        super::persist(&patch_path, &captured.ignored_files).expect("persist");
        let copied = super::apply(&patch_path, root).expect("apply");
        assert_eq!(copied, vec!["docs/report.md".to_string()]);
        assert_eq!(
            std::fs::read_to_string(root.join("docs/report.md")).expect("read"),
            "# audit\n"
        );
    }
}

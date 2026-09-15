//! The scope ceiling over tempdir repositories: which roots a plan derives
//! from its declared targets and the repository's package markers, and what
//! those roots cover.

use std::path::Path;

use archon_write_plan::{TargetFilesSource, WritePlan, normalize_target};

use super::{ScopeRoots, scope_roots};

/// A repository holding `files` (content irrelevant) and a plan declaring
/// `targets` and `scopes` against it.
fn plan(root: &Path, files: &[&str], targets: &[&str], scopes: &[&str]) -> WritePlan {
    for path in files {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "x\n").unwrap();
    }
    WritePlan {
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: "item".into(),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join("../iso"),
        target_files: targets
            .iter()
            .map(|path| normalize_target(path, root).unwrap())
            .collect(),
        target_dir_scopes: scopes
            .iter()
            .map(|path| normalize_target(path, root).unwrap())
            .collect(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    }
}

fn roots(root: &Path, files: &[&str], targets: &[&str], scopes: &[&str]) -> ScopeRoots {
    scope_roots(&plan(root, files, targets, scopes))
}

/// The live shape (Issue-27): targets in two crates and `src/`, the crates
/// marked by `Cargo.toml`, `src/` unmarked. The crates are roots, `src/` is
/// the top-level fallback, and a sibling crate is NOT covered.
#[test]
fn a_package_manifest_below_the_root_makes_the_package_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(
        dir.path(),
        &[
            "Cargo.toml",
            "crates/archon-trading/Cargo.toml",
            "crates/archon-tui/Cargo.toml",
            "crates/archon-workflow/Cargo.toml",
        ],
        &[
            "crates/archon-trading/src/coverage.rs",
            "crates/archon-trading/src/lib.rs",
            "crates/archon-tui/src/commands.rs",
            "src/command/trading_data.rs",
        ],
        &[],
    );
    assert_eq!(
        roots.describe(),
        "crates/archon-trading/, crates/archon-tui/, src/"
    );
    assert!(roots.covers("crates/archon-trading/tests/coverage.rs"));
    assert!(roots.covers("crates/archon-tui/src/lib.rs"));
    assert!(roots.covers("src/main.rs"));
    assert!(!roots.covers("crates/archon-workflow/src/lib.rs"));
    assert!(!roots.covers("crates/archon-knowledge/src/lib.rs"));
    assert!(!roots.covers("crates/other.rs"));
}

/// The repository root's own manifest is never a package root: the walk
/// stops strictly below it, so the top-level directory is the fallback.
#[test]
fn without_a_manifest_below_the_root_the_top_level_directory_is_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(
        dir.path(),
        &["Cargo.toml", "src/command/x.rs"],
        &["src/command/x.rs"],
        &[],
    );
    assert_eq!(roots.describe(), "src/");
    assert!(roots.covers("src/other/y.rs"));
    assert!(!roots.covers("tests/z.rs"));
}

/// A declared file at the repository root is an exact-file root, and every
/// root-level candidate is covered whether declared or not.
#[test]
fn a_root_level_declared_file_is_exact_and_root_level_candidates_are_always_covered() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(dir.path(), &["owned.txt"], &["owned.txt"], &[]);
    assert_eq!(roots.describe(), "owned.txt");
    assert!(roots.covers("owned.txt"));
    assert!(roots.covers("Cargo.lock"));
    assert!(roots.covers("forgotten.txt"));
    assert!(!roots.covers("src/anything.rs"));
}

/// Under a crate, a manifest in an intermediate directory (a nested package)
/// is the nearest one: the walk starts at the declared file's own directory.
#[test]
fn the_nearest_manifest_wins() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(
        dir.path(),
        &["crates/x/Cargo.toml", "crates/x/plugin/package.json"],
        &["crates/x/plugin/src/index.js"],
        &[],
    );
    assert_eq!(roots.describe(), "crates/x/plugin/");
    assert!(!roots.covers("crates/x/src/lib.rs"));
}

/// A directory scope is its own root when nothing packages it, and its
/// package root when something does.
#[test]
fn a_directory_scope_contributes_itself_or_its_package_root() {
    let dir = tempfile::tempdir().unwrap();
    let unpackaged = roots(dir.path(), &["src/gen/a.rs"], &[], &["src/gen"]);
    assert_eq!(unpackaged.describe(), "src/gen/");
    assert!(unpackaged.covers("src/gen/b.rs"));
    assert!(!unpackaged.covers("src/other.rs"));
    let packaged = roots(
        dir.path(),
        &["crates/x/Cargo.toml", "crates/x/src/gen/a.rs"],
        &[],
        &["crates/x/src/gen"],
    );
    assert_eq!(packaged.describe(), "crates/x/");
    assert!(packaged.covers("crates/x/src/lib.rs"));
}

/// Every manifest name marks a package, and a `*.csproj` does too.
#[test]
fn every_manifest_name_marks_a_package() {
    let dir = tempfile::tempdir().unwrap();
    for (i, manifest) in super::PACKAGE_MANIFESTS
        .iter()
        .copied()
        .chain(["App.csproj"])
        .enumerate()
    {
        let pkg = format!("pkgs/p{i}");
        let roots = roots(
            dir.path(),
            &[&format!("{pkg}/{manifest}")],
            &[&format!("{pkg}/src/deep/file")],
            &[],
        );
        assert_eq!(roots.describe(), format!("{pkg}/"), "{manifest}");
    }
}

/// A root nested under another is pruned from what the agent is told, and
/// coverage is unchanged by it.
#[test]
fn a_nested_root_is_pruned_from_the_description() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(
        dir.path(),
        &["crates/x/Cargo.toml"],
        &["crates/x/src/lib.rs"],
        &["crates/x/src/gen", "docs/api"],
    );
    assert_eq!(roots.describe(), "crates/x/, docs/api/");
    assert!(roots.covers("crates/x/src/gen/a.rs"));
}

/// Nothing declared, no ceiling: the grant behaves as it did before.
#[test]
fn an_undeclared_plan_covers_everything() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(dir.path(), &[], &[], &[]);
    assert!(roots.is_empty());
    assert!(roots.covers("crates/anything/src/lib.rs"));
    assert_eq!(roots.preamble(), "");
}

/// The prefix rule is boundary-aware, as `path_is_planned` is.
#[test]
fn coverage_is_boundary_aware() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(dir.path(), &[], &["src/a.rs"], &[]);
    assert!(roots.covers("src/b.rs"));
    assert!(!roots.covers("srcs/b.rs"));
    assert!(!roots.covers("src-old/b.rs"));
}

/// A canonical root that does not exist on disk has no manifests: the
/// top-level fallback applies, which is what the in-memory grant fixtures
/// rely on.
#[test]
fn a_missing_canonical_root_falls_back_to_top_level_directories() {
    let plan = plan(
        Path::new("/nonexistent/repo"),
        &[],
        &["crates/t/src/lib.rs"],
        &[],
    );
    let roots = scope_roots(&plan);
    assert_eq!(roots.describe(), "crates/");
    assert!(roots.covers("crates/other/src/lib.rs"));
}

/// The preamble names the roots and the rule in one sentence.
#[test]
fn the_preamble_names_the_roots() {
    let dir = tempfile::tempdir().unwrap();
    let roots = roots(dir.path(), &[], &["src/a.rs"], &[]);
    let text = roots.preamble();
    assert!(text.starts_with("\nScope roots: src/. "), "{text}");
    assert!(text.contains("discarded before capture"), "{text}");
}

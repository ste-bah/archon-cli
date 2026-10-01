use super::*;
use crate::check_source_pins::{BlobStore, ORIGIN_FREEZE, pin_contract};

fn contract(checks: &[(&str, &str)]) -> crate::task_set_contract::AcceptanceContract {
    crate::check_source_pins::tests_support::contract(checks)
}

fn no_inputs(_: &str) -> bool {
    false
}

fn view<'a>(worktree: &'a Path, project: &'a Path) -> LandingView<'a> {
    LandingView {
        worktree,
        project,
        project_input: &no_inputs,
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn package(root: &Path) {
    write(root, "Cargo.toml", "[package]\nname = \"pkg\"\n");
    write(
        root,
        "src/lib.rs",
        "pub fn f() -> u8 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn keeps() { assert_eq!(super::f(), 1); }\n}\n",
    );
    write(root, "tests/it.rs", "#[test]\nfn it() { assert!(true); }\n");
}

#[test]
fn acceptance_drift_names_edits_creations_and_new_specific_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    package(root);
    let roots = Roots {
        repository: root,
        project: root,
    };
    let blobs = BlobStore::at(root.join(".blobs"));
    let c = contract(&[
        ("AC-1", "cargo test --test it"),
        ("AC-2", "cargo test keeps"),
        ("AC-3", "cargo test --test later"),
        ("AC-4", "cargo test"),
    ]);
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &blobs);
    assert!(tree_drift(&pins, &roots).is_empty());
    // The implementation changes, the unit test does not: no drift.
    write(
        root,
        "src/lib.rs",
        "pub fn f() -> u8 { 2 - 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn keeps() { assert_eq!(super::f(), 1); }\n\n    #[test]\n    fn added() {}\n}\n",
    );
    assert!(
        tree_drift(&pins, &roots).is_empty(),
        "an added suite test only adds"
    );
    // A weakened unit test, a weakened target, a created absent target.
    write(
        root,
        "src/lib.rs",
        "pub fn f() -> u8 { 3 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn keeps() {}\n}\n",
    );
    write(root, "tests/it.rs", "#[test]\nfn it() {}\n");
    write(root, "tests/later.rs", "#[test]\nfn later() {}\n");
    let drift = tree_drift(&pins, &roots);
    let labels: Vec<(String, Vec<String>)> = drift
        .iter()
        .map(|c| (c.label(), c.check_ids.iter().cloned().collect()))
        .collect();
    assert_eq!(
        labels,
        [
            (
                "src/lib.rs (fn:tests::keeps)".into(),
                vec!["AC-2".into(), "AC-4".into()]
            ),
            ("tests/it.rs".into(), vec!["AC-1".into(), "AC-4".into()]),
            ("tests/later.rs".into(), vec!["AC-3".into()]),
        ]
    );
    assert!(drift.iter().all(|c| c.was_pinned));
}

#[test]
fn a_landing_holds_pinned_edits_watched_definitions_and_their_new_modules() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().join("canonical");
    package(&canonical);
    let roots = Roots {
        repository: &canonical,
        project: &canonical,
    };
    let blobs = BlobStore::at(dir.path().join(".blobs"));
    let c = contract(&[
        ("AC-1", "cargo test --test it"),
        ("AC-2", "cargo test not_yet -- --exact"),
    ]);
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &blobs);
    assert_eq!(pins.checks["AC-2"].watches.len(), 1);
    let worktree = dir.path().join("worktree");
    package(&worktree);
    write(
        &worktree,
        "tests/it.rs",
        "mod helper;\n#[test]\nfn it() {}\n",
    );
    write(&worktree, "tests/helper.rs", "pub fn h() {}\n");
    write(
        &worktree,
        "src/lib.rs",
        "pub fn f() -> u8 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn keeps() { assert_eq!(super::f(), 1); }\n    #[test]\n    fn not_yet() {}\n}\n",
    );
    let changed: Vec<String> = ["src/lib.rs", "tests/helper.rs", "tests/it.rs"]
        .map(String::from)
        .to_vec();
    let held = landing_changes(&pins, &changed, &view(&worktree, &worktree));
    let labels: Vec<String> = held.iter().map(SourceChange::label).collect();
    assert_eq!(
        labels,
        [
            "src/lib.rs (fn:tests::not_yet)",
            "tests/helper.rs",
            "tests/it.rs"
        ]
    );
    assert!(!held[0].was_pinned && !held[1].was_pinned && held[2].was_pinned);
    assert_eq!(held[1].check_ids, held[2].check_ids);
    // An unrelated change holds nothing.
    let other = vec!["README.md".to_string()];
    assert!(landing_changes(&pins, &other, &view(&worktree, &worktree)).is_empty());
}

/// Item 4: a landing judges a newly resolved source by the same rule as
/// acceptance -- a new conftest beside a pinned pytest directory is the
/// check's own and is held, a new data file there is not.
#[test]
fn a_landing_holds_a_new_conftest_beside_a_pinned_pytest_file() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().join("canonical");
    write(&canonical, "py/tests/test_a.py", "def test_a(): assert 1\n");
    let roots = Roots {
        repository: &canonical,
        project: &canonical,
    };
    let blobs = BlobStore::at(dir.path().join(".blobs"));
    let c = contract(&[("AC-1", "python3 -m pytest py/tests/test_a.py")]);
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &blobs);
    let worktree = dir.path().join("worktree");
    write(&worktree, "py/tests/test_a.py", "def test_a(): assert 1\n");
    write(&worktree, "py/tests/conftest.py", "import pytest\n");
    let changed = vec!["py/tests/conftest.py".to_string()];
    let held = landing_changes(&pins, &changed, &view(&worktree, &canonical));
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].path, "py/tests/conftest.py");
    assert!(!held[0].was_pinned);
}

fn inputs_under_data(path: &str) -> bool {
    path.starts_with(".archon/lab/")
}

/// Item 5: a project-input copy of a pinned project-rooted script, changed
/// in the worktree, is held at landing.
#[test]
fn a_landing_holds_a_changed_project_input_copy_of_a_pinned_script() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, project) = (dir.path().join("repo"), dir.path().join("project"));
    write(&repo, "README.md", "r\n");
    write(&project, ".archon/lab/check.sh", "test -f out || exit 1\n");
    let roots = Roots {
        repository: &repo,
        project: &project,
    };
    let blobs = BlobStore::at(dir.path().join(".blobs"));
    let mut c = contract(&[("AC-1", "bash .archon/lab/check.sh")]);
    if let crate::task_set_contract::AcceptanceCheck::Command { cwd, .. } =
        &mut c.acceptance[0].check
    {
        *cwd = crate::task_set_contract::TrustedCwd::ProjectRoot;
    }
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &blobs);
    assert_eq!(pins.checks["AC-1"].sources[0].root, SourceRoot::Project);
    let worktree = dir.path().join("worktree");
    write(&worktree, "README.md", "r\n");
    write(&worktree, ".archon/lab/check.sh", "exit 0\n");
    let view = LandingView {
        worktree: &worktree,
        project: &project,
        project_input: &inputs_under_data,
    };
    let held = landing_changes(&pins, &[], &view);
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(
        (held[0].root, held[0].path.as_str()),
        (SourceRoot::Project, ".archon/lab/check.sh")
    );
}

/// Review minor 8: a watch on a package at the tree root does not match a
/// test another package below it defines.
#[test]
fn a_root_package_watch_does_not_match_a_nested_package() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().join("canonical");
    package(&canonical);
    write(
        &canonical,
        "crates/other/Cargo.toml",
        "[package]\nname = \"other\"\n",
    );
    let roots = Roots {
        repository: &canonical,
        project: &canonical,
    };
    let blobs = BlobStore::at(dir.path().join(".blobs"));
    let c = contract(&[("AC-1", "cargo test -p pkg not_yet -- --exact")]);
    let pins = pin_contract(&c, "d", &roots, ORIGIN_FREEZE, &blobs);
    assert_eq!(pins.checks["AC-1"].watches[0].excluded, ["crates/other"]);
    let worktree = dir.path().join("worktree");
    package(&worktree);
    write(
        &worktree,
        "crates/other/Cargo.toml",
        "[package]\nname = \"other\"\n",
    );
    write(
        &worktree,
        "crates/other/src/lib.rs",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn not_yet() {}\n}\n",
    );
    let changed = vec!["crates/other/src/lib.rs".to_string()];
    assert!(landing_changes(&pins, &changed, &view(&worktree, &worktree)).is_empty());
    let watch = &pins.checks["AC-1"].watches[0];
    let at = |path: &str| Found {
        root: SourceRoot::Repository,
        path: path.into(),
        item: Some("fn:tests::not_yet".into()),
        role: "unit test function".into(),
    };
    assert!(
        !satisfies(watch, &at("crates/other/src/lib.rs")),
        "another package's test"
    );
    assert!(
        satisfies(watch, &at("src/lib.rs")),
        "the root package's own"
    );
}

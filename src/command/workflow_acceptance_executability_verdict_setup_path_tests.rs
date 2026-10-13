//! Issue 420 (gap 2): a check that fails before implementation only because
//! a path it names for its setup lies outside the tree it ran in, and is
//! absent there, gave no verdict. Each case runs the real command in a real
//! directory and judges its real output.
use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceContract;

use super::super::{Context, no_verdict};

fn contract() -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [], "supplementary": []
    }))
    .unwrap()
}

fn host() -> Context {
    Context::on_host_path(&contract())
}

/// Run `command` with `/bin/sh -c` in `cwd`, as a check runs.
fn real(command: &str, cwd: &Path) -> CheckResult {
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
        .expect("the system shell runs the check");
    CheckResult {
        acceptance_id: "AC-420".into(),
        exit_code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
        environment_note: None,
    }
}

/// `tree/` (the check's working directory, with `sub/`) inside a parent
/// that holds nothing else.
fn tree() -> (tempfile::TempDir, std::path::PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let tree = parent.path().join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    (parent, tree)
}

const ABSENT_ROOT: &str = "../archon-420-absent-root";

#[cfg(unix)]
#[test]
fn a_manifest_path_outside_the_tree_that_is_absent_gives_no_verdict() {
    let (_parent, tree) = tree();
    for command in [
        format!("cargo metadata --no-deps --format-version 1 --manifest-path {ABSENT_ROOT}/Cargo.toml"),
        format!("cargo metadata --no-deps --format-version 1 --manifest-path={ABSENT_ROOT}/Cargo.toml"),
        "cargo metadata --no-deps --format-version 1 --manifest-path /archon-420-absent-root/Cargo.toml".to_string(),
    ] {
        let result = real(&command, &tree);
        assert_ne!(result.exit_code, Some(0), "{command}");
        let output = String::from_utf8_lossy(&result.stderr);
        assert!(output.contains("does not exist"), "{command}: {output}");
        let why = no_verdict(&command, &result, &host())
            .unwrap_or_else(|| panic!("a setup failure counted as a verdict: {command}: {output}"));
        assert!(why.contains("outside the tree"), "{why}");
        assert!(why.contains("Cargo.toml"), "{why}");
    }
}

#[cfg(unix)]
#[test]
fn entering_a_directory_outside_the_tree_that_is_absent_gives_no_verdict() {
    let (_parent, tree) = tree();
    for command in [
        format!("cd {ABSENT_ROOT} && test -f feature.txt"),
        format!("set -e\ncd {ABSENT_ROOT}/crates\ntest -f feature.txt"),
        format!("cd sub && cd ../{ABSENT_ROOT} && test -f feature.txt"),
    ] {
        let result = real(&command, &tree);
        assert_ne!(result.exit_code, Some(0), "{command}");
        let why = no_verdict(&command, &result, &host()).unwrap_or_else(|| {
            panic!(
                "{command}: {}",
                String::from_utf8_lossy(&result.stderr)
            )
        });
        assert!(why.contains("outside the tree"), "{why}");
        assert!(why.contains("archon-420-absent-root"), "{why}");
    }
}

#[cfg(unix)]
#[test]
fn an_absent_path_inside_the_tree_is_the_deliverable_not_there_yet() {
    let (_parent, tree) = tree();
    for command in [
        "cargo metadata --no-deps --format-version 1 --manifest-path crates/new/Cargo.toml",
        "cd crates/new && test -f feature.txt",
        // `sub/..` is the tree itself: the absent crate is a deliverable.
        "cd sub && cargo metadata --no-deps --format-version 1 --manifest-path ../crates/new/Cargo.toml",
    ] {
        let result = real(command, &tree);
        assert_ne!(result.exit_code, Some(0), "{command}");
        assert_eq!(
            no_verdict(command, &result, &host()),
            None,
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn a_path_outside_the_tree_that_exists_leaves_the_assertion_its_verdict() {
    let (_parent, tree) = tree();
    // `..` exists: the check entered it and its own assertion failed.
    let command = "cd .. && test -f archon-420-feature.txt";
    let result = real(command, &tree);
    assert_eq!(result.exit_code, Some(1));
    assert_eq!(no_verdict(command, &result, &host()), None);
    // An absent outside path the output never reports missing (the check
    // silenced it) is not decided by this rule.
    let silenced = format!("cd {ABSENT_ROOT} 2>/dev/null || exit 1");
    let result = real(&silenced, &tree);
    assert_eq!(result.exit_code, Some(1));
    assert_eq!(no_verdict(&silenced, &result, &host()), None);
}

#[cfg(unix)]
#[test]
fn headroom_keeps_a_parent_path_inside_a_tree_whose_project_is_nested() {
    // The project lies one level inside the repository copy: `..` from the
    // project is still the tree, so an absent `../crates/new` is a
    // deliverable, while `../../x` leaves it.
    let (_parent, tree) = tree();
    let project = tree.join("sub");
    let inside = "cargo metadata --no-deps --format-version 1 --manifest-path ../crates/new/Cargo.toml";
    let result = real(inside, &project);
    assert_ne!(result.exit_code, Some(0));
    let nested = host().with_headroom(1);
    assert_eq!(no_verdict(inside, &result, &nested), None);
    assert!(no_verdict(inside, &result, &host()).is_some());
    let outside = "cargo metadata --no-deps --format-version 1 --manifest-path ../../archon-420-absent-root/Cargo.toml";
    let result = real(outside, &project);
    assert!(no_verdict(outside, &result, &nested).is_some());
}

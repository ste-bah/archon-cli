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

/// The live tree: no copy, so the rule never applies.
fn live() -> Context {
    Context::on_host_path(&contract())
}

/// A copy whose top holds only `tree/`, the checks' working directory.
fn host() -> Context {
    live().within(1, &["tree"])
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
        let why = no_verdict(&command, &result, &host())
            .unwrap_or_else(|| panic!("{command}: {}", String::from_utf8_lossy(&result.stderr)));
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
fn a_project_nested_in_its_repository_copy_keeps_parent_paths_inside() {
    // The project lies one level inside the repository copy (`tree/sub`):
    // `..` from the project is still the tree, so an absent `../crates/new`
    // is a deliverable, while `../../x` leaves the copy.
    let (_parent, tree) = tree();
    let project = tree.join("sub");
    let inside =
        "cargo metadata --no-deps --format-version 1 --manifest-path ../crates/new/Cargo.toml";
    let result = real(inside, &project);
    assert_ne!(result.exit_code, Some(0));
    let nested = live().within(2, &["tree"]);
    assert_eq!(no_verdict(inside, &result, &nested), None);
    let outside = "cargo metadata --no-deps --format-version 1 --manifest-path ../../archon-420-absent-root/Cargo.toml";
    let result = real(outside, &project);
    assert!(no_verdict(outside, &result, &nested).is_some());
}

#[cfg(unix)]
#[test]
fn a_root_the_copy_holds_beside_the_working_directory_is_inside() {
    // A copy holding `repo` and `project` (the probe's own copy, the scratch
    // site): from `project`, `../repo/crates/new` is the repository copy, so
    // an absent crate there is a deliverable; `../code-root` is not held.
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(parent.path().join("repo")).unwrap();
    let copy = live().within(1, &["repo", "project"]);
    let sibling =
        "cargo metadata --no-deps --format-version 1 --manifest-path ../repo/crates/new/Cargo.toml";
    let result = real(sibling, &project);
    assert_ne!(result.exit_code, Some(0));
    assert_eq!(no_verdict(sibling, &result, &copy), None);
    let foreign =
        "cargo metadata --no-deps --format-version 1 --manifest-path ../code-root/Cargo.toml";
    let result = real(foreign, &project);
    assert!(no_verdict(foreign, &result, &copy).is_some());
}

#[cfg(unix)]
#[test]
fn on_the_live_tree_no_parent_path_is_judged_a_setup_failure() {
    // Live layout: project beside its repository. `../repo/crates/new` is a
    // real path there and the crate is the deliverable not built yet; an
    // absent root on the live tree is not decided either.
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(parent.path().join("repo")).unwrap();
    for command in [
        "cargo metadata --no-deps --format-version 1 --manifest-path ../repo/crates/new-feature/Cargo.toml",
        "cargo metadata --no-deps --format-version 1 --manifest-path /archon-420-absent-root/Cargo.toml",
        "cd ../archon-420-absent-root && test -f feature.txt",
    ] {
        let result = real(command, &project);
        assert_ne!(result.exit_code, Some(0), "{command}");
        assert_eq!(no_verdict(command, &result, &live()), None, "{command}");
    }
}

#[cfg(unix)]
#[test]
fn git_dash_c_names_a_setup_path_and_tar_dash_c_does_not() {
    let (_parent, tree) = tree();
    let git = format!("git -C {ABSENT_ROOT} status");
    let result = real(&git, &tree);
    assert_ne!(result.exit_code, Some(0));
    let output = String::from_utf8_lossy(&result.stderr).to_string();
    assert!(output.contains(ABSENT_ROOT), "{output}");
    let why = no_verdict(&git, &result, &host()).unwrap_or_else(|| panic!("{output}"));
    assert!(why.contains("(-C)"), "{why}");
    // `tar -C` is the directory tar extracts into, not a setup path here.
    let tar = format!("tar -C {ABSENT_ROOT} -xf archive.tar");
    let result = real(&tar, &tree);
    assert_ne!(result.exit_code, Some(0));
    assert_eq!(no_verdict(&tar, &result, &host()), None);
}

#[cfg(unix)]
#[test]
fn a_quoted_setup_path_is_read_as_one_word() {
    let (_parent, tree) = tree();
    let command = "cd '../archon 420 absent root' && test -f feature.txt";
    let result = real(command, &tree);
    assert_ne!(result.exit_code, Some(0));
    let why = no_verdict(command, &result, &host())
        .unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(&result.stderr)));
    assert!(why.contains("`../archon 420 absent root`"), "{why}");
}

#[cfg(unix)]
#[test]
fn after_cd_dash_relative_paths_are_not_decided_and_pushd_is_followed() {
    let (_parent, tree) = tree();
    // `cd -` goes back to a directory this rule does not track, so a later
    // relative path is not decided: the failure stays a verdict.
    let back = format!("cd sub && cd - && cd {ABSENT_ROOT}");
    let result = real(&back, &tree);
    assert_ne!(result.exit_code, Some(0));
    assert_eq!(no_verdict(&back, &result, &host()), None);
    // `pushd` enters its operand like `cd`. Where the shell has `pushd` and
    // reports the path absent, it is a setup failure; a shell without it
    // (dash: `pushd: not found`) names no path, so no verdict is taken away.
    let push = format!("pushd {ABSENT_ROOT}");
    let result = real(&push, &tree);
    assert_ne!(result.exit_code, Some(0));
    let output = String::from_utf8_lossy(&result.stderr).to_string();
    let why = no_verdict(&push, &result, &host());
    if output.contains(ABSENT_ROOT) {
        assert!(why.is_some_and(|why| why.contains("(pushd)")), "{output}");
    } else {
        assert_eq!(why, None, "{output}");
    }
}

//! The generic input mutation: what it names, what it never moves, and that
//! it restores.

use std::collections::BTreeMap;
use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::acceptance_policy_findings;

use super::*;
use crate::command::workflow_task_set::executability::tests::{CRASHING, FIXED};
use crate::command::workflow_task_set::republish::test_fixture::criterion;

fn contract_of(commands: &[(&str, &str)]) -> AcceptanceContract {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set(
        &commands
            .iter()
            .map(|(id, command)| (*id, *command, true))
            .collect::<Vec<_>>(),
    );
    set.contract()
}

fn inputs_of(command: &str) -> Vec<String> {
    named_inputs(&criterion("AC-M-001", command, true))
}

#[test]
fn a_check_names_its_relative_data_and_never_an_absolute_or_parent_path() {
    let names = inputs_of(
        "grep -q ready data/state.txt && cat ./notes.md /etc/passwd ../outside/x && python3 -c \"open('present')\" && test -d target/debug && ls .git/HEAD",
    );
    for expected in ["data/state.txt", "notes.md", "present"] {
        assert!(
            names.contains(&expected.to_string()),
            "{expected}: {names:?}"
        );
    }
    for refused in [
        "/etc/passwd",
        "../outside/x",
        "etc/passwd",
        "target/debug",
        ".git/HEAD",
    ] {
        assert!(
            !names.contains(&refused.to_string()),
            "{refused}: {names:?}"
        );
    }
}

/// Fix 1: what makes the check run is never moved: the command word, an
/// interpreter's script (and the directory holding it), a `cd` or `-C`
/// target, and a build manifest.
#[test]
fn a_check_never_names_its_own_script_program_cd_target_or_manifest() {
    let names = inputs_of(
        "cd sub && sh scripts/check.sh data/in.txt && ./bin/tool data/b.txt && make -C build && cargo test --manifest-path crate/Cargo.toml && grep -q x Cargo.toml && python3 - <<'PY'\nopen('data/c.txt')\nPY\n",
    );
    for never in [
        "sub",
        "scripts/check.sh",
        "scripts",
        "bin/tool",
        "bin",
        "build",
        "crate/Cargo.toml",
        "Cargo.toml",
        "sh",
        "make",
    ] {
        assert!(!names.contains(&never.to_string()), "{never}: {names:?}");
    }
    for data in ["data/in.txt", "data/b.txt", "data/c.txt"] {
        assert!(names.contains(&data.to_string()), "{data}: {names:?}");
    }
    let inline = inputs_of("python3 -c 'print(open(\"data/d.txt\").read())'");
    assert!(inline.contains(&"data/d.txt".to_string()), "{inline:?}");
}

#[test]
fn a_mutated_check_resolves_like_its_original_under_the_host_policy() {
    let contract = contract_of(&[
        ("AC-M-001", "grep -q ready data/state.txt"),
        ("AC-M-002", CRASHING),
        ("AC-M-003", FIXED),
        (
            "AC-M-004",
            "trap 'rm -rf \"$d\"' EXIT; d=$(mktemp -d) || exit 1; test -f present",
        ),
    ]);
    assert!(acceptance_policy_findings(&contract).is_empty());
    let inputs: BTreeMap<String, Vec<String>> = (contract.acceptance.iter())
        .map(|entry| (entry.id.clone(), named_inputs(entry)))
        .filter(|(_, names)| !names.is_empty())
        .collect();
    let mutated = mutated(
        &contract,
        &inputs,
        &[Path::new("/live/repo")],
        &Markers::new(),
    );
    assert_eq!(acceptance_policy_findings(&mutated), Vec::new());
}

/// Runs `script` with `sh` in `dir`: its result as the probe sees one.
pub(crate) fn sh(dir: &Path, script: &str) -> CheckResult {
    use std::io::Write;
    let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
        .arg("-s")
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    CheckResult {
        classification: None,
        acceptance_id: "AC".into(),
        exit_code: out.status.code(),
        quota_walk_count: 0,
        stdout: out.stdout,
        stderr: out.stderr,
        operational_error: None,
    }
}

fn names_of(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

#[test]
fn the_mutation_moves_named_inputs_aside_and_restores_them_whatever_the_check_traps() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/state.txt"), "ready").unwrap();
    let original =
        "trap 'rm -rf \"$d\"' EXIT; d=$(mktemp -d) || exit 1; grep -q ready data/state.txt";
    assert_eq!(sh(dir.path(), original).exit_code, Some(0));
    let markers = Markers::new();
    let names = names_of(&["data/state.txt", "absent"]);
    let ran = sh(dir.path(), &wrap(original, &names, &[], &markers));
    assert_ne!(ran.exit_code, Some(0), "fails with its input moved aside");
    assert_eq!(markers.moved(&ran), names_of(&["data/state.txt"]));
    assert!(
        markers.restored_all(&ran),
        "{}",
        String::from_utf8_lossy(&ran.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/state.txt")).unwrap(),
        "ready"
    );
    // A check that recreates the input it reads is undone too.
    let recreates = "printf 'other' > data/state.txt; exit 0";
    sh(dir.path(), &wrap(recreates, &names, &[], &Markers::new()));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/state.txt")).unwrap(),
        "ready"
    );
}

/// Fix 3: a nested input and its parent come back in reverse order, so
/// nothing is left behind.
#[test]
fn nested_inputs_are_restored_in_reverse_order() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("d")).unwrap();
    std::fs::write(dir.path().join("d/f"), "x").unwrap();
    let markers = Markers::new();
    let ran = sh(
        dir.path(),
        &wrap("exit 3", &names_of(&["d/f", "d"]), &[], &markers),
    );
    assert_eq!(ran.exit_code, Some(3));
    assert!(markers.restored_all(&ran));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("d/f")).unwrap(),
        "x"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("d")).unwrap().count(),
        1,
        "no leftover"
    );
}

/// Fix 3: a killed run restores nothing and reports no restore: unproven.
/// The next run sweeps what it left before moving anything.
#[test]
fn a_killed_mutation_is_unproven_and_its_leftovers_are_swept() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input"), "x").unwrap();
    let markers = Markers::new();
    let killed = sh(
        dir.path(),
        &wrap("kill -9 $$", &names_of(&["input"]), &[], &markers),
    );
    assert!(
        !markers.restored_all(&killed),
        "a killed run proves nothing"
    );
    assert!(
        !dir.path().join("input").exists(),
        "its input was left moved"
    );
    let next = Markers::new();
    let ran = sh(
        dir.path(),
        &wrap("test -f input", &names_of(&["other"]), &[], &next),
    );
    assert_eq!(ran.exit_code, Some(0), "the leftover was swept back first");
    assert!(next.restored_all(&ran));
    assert!(dir.path().join("input").exists());
}

/// Fix 4: an input reached through a symlink is never moved: it could be a
/// live file.
#[cfg(unix)]
#[test]
fn an_input_behind_a_symlink_is_never_moved() {
    let live = tempfile::tempdir().unwrap();
    std::fs::write(live.path().join("state.txt"), "ready").unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(live.path(), dir.path().join("ext")).unwrap();
    std::fs::write(dir.path().join("link-file"), "x").unwrap();
    std::os::unix::fs::symlink(dir.path().join("link-file"), dir.path().join("alias")).unwrap();
    let markers = Markers::new();
    let ran = sh(
        dir.path(),
        &wrap(
            "exit 1",
            &names_of(&["ext/state.txt", "alias"]),
            &[],
            &markers,
        ),
    );
    assert!(markers.moved(&ran).is_empty(), "{:?}", markers.moved(&ran));
    assert!(live.path().join("state.txt").exists());
}

/// Minor: markers carry a per-run nonce, so a check cannot forge them.
#[test]
fn a_forged_marker_without_the_nonce_counts_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let markers = Markers::new();
    let forged = "printf 'archon-mutation-moved-0: data\\n' >&2; printf 'archon-mutation-restored-0\\n'; exit 1";
    let ran = sh(dir.path(), forged);
    assert!(markers.moved(&ran).is_empty());
    assert!(!markers.restored_all(&ran));
}

#[test]
fn the_mutation_refuses_to_move_anything_inside_a_live_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input"), "x").unwrap();
    let live = dir
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let markers = Markers::new();
    let ran = sh(
        dir.path(),
        &wrap("test -f input", &names_of(&["input"]), &[&live], &markers),
    );
    assert_eq!(ran.exit_code, Some(GUARD_STATUS));
    assert!(markers.guarded(&ran), "a guarded run is never a verdict");
    assert!(dir.path().join("input").exists(), "nothing was moved");
}

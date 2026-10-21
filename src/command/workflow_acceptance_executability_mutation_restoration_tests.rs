//! Filesystem and policy regressions for the acceptance input mutation.

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
    let project = tempfile::tempdir().unwrap();
    for path in [
        "data/state.txt",
        "data/in.txt",
        "data/b.txt",
        "data/c.txt",
        "data/d.txt",
        "notes.md",
        "present",
    ] {
        let path = project.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "fixture").unwrap();
    }
    named_inputs(
        &criterion("AC-M-001", command, true),
        project.path(),
        project.path(),
    )
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

/// The command mechanics and inline interpreter data must never be moved.
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
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("data")).unwrap();
    std::fs::write(project.path().join("data/state.txt"), "ready").unwrap();
    std::fs::write(project.path().join("present"), "present").unwrap();
    let inputs: BTreeMap<String, Vec<String>> = contract
        .acceptance
        .iter()
        .map(|entry| {
            (
                entry.id.clone(),
                named_inputs(entry, project.path(), project.path()),
            )
        })
        .filter(|(_, names)| !names.is_empty())
        .collect();
    let wrapped = mutated(
        &contract,
        &inputs,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[Path::new("/live/repo")],
        &Markers::new(),
    );
    assert_eq!(acceptance_policy_findings(&wrapped), Vec::new());
}

/// Runs `script` with `sh` in `dir`: its result as the probe sees one.
fn sh(dir: &Path, script: &str) -> CheckResult {
    let out = super::super::test_helpers::run_with_host_mutation(dir, script);
    let result = CheckResult {
        classification: None,
        acceptance_id: "AC".into(),
        exit_code: out.status.code(),
        quota_walk_count: 0,
        stdout: out.stdout,
        stderr: out.stderr,
        operational_error: None,
        environment_note: None,
    };
    result
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
    assert_eq!(markers.moved(&ran, &names), names_of(&["data/state.txt"]));
    assert!(
        markers.restored_all(&ran),
        "{}",
        String::from_utf8_lossy(&ran.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/state.txt")).unwrap(),
        "ready"
    );
    // A recreated input still present when restoration runs is undone too.
    let recreates = "printf 'other' > data/state.txt; exit 0";
    let recreate_markers = Markers::new();
    let recreated = sh(dir.path(), &wrap(recreates, &names, &[], &recreate_markers));
    assert_eq!(
        recreate_markers.recreated(&recreated, &names),
        BTreeSet::from(["data/state.txt".to_string()]),
        "{}",
        String::from_utf8_lossy(&recreated.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/state.txt")).unwrap(),
        "other"
    );
    assert_eq!(
        std::fs::read_to_string(
            dir.path()
                .join(format!("data/state.txt.{}", recreate_markers.tag()))
        )
        .unwrap(),
        "ready"
    );
    assert_eq!(
        recreate_markers
            .states(&recreated, &names)
            .get("data/state.txt")
            .map(String::as_str),
        Some("original-recreated")
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_parent_makes_the_wrapper_report_unsafe_to_restore() {
    let outside = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/state.txt"), "inside").unwrap();
    let markers = Markers::new();
    let outside_aside = outside.path().join(format!("state.txt.{}", markers.tag()));
    std::fs::write(&outside_aside, "outside aside").unwrap();
    let command = format!("rm -rf data; ln -s '{}' data", outside.path().display());

    let result = sh(
        dir.path(),
        &wrap(&command, &["data/state.txt".into()], &[], &markers),
    );

    let names = names_of(&["data/state.txt"]);
    assert!(
        markers
            .states(&result, &names)
            .get("data/state.txt")
            .is_some_and(|state| state.starts_with("unsafe-to-restore"))
    );
    assert!(outside_aside.exists());
    assert!(!outside.path().join("state.txt").exists());
    assert!(!markers.restored_all(&result));
}

#[cfg(unix)]
#[test]
fn a_mutation_wrapper_keeps_sibling_inputs_visible_and_readable() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(project.join("data")).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(project.join("data/input.txt"), "input").unwrap();
    std::fs::write(sibling.join("fixture.txt"), "ready\n").unwrap();
    let original = "grep -q ready ../sibling/fixture.txt";
    assert_eq!(sh(&project, original).exit_code, Some(0));

    let markers = Markers::new();
    let wrapped = wrap(original, &["data/input.txt".into()], &[], &markers);
    let found = archon_workflow::acceptance_sibling_paths::discover(&project, &[wrapped.clone()]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].source, sibling.canonicalize().unwrap());
    let result = sh(&project, &wrapped);
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(markers.restored_all(&result));
}

#[cfg(unix)]
#[test]
fn sibling_only_input_is_never_moved_during_the_can_fail_run() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let fixture = sibling.join("fixture.txt");
    std::fs::write(&fixture, "ready\n").unwrap();
    std::fs::write(sibling.join("fixture.txt.archon-mutated-old"), "old\n").unwrap();
    let before = std::fs::read_dir(&sibling)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    let original = "grep -q ready ../sibling/fixture.txt";
    assert!(named_inputs(&criterion("AC-M-001", original, true), &project, &project).is_empty());

    let markers = Markers::new();
    let wrapped = wrap(original, &[], &[], &markers);
    let discovered =
        archon_workflow::acceptance_sibling_paths::discover(&project, &[wrapped.clone()]);
    assert_eq!(
        discovered.len(),
        1,
        "wrapper keeps sibling discovery visible"
    );
    let can_fail = sh(&project, &wrapped);
    assert_eq!(
        can_fail.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&can_fail.stderr)
    );
    assert!(markers.moved(&can_fail, &[]).is_empty());
    let after = std::fs::read_dir(&sibling)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(after, before, "the sibling copy stays byte-identical");
    assert_eq!(std::fs::read_to_string(fixture).unwrap(), "ready\n");
}

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

/// An input reached through a symlink may be live, so never move it.
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
    let names = names_of(&["ext/state.txt", "alias"]);
    assert!(
        markers.moved(&ran, &names).is_empty(),
        "{:?}",
        markers.moved(&ran, &names)
    );
    assert!(live.path().join("state.txt").exists());
    assert!(dir.path().join("link-file").exists());
    let states = markers.move_states(&ran, &names);
    assert_eq!(
        states.get("ext/state.txt").map(String::as_str),
        Some("refused: symlink component")
    );
    assert_eq!(
        states.get("alias").map(String::as_str),
        Some("refused: symlink component")
    );
}

#[path = "workflow_acceptance_executability_mutation_alias_tests.rs"]
mod alias_tests;

#[test]
fn an_absent_candidate_has_an_explicit_move_state() {
    let dir = tempfile::tempdir().unwrap();
    let markers = Markers::new();
    let names = names_of(&["missing/input.txt"]);
    let ran = sh(dir.path(), &wrap("exit 1", &names, &[], &markers));
    assert_eq!(
        markers
            .move_states(&ran, &names)
            .get("missing/input.txt")
            .map(String::as_str),
        Some("absent at the selected root")
    );
}

#[test]
fn a_forged_marker_without_the_nonce_counts_for_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let markers = Markers::new();
    let forged = "printf 'archon-mutation-moved-0: 0\\n' >&2; printf 'archon-mutation-restored-0\\n'; exit 1";
    let ran = sh(dir.path(), forged);
    assert!(markers.moved(&ran, &["data".into()]).is_empty());
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

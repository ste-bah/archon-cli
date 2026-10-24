use super::*;
use std::path::Path;

fn names_of(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

fn sh(dir: &Path, script: &str) -> CheckResult {
    let out = super::test_helpers::run_with_host_mutation(dir, script);
    let result = CheckResult {
        fault: None,
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

#[test]
fn mutation_state_protocol_reports_directory_recreation_and_normal_restore() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data/tree")).unwrap();
    std::fs::write(dir.path().join("data/tree/old"), "old").unwrap();
    let names = names_of(&["data/tree"]);
    let markers = Markers::new();
    let normal = sh(
        dir.path(),
        &wrap("test -d data/tree", &names, &[], &markers),
    );
    assert_eq!(
        markers
            .states(&normal, &names)
            .get("data/tree")
            .map(String::as_str),
        Some("restored")
    );
    assert!(
        markers
            .status(&normal)
            .is_some_and(|status| status.ends_with(":1:none:none"))
    );

    let recreated_markers = Markers::new();
    let recreated = sh(
        dir.path(),
        &wrap(
            "mkdir -p data/tree; printf fresh > data/tree/new",
            &names,
            &[],
            &recreated_markers,
        ),
    );
    assert_eq!(
        recreated_markers
            .states(&recreated, &names)
            .get("data/tree")
            .map(String::as_str),
        Some("original-recreated")
    );
    assert!(
        dir.path()
            .join(format!("data/tree.{}", recreated_markers.tag()))
            .join("old")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn mutation_markers_map_indices_to_newline_and_quoted_input_names() {
    let dir = tempfile::tempdir().unwrap();
    let names = names_of(&["line\nbreak.txt", "space and 'quote'.txt"]);
    for name in &names {
        std::fs::write(dir.path().join(name), "input").unwrap();
    }
    let markers = Markers::new();
    let result = sh(dir.path(), &wrap("exit 1", &names, &[], &markers));

    assert_eq!(markers.moved(&result, &names), names);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!stderr.contains("line\nbreak.txt"));
    assert!(!stderr.contains("space and 'quote'.txt"));
    assert!(names.iter().all(|name| dir.path().join(name).is_file()));
}

#[cfg(unix)]
#[test]
fn a_recreated_symlink_is_original_recreated_without_touching_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/state.txt"), "input").unwrap();
    std::fs::write(outside.path().join("sentinel"), "outside").unwrap();
    let names = names_of(&["data/state.txt"]);
    let markers = Markers::new();
    let command = format!(
        "ln -s {} data/state.txt",
        quote(&outside.path().to_string_lossy())
    );
    let result = sh(dir.path(), &wrap(&command, &names, &[], &markers));

    let states = markers.states(&result, &names);
    assert_eq!(
        states.get("data/state.txt").map(String::as_str),
        Some("original-recreated")
    );
    assert!(
        markers
            .recreated(&result, &names)
            .contains("data/state.txt")
    );
    assert_eq!(
        std::fs::read_to_string(outside.path().join("sentinel")).unwrap(),
        "outside"
    );
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[test]
fn a_recreated_input_read_and_deleted_by_the_check_is_marked_recreated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/state.txt"), "ready").unwrap();
    let names = names_of(&["data/state.txt"]);
    let recreates = "d=data; f=state.txt; printf 'other' > \"$d/$f\"; cat \"$d/$f\" >/dev/null; rm \"$d/$f\"; exit 0";
    let markers = Markers::new();
    let recreated = sh(dir.path(), &wrap(recreates, &names, &[], &markers));
    assert_eq!(recreated.exit_code, Some(0));
    assert_eq!(
        markers
            .states(&recreated, &names)
            .get("data/state.txt")
            .map(String::as_str),
        Some("original-recreated")
    );
    assert!(
        !crate::command::workflow_task_set::executability::prove::all_moved_inputs_restored(
            &recreated, &markers, &names
        )
    );
    assert_eq!(
        markers
            .states(&recreated, &names)
            .get("data/state.txt")
            .map(String::as_str),
        Some("original-recreated"),
        "the restoration protocol records the recreated input"
    );
    assert!(
        !crate::command::workflow_task_set::executability::prove::all_moved_inputs_restored(
            &recreated, &markers, &names
        ),
        "recreated input must not be proven"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/state.txt")).unwrap(),
        "ready"
    );
}

#[test]
fn recreating_reading_and_deleting_an_input_cannot_hide_behind_restored_parent_mtime() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/state.txt"), "ready").unwrap();
    let touch_probe = sh(dir.path(), "touch -r data .parent-time-ref");
    if touch_probe.exit_code != Some(0) {
        let stderr = String::from_utf8_lossy(&touch_probe.stderr).to_ascii_lowercase();
        let unsupported = ["illegal option", "invalid option", "unknown option"]
            .iter()
            .any(|message| stderr.contains(message));
        assert!(
            unsupported,
            "touch -r probe failed for a reason other than an unsupported option (exit {:?}): {}",
            touch_probe.exit_code, stderr
        );
        eprintln!("skipping: touch -r is unsupported on this host");
        return;
    }
    std::fs::remove_file(dir.path().join(".parent-time-ref")).unwrap();

    let names = names_of(&["data/state.txt"]);
    let recreates = "set -e; touch -r data .parent-time-before; touch -r data .parent-time-ref; sleep 1; printf 'other' > data/state.txt; cat data/state.txt >/dev/null; rm data/state.txt; touch -m -r .parent-time-ref data; touch -r data .parent-time-after; rm .parent-time-ref; exit 0";
    let markers = Markers::new();
    let recreated = sh(dir.path(), &wrap(recreates, &names, &[], &markers));
    assert_eq!(recreated.exit_code, Some(0));
    let parent_mtime_before = std::fs::metadata(dir.path().join(".parent-time-before"))
        .unwrap()
        .modified()
        .unwrap();
    let parent_mtime_after = std::fs::metadata(dir.path().join(".parent-time-after"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(parent_mtime_before, parent_mtime_after);
    assert_eq!(
        markers
            .states(&recreated, &names)
            .get("data/state.txt")
            .map(String::as_str),
        Some("original-recreated"),
        "the parent metadata signal still catches the recreated input after its timestamp is restored"
    );
    assert!(
        !crate::command::workflow_task_set::executability::prove::all_moved_inputs_restored(
            &recreated, &markers, &names
        ),
        "parent metadata change must prevent proof even when its timestamp is restored"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("data/state.txt")).unwrap(),
        "ready"
    );
}

#[test]
fn mutation_exit_143_without_wrapper_signal_is_an_ordinary_check_exit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/state"), "old").unwrap();
    let markers = Markers::new();
    let result = sh(
        dir.path(),
        &wrap("exit 143", &["data/state".into()], &[], &markers),
    );
    assert!(
        markers
            .status(&result)
            .is_some_and(|status| status.ends_with(":143:15:none"))
    );
    assert_eq!(markers.observed_signal(&result), None);
    assert_eq!(
        markers
            .states(&result, &["data/state".into()])
            .get("data/state")
            .map(String::as_str),
        Some("restored")
    );
}

#[cfg(unix)]
#[test]
fn mutation_wrapper_records_a_signal_it_receives() {
    use std::io::Write;
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("input"), "old").unwrap();
    let markers = Markers::new();
    let mut child = Command::new(archon_shell::resolve_posix_shell())
        .arg("-s")
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(wrap("sleep 2", &["input".into()], &[], &markers).as_bytes())
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let signal = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(signal.success());
    let output = child.wait_with_output().unwrap();
    let result = CheckResult {
        fault: None,
        classification: None,
        acceptance_id: "AC".into(),
        exit_code: output.status.code(),
        quota_walk_count: 0,
        stdout: output.stdout,
        stderr: output.stderr,
        operational_error: None,
        environment_note: None,
    };
    assert_eq!(markers.observed_signal(&result).as_deref(), Some("TERM"));
    assert!(
        markers
            .status(&result)
            .is_some_and(|status| status.contains(":TERM"))
    );
}

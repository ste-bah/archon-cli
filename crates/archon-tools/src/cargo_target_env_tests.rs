//! Tests for the parent module, split out to keep `cargo_target_env.rs` under
//! the 500-line gate. `#[path]` keeps `super` meaning the parent module, so
//! every `use super::*` below resolves exactly as it did when inline.

use super::*;

#[test]
fn shell_word_detection_matches_cargo_not_substrings() {
    assert!(contains_shell_word("cargo test -p demo", "cargo"));
    assert!(contains_shell_word(
        "cd repo && cargo +nightly test",
        "cargo"
    ));
    assert!(!contains_shell_word("echo cargo-test", "cargo"));
    assert!(!contains_shell_word("echo xcargo", "cargo"));
}

#[test]
fn stable_hash_is_deterministic() {
    let path = Path::new("/Volumes/Externalwork/demo");
    assert_eq!(stable_path_hash(path), stable_path_hash(path));
    assert_ne!(
        stable_path_hash(path),
        stable_path_hash(Path::new("/Volumes/Externalwork/other"))
    );
}

#[test]
fn local_target_root_respects_configured_temp_root() {
    let temp = Path::new("/Volumes/Externalwork/archon-cli/tmp");
    assert_eq!(
        local_target_root_for_temp(temp),
        Path::new("/Volumes/Externalwork/archon-cli/tmp/archon-cargo-target")
    );
}

#[test]
fn linked_worktree_uses_primary_repository_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let primary = temp.path().join("primary");
    let worktree = temp.path().join("worktree");
    let worktree_git = primary.join(".git/worktrees/feature");
    std::fs::create_dir_all(&worktree_git).expect("worktree git dir");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git.display()),
    )
    .expect("git file");

    assert_eq!(
        repository_identity(&worktree),
        std::fs::canonicalize(primary).expect("canonical primary repository")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn external_volume_cargo_gets_local_target_dir() {
    let target = guarded_cargo_target_dir("cargo test", Path::new("/Volumes/Externalwork/demo"))
        .expect("external Cargo command should be guarded");
    assert!(target.starts_with(local_target_root()));
}

#[test]
fn agent_cargo_target_assignment_is_replaced_by_host_value() {
    let command =
        enforce_host_cargo_target_dir("CARGO_TARGET_DIR=target/task-demo cargo test -p demo", true);

    assert!(!command.contains("target/task-demo"));
    assert!(command.contains("CARGO_TARGET_DIR=\"${ARCHON_CARGO_TARGET_DIR}\""));
}

#[test]
fn unrelated_or_unguarded_command_is_unchanged() {
    let command = "printf 'CARGO_TARGET_DIR=example'";
    assert_eq!(enforce_host_cargo_target_dir(command, false), command);
    assert_eq!(
        enforce_host_cargo_target_dir("cargo test", true),
        "cargo test"
    );
}

#[test]
fn incomplete_tree_sitter_output_requests_scoped_cache_repair() {
    let temp = tempfile::tempdir().expect("tempdir");
    let build = temp.path().join("debug/build/tree-sitter-example/out");
    std::fs::create_dir_all(&build).expect("build output");
    std::fs::write(build.parent().unwrap().join("invoked.timestamp"), "").expect("stamp");
    std::fs::write(build.join("libtree-sitter.a"), "").expect("library");

    assert!(incomplete_tree_sitter_build_output(temp.path()));
    std::fs::write(build.join("stdlib-symbols.txt"), "symbols").expect("symbols");
    assert!(!incomplete_tree_sitter_build_output(temp.path()));
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn inherited_cargo_target_dir_is_overridden() {
    let mut env = vec![("CARGO_TARGET_DIR".to_string(), "/agent/target".to_string())];
    let _lock = apply_cargo_target_dir_guard(
        &mut env,
        "CARGO_TARGET_DIR=target/task-demo cargo test",
        Path::new("/Volumes/Externalwork/demo"),
        "override-test",
        None,
    )
    .await
    .expect("guard")
    .expect("external Cargo command should be guarded");
    let target_values: Vec<&str> = env
        .iter()
        .filter(|(key, _)| key == "CARGO_TARGET_DIR")
        .map(|(_, value)| value.as_str())
        .collect();

    assert_eq!(target_values.len(), 1);
    assert_ne!(target_values[0], "/agent/target");
    assert!(
        env.iter()
            .any(|(key, value)| { key == "ARCHON_CARGO_TARGET_DIR" && value == target_values[0] })
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn cargo_target_lock_serializes_same_external_repo() {
    let mut first_env = Vec::new();
    let first = apply_cargo_target_dir_guard(
        &mut first_env,
        "cargo test",
        Path::new("/Volumes/Externalwork/demo"),
        "session-1",
        None,
    )
    .await
    .unwrap()
    .expect("first cargo command should lock target");

    let mut second_env = Vec::new();
    let second = apply_cargo_target_dir_guard(
        &mut second_env,
        "cargo test",
        Path::new("/Volumes/Externalwork/demo"),
        "session-2",
        None,
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), second)
            .await
            .is_err(),
        "second cargo command should wait for the same target lock"
    );
    drop(first);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn cargo_lock_wait_is_recorded_outside_session_timeout() {
    let mut first_env = Vec::new();
    let first = apply_cargo_target_dir_guard(
        &mut first_env,
        "cargo test",
        Path::new("/Volumes/Externalwork/demo"),
        "holder",
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let waiting = tokio::spawn(async {
        let mut env = Vec::new();
        apply_cargo_target_dir_guard(
            &mut env,
            "cargo test",
            Path::new("/Volumes/Externalwork/demo"),
            "queued",
            None,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(current_timeout_exempt_cargo_wait("queued") >= Duration::from_millis(50));
    drop(first);
    waiting.await.unwrap().unwrap();

    assert!(take_timeout_exempt_cargo_wait("queued") >= Duration::from_millis(50));
}

/// The repair prelude must not put anything on the caller's output.
///
/// There was no test here at all, which is how the defect shipped: the prelude
/// was a string nothing asserted on. `cargo clean` writes
/// `Removed 15 files, 50.2KiB total` to *stderr*, and the old
/// `cargo clean -p tree-sitter >/dev/null` redirected only stdout. Archon
/// captures stderr into the tool result, so that line was prepended to whatever
/// the caller's command printed -- and only when the cache happened to be
/// dirty, so it looked like the command itself had gone wrong.
#[test]
fn cache_repair_prelude_silences_stderr_and_still_reports_failure() {
    let prelude = cargo_cache_repair_prelude_for(true);

    assert!(
        prelude.contains("2>&1 >/dev/null"),
        "stderr must be captured and stdout discarded, in that order; got: {prelude}"
    );
    assert!(
        !prelude.contains(">/dev/null ||"),
        "the old stdout-only redirect leaks cargo's `Removed N files` line into \
         the caller's output: {prelude}"
    );
    // A silent success is the point, but a silent *failure* is the bug this
    // would otherwise trade for: `exit $?` with stderr discarded aborts the
    // whole tool call with a status and no reason.
    assert!(
        prelude.contains(r#"printf '%s\n' "$__archon_clean" >&2"#),
        "a failed repair must say why: {prelude}"
    );
    assert!(
        prelude.contains(r#"exit "$__archon_rc""#),
        "a failed repair must propagate cargo's exit code: {prelude}"
    );
}

/// No repair wanted means no prelude at all, not an empty command.
#[test]
fn cache_repair_prelude_is_empty_when_the_cache_is_intact() {
    assert_eq!(cargo_cache_repair_prelude_for(false), "");
    assert_eq!(cargo_cache_repair_prelude(None), "");
}

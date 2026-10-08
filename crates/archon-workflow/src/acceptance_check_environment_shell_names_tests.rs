//! Shell-maintained names (`_`, `PWD`, `OLDPWD`, `SHLVL`) are never withheld
//! candidates on the production capture path (Issue 349). Each case runs in a
//! child of this test binary with those names exported, so `capture()` and
//! `capture_with_dispatch()` read them from the real OS environment.

use super::*;

const CASE: &str = "ISSUE_349_SHELL_NAME_CASE";
const WITHHELD: &str = "ARCHON_349_WITHHELD_FIXTURE";

#[cfg(unix)]
fn shell_name_case(case: &str, text: &str, dispatch: bool) {
    if std::env::var(CASE).as_deref() != Ok(case) {
        let home = tempfile::tempdir().unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([case, "--exact", "--nocapture"])
            .env_clear()
            .env(CASE, case)
            .env("HOME", home.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("_", "/usr/bin/env")
            .env("PWD", home.path())
            .env("OLDPWD", "/")
            .env("SHLVL", "2")
            .env(WITHHELD, "fixture-value")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let environment = if dispatch {
        CommandEnvironment::capture_with_dispatch(None, &[]).unwrap()
    } else {
        CommandEnvironment::capture(None).unwrap()
    };
    let output = environment
        .command("/bin/sh")
        .args(["-c", "printf '%s' \"$1\"; exit 3", "fixture", text])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(environment.note(&[&output.stdout, &output.stderr]), None);
    // The same capture still notes a real withheld name, so the empty note
    // above is the shell-name rule, not an empty withheld set.
    let note = environment
        .note(&[format!("{text} {WITHHELD}").as_bytes()])
        .expect("a real withheld name is noted");
    let named = format!("Note: output mentions withheld variable(s) {WITHHELD}. ");
    assert!(note.starts_with(&named), "{note}");
    assert!(!note.contains("fixture-value"), "{note}");
}

#[cfg(unix)]
#[test]
fn r6_capture_rust_underscore_pattern_gives_no_note() {
    shell_name_case(
        "acceptance_check_environment::tests::shell_names::r6_capture_rust_underscore_pattern_gives_no_note",
        "error: Err(_) => let _ = |_| failed",
        false,
    );
}

#[cfg(unix)]
#[test]
fn r6_capture_working_directory_names_give_no_note() {
    shell_name_case(
        "acceptance_check_environment::tests::shell_names::r6_capture_working_directory_names_give_no_note",
        "cd $PWD failed; previous $OLDPWD",
        false,
    );
}

#[cfg(unix)]
#[test]
fn r6_dispatch_capture_shell_level_and_python_loop_give_no_note() {
    shell_name_case(
        "acceptance_check_environment::tests::shell_names::r6_dispatch_capture_shell_level_and_python_loop_give_no_note",
        "SHLVL=2\n  for _ in range(3):\nAssertionError",
        true,
    );
}

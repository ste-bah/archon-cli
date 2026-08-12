//! Issue #174 — the keyboard-enhancement flags must be popped on *every* way
//! out of the TUI.
//!
//! A terminal left in modified-keys mode outlives the archon process: the
//! user's shell keeps receiving disambiguated key encodings and there is no
//! obvious way to undo it. So this is a spawn-and-kill harness rather than an
//! in-process assertion — it re-executes this test binary as a child with
//! stdout on a pipe, walks the child down one teardown path, and asserts the
//! pop sequence actually reached the child's stdout.
//!
//! The child runs the production `TerminalGuard` (via
//! `enter_without_raw_mode`, which differs only in not calling
//! `enable_raw_mode` — there is no terminal on a pipe to put into raw mode).
//! `ARCHON_TUI_KEYBOARD_ENHANCEMENT=1` forces the push so the harness does
//! not depend on what the CI runner's terminal happens to support.

use std::process::{Command, Stdio};

use archon_tui::terminal::{TerminalGuard, keyboard};

/// Set on the child to select a teardown path. Absent in the parent.
const MODE_ENV: &str = "ARCHON_TUI_TEARDOWN_MODE";

const CHILD_TEST: &str = "teardown_child_harness";

/// Written by the child once the guard is up, so a missing pop can be told
/// apart from a child that never started.
const READY: &str = "<<ready>>";
/// Written by the child while it is suspended — i.e. after the pop that
/// `suspend` owes and before the push that `resume` owes.
const SUSPENDED: &str = "<<suspended>>";

fn emit(marker: &str) {
    use std::io::Write;
    // Raw stdout, not `print!`: libtest's capture only intercepts the `print!`
    // machinery, and the escape sequences under test go to raw stdout too, so
    // markers and sequences must share the same stream to stay ordered.
    let mut out = std::io::stdout();
    let _ = out.write_all(marker.as_bytes());
    let _ = out.flush();
}

/// The child body. In the parent (no `MODE_ENV`) this is a no-op that keeps
/// libtest happy about the test name existing.
#[test]
fn teardown_child_harness() {
    let Ok(mode) = std::env::var(MODE_ENV) else {
        return;
    };

    let guard = TerminalGuard::enter_without_raw_mode();
    assert!(
        keyboard::is_active(),
        "harness precondition: the forced push must have happened"
    );
    emit(READY);

    match mode.as_str() {
        // Clean exit: `Drop` is the only thing that pops.
        "clean" => drop(guard),
        // Panic: `Drop` runs during unwind, but libtest catches the unwind, so
        // the *hook* is what must pop in a real single-threaded binary. Both
        // are exercised here and the pop must still appear exactly once.
        "panic" => panic!("teardown harness panic"),
        "suspend" => {
            guard.suspend();
            emit(SUSPENDED);
            guard.resume().expect("resume must re-take the terminal");
            drop(guard);
        }
        other => panic!("unknown teardown mode {other:?}"),
    }
}

/// Run the child down `mode` and return its stdout.
fn run_child(mode: &str) -> String {
    let exe = std::env::current_exe().expect("current test binary");
    let output = Command::new(exe)
        .arg("--exact")
        .arg(CHILD_TEST)
        .arg("--nocapture")
        .env(MODE_ENV, mode)
        .env(keyboard::ENHANCEMENT_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn teardown child");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_balanced(mode: &str, stdout: &str) {
    let push = keyboard::push_sequence();
    let pop = keyboard::pop_sequence();
    assert!(
        stdout.contains(READY),
        "[{mode}] child never reached the guard; stdout: {stdout:?}"
    );
    assert!(
        stdout.contains(&push),
        "[{mode}] child never pushed the enhancement flags; stdout: {stdout:?}"
    );
    assert_eq!(
        stdout.matches(&pop).count(),
        stdout.matches(&push).count(),
        "[{mode}] every push must be matched by exactly one pop; stdout: {stdout:?}"
    );
    let last_push = stdout.rfind(&push).expect("push present");
    let last_pop = stdout.rfind(&pop).expect("pop present");
    assert!(
        last_pop > last_push,
        "[{mode}] the child exited with a push outstanding; stdout: {stdout:?}"
    );
}

#[test]
fn clean_exit_pops_the_enhancement_flags() {
    assert_balanced("clean", &run_child("clean"));
}

#[test]
fn panic_pops_the_enhancement_flags() {
    let stdout = run_child("panic");
    assert_balanced("panic", &stdout);
}

/// Suspend must pop *before* handing the terminal over, and resume must push
/// again — otherwise the program archon suspends for (or the shell, if archon
/// stays stopped) inherits modified-keys mode.
#[test]
fn suspend_pops_and_resume_repushes() {
    let stdout = run_child("suspend");
    assert_balanced("suspend", &stdout);

    let pop = keyboard::pop_sequence();
    let push = keyboard::push_sequence();
    let suspended_at = stdout
        .find(SUSPENDED)
        .unwrap_or_else(|| panic!("child never reported being suspended; stdout: {stdout:?}"));
    let before_suspend = &stdout[..suspended_at];
    let after_suspend = &stdout[suspended_at..];

    assert!(
        before_suspend.contains(&pop),
        "suspend must pop before the terminal is handed over; stdout: {stdout:?}"
    );
    assert!(
        after_suspend.contains(&push),
        "resume must push the flags back; stdout: {stdout:?}"
    );
    assert!(
        after_suspend.contains(&pop),
        "the final drop must pop again; stdout: {stdout:?}"
    );
}

/// The sequences the assertions above are written against. If crossterm ever
/// changes them this fails first and names the reason.
#[test]
fn harness_asserts_on_the_kitty_wire_sequences() {
    assert_eq!(keyboard::push_sequence(), "\u{1b}[>1u");
    assert_eq!(keyboard::pop_sequence(), "\u{1b}[<1u");
}

//! Guard for #246: in a debug build, the deep workflow paths must fit in a
//! fixed thread stack well under the 2 MiB default.
//!
//! The deep frames are on tokio threads (a blocking thread drives the script
//! or the decomposed lifecycle there), not on the test thread, so the budget
//! is set with `RUST_MIN_STACK` in a child process (this test binary, run
//! again with a filter). That sizes every thread the child starts, so the
//! result does not depend on the parent's `RUST_MIN_STACK` (CI sets 32 MiB).
//! A stack overflow aborts the process, so only a child can report it as a
//! failure.

/// Measured on macOS arm64 (debug): each fixed decomposition test needs
/// about 500 KiB; before #246 the cancelled launch needed about 2.4 MiB and
/// overflowed the 2 MiB default.
const STACK_BUDGET: usize = 1024 * 1024;

/// The tests that drive a call through the script host's dispatch chain.
const DEEP_TESTS: [&str; 2] = [
    "cancelled_resumable_run_retains_task_root_ownership",
    "fixed_resume_appends_marker_and_canonical_resumed_event_before_provider",
];

/// The legacy decomposed lifecycle (round-4 review minor 1), measured on
/// macOS arm64 (debug) with `RUST_MIN_STACK`: before the fix the e2e test
/// below overflowed at 1.5 MiB and passed at 1.75 MiB of the 2 MiB default;
/// with the large children of its call chain built on the heap it overflows
/// at 960 KiB and passes at 1 MiB. This budget keeps 0.75 MiB of the default
/// free.
const LIFECYCLE_STACK_BUDGET: usize = 1280 * 1024;

/// The full libtest path (no crate name). A rename makes the child run no
/// test, which fails the `1 passed` check below: never a silent pass.
const LIFECYCLE_TEST: &str = "command::workflow_live::workflow_live_v2::workflow_live_v2_script::\
     workflow_live_v2_lifecycle_e2e_tests::workflow_live_v2_lifecycle_e2e_tests_b::\
     real_decomposed_lifecycle_normalizes_reclassified_ids_and_reaches_terminal";

/// Runs `names` in a child of this test binary with every thread stack set
/// to `budget` bytes, and fails unless all of them pass.
fn assert_fits(names: &[String], budget: usize, what: &str) {
    let output = archon_shell::spawn::command(std::env::current_exe().expect("test binary path"))
        .args(names)
        .args(["--exact", "--test-threads=1"])
        .env("RUST_MIN_STACK", budget.to_string())
        .output()
        .expect("run child test process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let passed = format!("{} passed", names.len());
    assert!(
        output.status.success() && stdout.contains(&passed),
        "the {what} tests did not pass with a {budget}-byte thread stack (#246). If stderr \
         shows a stack overflow, a future on that path got larger; `Box::pin` it where it is \
         passed to a wrapper or to `block_on`. \
         Child status: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
}

#[test]
fn fixed_decomposition_dispatch_fits_debug_stack_budget() {
    // `command::workflow_decompose_tests`, without the crate name and this
    // module's own name.
    let path = module_path!();
    let parent = path
        .split_once("::")
        .and_then(|(_, rest)| rest.rsplit_once("::"))
        .map_or("", |(parent, _)| parent);
    let names =
        DEEP_TESTS.map(|test| format!("{parent}::workflow_decomposition_resume_tests::{test}"));
    assert_fits(&names, STACK_BUDGET, "fixed decomposition dispatch");
}

#[test]
fn decomposed_lifecycle_fits_debug_stack_budget() {
    assert_fits(
        &[LIFECYCLE_TEST.to_string()],
        LIFECYCLE_STACK_BUDGET,
        "decomposed lifecycle",
    );
}

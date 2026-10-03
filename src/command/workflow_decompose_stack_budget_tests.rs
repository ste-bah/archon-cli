//! Guard for #246: in a debug build, a fixed decomposition launch that is
//! cancelled during work, and a resume that reaches its first agent call, must
//! fit in a fixed thread stack.
//!
//! The deep frames are on a tokio blocking thread (`WorkflowV2ScriptRunner::run`
//! drives the script there), not on the test thread, so the budget is set with
//! `RUST_MIN_STACK` in a child process (this test binary, run again with a
//! filter). That sizes every thread the child starts, so the result does not
//! depend on the parent's `RUST_MIN_STACK` (CI sets 32 MiB). A stack overflow
//! aborts the process, so only a child can report it as a failure.

/// Measured on macOS arm64 (debug): each test needs about 500 KiB; before
/// #246 the cancelled launch needed about 2.4 MiB and overflowed the 2 MiB
/// default.
const STACK_BUDGET: usize = 1024 * 1024;

/// The tests that drive a call through the script host's dispatch chain.
const DEEP_TESTS: [&str; 2] = [
    "cancelled_resumable_run_retains_task_root_ownership",
    "fixed_resume_appends_marker_and_canonical_resumed_event_before_provider",
];

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
    let output = std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .args(&names)
        .args(["--exact", "--test-threads=1"])
        .env("RUST_MIN_STACK", STACK_BUDGET.to_string())
        .output()
        .expect("run child test process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let passed = format!("{} passed", DEEP_TESTS.len());
    assert!(
        output.status.success() && stdout.contains(&passed),
        "the fixed decomposition dispatch tests did not pass with a {STACK_BUDGET}-byte thread \
         stack (#246). If stderr shows a stack overflow, a future in the script host's dispatch \
         chain got larger; `Box::pin` it where it is passed to a wrapper. \
         Child status: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
}

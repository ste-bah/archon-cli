// The native acceptance observer runs authorized commands under a POSIX shell
// in a SIGKILL-reaped process group (`acceptance_scratch_process.rs`), and
// these tests drive it with `test -f`, `printf`, `/usr/bin:/bin` toolchain
// paths and Unix file modes. None of that exists on Windows, where the lease is
// refused before a command runs, so the whole target is Unix-only.
#![cfg(unix)]

#[path = "../tests/support/native_fixture.rs"]
mod support;

/// The owned worktree removal makes no progress for 6 s of the progress
/// clock: past the old five-second bound, inside the 20 s profile window.
/// The clock is virtual and thread-local, so the test runs in-process with
/// the real git on `PATH` and has no wall-clock deadline of its own.
#[test]
fn cleanup_uses_profile_budget_for_slow_owned_tree_removal() {
    let (_temp, mut policy, commit, _, _) = support::fixture("test -f input");
    policy.timeout_secs = 20;
    let mut roots =
        archon_workflow::acceptance_scratch::ScratchRoots::prepare(&policy, &commit).unwrap();
    let owned = roots.root().to_path_buf();
    let _clock = super::test_clock::delay_next_removal(std::time::Duration::from_secs(6));
    roots
        .cleanup()
        .expect("cleanup must use the host profile budget, not five seconds");
    assert!(
        super::test_clock::removal_charged(),
        "the worktree removal ran on this thread and was charged 6 s"
    );
    assert!(!owned.exists());
}

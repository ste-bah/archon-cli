//! Waiting primitives for the compilation-gate lifecycle tests.
//!
//! Every wait here ends on an event. None of them carries a deadline, because a
//! deadline around process creation measures the machine: `.config/nextest.toml`
//! records 30 spawn samples on a loaded 32-core Windows box at min 893ms, mean
//! 2017ms, max 4960ms, for work that then took about 300ms. A precondition
//! bounded by a number in that range is a coin toss with the load as the coin.
//!
//! # Why the timeout tests pause the clock
//!
//! Those tests are about what the gate does when a limit expires against a child
//! in a particular state, and the state has to be established first. Under a real
//! clock the two are in a race the machine decides: a hundred-millisecond limit
//! against a debug-profile spawn costing seconds under load is a test of the box,
//! not of the gate. Under `#[tokio::test(start_paused = true)]` the limit is
//! virtual, so [`hold_until_started`] and [`hold_until_exited`] can establish the
//! state with the deadline held off, and [`fire_deadline`] then brings it about
//! deliberately.
//!
//! Holding the clock still: tokio auto-advances a paused clock when the runtime
//! has nothing left to poll. `yield_now` keeps deferred work permanently
//! outstanding, so the runtime is never in that state. Measured on Windows - 543ms
//! of real yielding moved virtual time by 0ns, with a 100ms timer still unfired.
//!
//! Firing by hand rather than letting auto-advance do it: auto-advance does not
//! survive contact with this gate. Tokio backs child stdio with blocking reads on
//! Windows, and one in flight stops the runtime ever being judged idle - measured
//! here as `timeout(30s, stdout.read_to_end())` never returning, where the same
//! timeout around `child.wait()` alone fired at once. The gate always reads both
//! streams while it waits, so on Windows the deadline would never arrive at all.
//! Advancing explicitly also makes the moment identical on every platform instead
//! of depending on a scheduler heuristic.
//!
//! The `std::thread::sleep` in each loop is real-time pacing, not a budget: it
//! cannot move a frozen clock, and without it these would spin a core flat out on
//! a box that is already busy - the very load the tests have to survive.

use super::*;

/// Hold the paused clock still until `marker` appears, and return the process id
/// the fixture wrote into it.
pub(super) async fn hold_until_started(marker: &Path) -> u32 {
    loop {
        if let Some(pid) = read_pid(marker) {
            return pid;
        }
        tokio::task::yield_now().await;
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Hold the paused clock still until process `pid` has exited.
///
/// "Exited" here means what `try_wait` means, which is the question the gate
/// actually asks: gone from the process table, or present as a zombie, which on
/// unix is an exited child that its parent has not reaped yet - and reaping it is
/// the gate's job, not ours.
///
/// The first attempt at this waited instead for the descendant to see EOF on a
/// pipe the direct child held, on the reasoning that a process closes its handles
/// as it tears down. It does - but handle closure precedes the process becoming
/// reapable, and that gap is real: the evidence assertion failed 4 runs in 10 on
/// an *idle* box, and passed under load, because load widened the gap in the
/// test's favour. A signal that works only when the machine is busy is the same
/// defect wearing the opposite sign. This asks the operating system the question
/// directly instead.
pub(super) async fn hold_until_exited(pid: u32) {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    while process_is_running(&mut system, pid) {
        tokio::task::yield_now().await;
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Bring on the gate's deadline, now that the state it should act against holds.
///
/// The margin added to `limit` only removes any doubt about `>=` at the deadline
/// itself; virtual time has not moved since the gate registered its timer.
pub(super) async fn fire_deadline(limit: Duration) {
    tokio::time::advance(limit + POLL_INTERVAL).await;
}

/// Read the process id a fixture wrote into its startup marker.
///
/// Returns `None` until the file holds a complete number: a reader can open the
/// marker between its creation and its write, and treating that as "not ready"
/// keeps the caller polling rather than panicking on a torn read.
pub(super) fn read_pid(marker: &Path) -> Option<u32> {
    std::fs::read_to_string(marker).ok()?.trim().parse().ok()
}

/// Block until process `pid` has left the process table.
///
/// This is the claim [`compilation_timeout_terminates_descendant_processes`]
/// makes, stated directly: a descendant that did not survive cleanup is a
/// descendant that is gone. It replaces a fixed observation window, which could
/// only ever be too short - and a window that is too short does not fail, it
/// passes for the wrong reason, so load quietly hollowed the test out instead of
/// reddening it.
///
/// `survivor` is checked on every turn so that a descendant which *did* outlive
/// cleanup reports itself immediately, rather than this loop waiting out a
/// process that is never going to exit.
///
/// A zombie counts as gone: on unix an orphan is reparented to init and may sit
/// unreaped for a moment, and it is termination, not reaping, that this asserts.
/// The residual risk is process-id reuse in the window between the descendant's
/// death and the first refresh below; reuse would make this wait longer, never
/// shorter, so it cannot turn a real failure into a pass.
pub(super) fn await_process_exit(pid: u32, survivor: &Path) {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    loop {
        assert!(
            !survivor.exists(),
            "timed-out compilation descendant survived process-tree cleanup"
        );
        if !process_is_running(&mut system, pid) {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn process_is_running(system: &mut sysinfo::System, pid: sysinfo::Pid) -> bool {
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system
        .process(pid)
        .is_some_and(|process| process.status() != sysinfo::ProcessStatus::Zombie)
}

/// Re-invoke this test binary as the [`controlled_child`] fixture.
pub(super) fn controlled_child_spec(
    project_root: &Path,
    marker: &Path,
    release: Option<&Path>,
) -> CommandSpec {
    let mut spec = CommandSpec::new(
        std::env::current_exe().unwrap(),
        [
            "--exact".to_owned(),
            "coding::compilation_gate::compilation_gate_tests::controlled_child".to_owned(),
            "--nocapture".to_owned(),
        ],
        project_root,
    )
    .with_env(CHILD_MARKER_ENV, marker.display().to_string());

    if let Some(release) = release {
        spec = spec
            .with_env(CHILD_MODE_ENV, "await-release")
            .with_env(CHILD_RELEASE_ENV, release.display().to_string());
    }

    spec
}

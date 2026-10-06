//! Issue 339: the liveness probe answers for real on every platform.

use super::process_alive;
use archon_test_support::live_process::LiveChild;

#[test]
fn a_running_child_is_alive_and_an_ended_one_is_not() {
    let mut child = LiveChild::spawn();
    let pid = child.pid();
    assert!(process_alive(pid), "pid {pid} is running");
    child.end();
    assert!(!process_alive(pid), "pid {pid} was killed and waited for");
}

#[test]
fn the_calling_process_is_alive() {
    assert!(process_alive(std::process::id()));
}

#[test]
fn pid_zero_is_never_a_running_process() {
    assert!(!process_alive(0));
}

#[test]
fn a_pid_outside_every_platform_range_is_not_running() {
    // On Unix it would turn negative and name a process group. Windows ignores
    // the low two bits of a pid, so this probes 0xFFFF_FFFC, far above any pid
    // Windows hands out.
    assert!(!process_alive(u32::MAX));
}

//! The unix process-group check behind the timeout evidence (issue #240).

use std::os::unix::process::{CommandExt, ExitStatusExt};

use super::super::{TREE_EXIT_BOUND, TreeTermination, await_group_exit, confirm_tree_terminated};
use super::*;

/// 0 would signal the gate's own group and 1 is init; an id that does not fit
/// a `pid_t`, or no id at all, names nothing. None of them may be signalled.
#[tokio::test]
async fn tree_check_refuses_ids_that_cannot_be_a_child_group() {
    for group in [None, Some(0), Some(1), Some(u32::MAX)] {
        assert_eq!(
            confirm_tree_terminated(group).await,
            TreeTermination::CheckFailed,
            "group id {group:?}"
        );
    }
}

/// The check kills a live group and returns only once the group is gone.
///
/// The member is this process's own child, so it is reaped on a thread: on
/// Linux an unreaped zombie still counts as a group member, which is exactly
/// the state the bound exists for, not the one this test is about.
#[test]
fn tree_check_kills_a_live_group_and_confirms_it_empty() {
    let mut member = Command::new("sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let pgid = libc::pid_t::try_from(member.id()).unwrap();
    // If the check fails to kill the group, end it here rather than leaving
    // the test (or a later `join`) behind a thirty-second sleep.
    let _guard = GroupKillGuard(pgid);
    let reaper = std::thread::spawn(move || member.wait());

    assert_eq!(
        await_group_exit(pgid, TREE_EXIT_BOUND),
        TreeTermination::Confirmed
    );
    let status = reaper.join().unwrap().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
}

/// A group whose only member is an unreaped zombie is not reported empty
/// until the zombie is reaped - on macOS that is the EPERM path, on Linux the
/// zombie still answers `killpg` - and is then confirmed empty.
#[test]
fn tree_check_waits_out_a_zombie_member_until_it_is_reaped() {
    let mut member = exited_unreaped_group_member();
    let pgid = libc::pid_t::try_from(member.id()).unwrap();
    // Reaping is the event the check waits for. The delay only makes the
    // check see the zombie first; the bound it races is the production one.
    let reaper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        member.wait()
    });

    assert_eq!(
        await_group_exit(pgid, TREE_EXIT_BOUND),
        TreeTermination::Confirmed
    );
    assert!(reaper.join().unwrap().unwrap().success());
}

/// A zombie that is never reaped within the bound must not be reported as an
/// empty group. macOS answers EPERM for a zombie-only group; Linux still counts
/// the zombie as a member.
#[test]
fn tree_check_does_not_confirm_a_group_whose_zombie_outlives_the_bound() {
    let mut member = exited_unreaped_group_member();
    let pgid = libc::pid_t::try_from(member.id()).unwrap();

    let outcome = await_group_exit(pgid, Duration::from_millis(50));
    member.wait().unwrap();

    #[cfg(target_os = "macos")]
    assert_eq!(outcome, TreeTermination::CheckFailed);
    #[cfg(target_os = "linux")]
    assert_eq!(outcome, TreeTermination::StillPresent);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    assert_ne!(outcome, TreeTermination::Confirmed);
}

/// Spawn `true` as the leader of its own group and block until it has exited,
/// leaving it a zombie: `WNOWAIT` observes the exit without reaping it.
fn exited_unreaped_group_member() -> std::process::Child {
    let member = Command::new("true").process_group(0).spawn().unwrap();
    let pid = libc::id_t::from(member.id());
    // SAFETY: `siginfo_t` is plain data; all-zero is a valid value.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid, writable `siginfo_t` for the call's duration.
    let rc = unsafe { libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOWAIT) };
    assert_eq!(rc, 0, "waitid: {}", std::io::Error::last_os_error());
    member
}

/// Kills the test's process group on drop, so a failing assertion cannot
/// leave a long-lived member running.
struct GroupKillGuard(libc::pid_t);

impl Drop for GroupKillGuard {
    fn drop(&mut self) {
        // SAFETY: `killpg` takes plain integers. ESRCH (already gone) is the
        // expected answer on the passing path and is ignored.
        unsafe { libc::killpg(self.0, libc::SIGKILL) };
    }
}

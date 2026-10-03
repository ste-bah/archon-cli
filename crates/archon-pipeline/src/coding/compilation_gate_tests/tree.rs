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
    let reaper = std::thread::spawn(move || member.wait());

    assert_eq!(
        await_group_exit(pgid, TREE_EXIT_BOUND),
        TreeTermination::Confirmed
    );
    let status = reaper.join().unwrap().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGKILL));
}

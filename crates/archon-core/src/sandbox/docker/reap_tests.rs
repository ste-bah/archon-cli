//! Reaping decides whether to destroy someone else's container, so both halves
//! of the decision are pinned here. The daemon side is proved against a real one
//! in `tests/sandbox_docker_world.rs`.

use super::*;

#[test]
fn a_listing_yields_one_candidate_per_container_and_skips_blank_lines() {
    let listed = parse_listing("alpha\towner-1\t4242\n\nbeta\towner-2\t7\n");

    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "alpha");
    assert_eq!(listed[0].owner, "owner-1");
    assert_eq!(listed[0].pid, Some(4242));
    assert_eq!(listed[1].name, "beta");
}

/// A container with no readable pid has nothing that will ever tear it down, so
/// it counts as ownerless rather than as protected.
#[test]
fn a_container_with_no_usable_pid_label_counts_as_dead() {
    let listed = parse_listing("alpha\towner-1\t\nbeta\towner-2\tnot-a-pid\n");
    let mut system = sysinfo::System::new();

    assert_eq!(listed[0].pid, None);
    assert_eq!(listed[1].pid, None);
    assert!(!owner_is_alive(&mut system, listed[0].pid));
    assert!(!owner_is_alive(&mut system, listed[1].pid));
}

/// The half that stops parallel Archon sessions destroying each other's
/// sandboxes: a live owner must read as live.
#[test]
fn a_running_owner_reads_as_alive() {
    let mut system = sysinfo::System::new();

    assert!(
        owner_is_alive(&mut system, Some(std::process::id())),
        "this process is running, so a container it owns must not be reaped"
    );
}

/// Pid 0 is not a process any owner can be. Not merely "unlikely to exist" —
/// picking an arbitrary high pid would make this test flaky on a busy machine.
#[test]
fn a_pid_that_cannot_name_a_process_reads_as_dead() {
    let mut system = sysinfo::System::new();

    assert!(!owner_is_alive(&mut system, Some(0)));
}

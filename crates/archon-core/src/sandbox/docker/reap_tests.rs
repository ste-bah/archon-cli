//! Reaping decides whether to destroy someone else's container, so both halves
//! of the decision are pinned here. The daemon side is proved against a real one
//! in `tests/sandbox_docker_world.rs`; the bound on its docker calls is proved
//! here against fake docker binaries.

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
///
/// Asking the OS is not enough: on Windows pid 0 is the System Idle Process,
/// which exists and never exits, so a container labelled `0` read as protected
/// forever and was never reaped. `owner_is_alive` rules zero out itself.
#[test]
fn a_pid_that_cannot_name_a_process_reads_as_dead() {
    let mut system = sysinfo::System::new();

    assert!(!owner_is_alive(&mut system, Some(0)));
}

/// Reaping runs before the first container of a process. A daemon that does
/// not answer the listing must not hold that first command for ever.
#[cfg(unix)]
#[tokio::test]
async fn a_listing_the_daemon_never_answers_skips_reaping_within_the_bound() {
    use super::super::fake_docker::{FakeDocker, TEST_BOUND, WELL_UNDER_A_HANG};
    let fake = FakeDocker::new("case \"$1\" in ps) sleep 30 ;; *) exit 0 ;; esac");
    let started = std::time::Instant::now();

    let result = reap_orphans(fake.binary(), TEST_BOUND).await;

    assert!(
        result.is_err_and(|error| error.is_no_answer()),
        "the caller must hear the daemon did not answer"
    );

    assert!(
        started.elapsed() < WELL_UNDER_A_HANG,
        "the bound did not fire"
    );
    assert!(
        fake.calls().iter().all(|call| !call.starts_with("rm")),
        "nothing listed, so nothing may be removed: {:?}",
        fake.calls()
    );
}

/// Each further removal would wait out the same bound, so the first one the
/// daemon does not answer ends reaping.
#[cfg(unix)]
#[tokio::test]
async fn reaping_stops_at_the_first_removal_the_daemon_never_answers() {
    use super::super::fake_docker::{FakeDocker, TEST_BOUND, WELL_UNDER_A_HANG};
    let fake = FakeDocker::new(
        "case \"$1\" in\n  ps) printf 'orphan-a\\towner-x\\t0\\norphan-b\\towner-y\\t0\\n' ;;\n  rm) sleep 30 ;;\nesac",
    );
    let started = std::time::Instant::now();

    let result = reap_orphans(fake.binary(), TEST_BOUND).await;

    assert!(result.is_err_and(|error| error.is_no_answer()));

    assert!(
        started.elapsed() < WELL_UNDER_A_HANG,
        "the bound did not fire"
    );
    let removals: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("rm"))
        .collect();
    assert_eq!(removals, vec!["rm --force orphan-a"]);
}

/// An answered reap removes every dead owner's container.
#[cfg(unix)]
#[tokio::test]
async fn an_answered_reap_removes_each_orphan() {
    use super::super::fake_docker::{FakeDocker, TEST_BOUND};
    let fake = FakeDocker::new(
        "case \"$1\" in\n  ps) printf 'orphan-a\\towner-x\\t0\\norphan-b\\towner-y\\t0\\n' ;;\n  rm) exit 0 ;;\nesac",
    );

    assert_eq!(reap_orphans(fake.binary(), TEST_BOUND).await, Ok(()));

    let removals: Vec<_> = fake
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("rm"))
        .collect();
    assert_eq!(removals, vec!["rm --force orphan-a", "rm --force orphan-b"]);
}

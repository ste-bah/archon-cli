// A site restored after one check's fault is the same site as the batch's
// first, and it is restored under the same build cache lock: what the live
// repository names differently, or another observation waiting on the cache,
// never changes the batch.
#![cfg(unix)]

#[path = "support/isolation_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::{CheckAllowance, CheckResult, ObserveHooks};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use support::*;

/// Hooks whose `on_check` calls `each` with the count of checks finished.
fn counting(each: impl Fn(usize) + Send + Sync + 'static) -> ObserveHooks {
    let finished = AtomicUsize::new(0);
    let mut hooks = ObserveHooks::for_resource_limits(Default::default());
    hooks.on_check = Some(Arc::new(move |_: &CheckResult| {
        each(finished.fetch_add(1, Ordering::SeqCst) + 1);
    }));
    hooks
}

fn assert_all_ran_but_the_first(result: &archon_workflow::acceptance_scratch::ObservationResult) {
    assert!(result.operational_errors.is_empty(), "{result:?}");
    let own = result.checks[0].operational_error.as_deref();
    assert!(is_checks_own(own), "the first check's own fault: {own:?}");
    for check in &result.checks[1..] {
        assert_eq!(check.exit_code, Some(0), "{check:?}");
        assert!(check.operational_error.is_none(), "{check:?}");
    }
}

fn assert_one_attributed_teardown(result: &archon_workflow::acceptance_scratch::ObservationResult) {
    let attribution_notes = result
        .checks
        .iter()
        .filter_map(|check| check.operational_error.as_deref())
        .map(|message| {
            message
                .matches("the scratch site could not be removed after this check:")
                .count()
        })
        .sum::<usize>();
    assert_eq!(attribution_notes, 1, "{result:?}");
    // D-A: attribution belongs to the owning check and is not a batch
    // operational error; this field deliberately changes that behavior.
    assert!(result.operational_errors.is_empty(), "{result:?}");
    assert!(
        result.teardown_verified,
        "attributed teardown does not void the observation: {result:?}"
    );
}

fn hooks_with_short_checks() -> ObserveHooks {
    let mut hooks = ObserveHooks::for_resource_limits(Default::default());
    hooks.allowance = Some(Arc::new(move || CheckAllowance::Run {
        timeout_secs: 1,
        cut: false,
    }));
    hooks
}

/// F1: the site was clean before one check used it. An immutable file
/// left by that check belongs to its author; an earlier verdict stays and the
/// next check is host-unproven because the site could not be rebuilt.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_unremovable_site_is_attributed_once() {
    let mut clear_immutable = None;
    let (_temp, result) = observe_hooked(
        &[
            ("sleep 30", "repo_root"),
            (
                "mkdir data/blocked && touch data/blocked/keep && chflags uchg data/blocked/keep && sleep 30",
                "project_root",
            ),
            ("rm data/value && true", "project_root"),
        ],
        |root, _| clear_immutable = Some(ClearImmutable(root.join("scratch"))),
        &hooks_with_short_checks(),
    )
    .await;
    let _clear = clear_immutable.unwrap();
    // D-A moves the attributed teardown note out of operational_errors.
    assert_one_attributed_teardown(&result);
    assert!(result.teardown_verified, "{result:?}");
    assert_eq!(
        result.checks.len(),
        3,
        "one result per planned check: {result:?}"
    );
    assert!(
        result.checks[0]
            .operational_error
            .as_deref()
            .is_some_and(|error| error.contains("timed out")),
        "{result:?}"
    );
    assert_eq!(
        result.checks[1].fault,
        Some(archon_workflow::acceptance_scratch::faults::FaultCause::SiteTeardownFailed),
        "{result:?}"
    );
    assert!(
        result.checks[2]
            .operational_error
            .as_deref()
            .is_some_and(|error| error.contains("host-unproven")),
        "{result:?}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_unattributed_restore_failure_uses_a_scratch_error_label() {
    let mut clear_immutable = None;
    let (_temp, result) = observe_hooked(
        &[
            ("test -f data/value", "project_root"),
            (
                "mkdir data/blocked && touch data/blocked/keep && chflags uchg data/blocked/keep && sleep 30",
                "project_root",
            ),
            ("test -f data/value", "project_root"),
        ],
        |root, _| clear_immutable = Some(ClearImmutable(root.join("scratch"))),
        &hooks_with_short_checks(),
    )
    .await;
    let _clear = clear_immutable.unwrap();
    let later = result.checks.last().unwrap();
    let error = later.operational_error.as_deref().unwrap_or_default();
    assert!(error.contains("host-unproven"), "{later:?}");
    assert!(
        error.contains("scratch site could not be restored for the remaining checks"),
        "{later:?}"
    );
    assert!(!error.contains("invalid workflow spec"), "{later:?}");
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_leaked_cached_site_does_not_block_the_next_observation() {
    let cache = tempfile::tempdir().unwrap();
    let mut clear_immutable = None;
    let (_temp, first) = observe_hooked(
        &[(
            "mkdir blocked && touch blocked/keep && chflags uchg blocked/keep && sleep 30",
            "project_root",
        )],
        |_, policy| {
            policy.build_cache = Some(cache.path().to_path_buf());
            clear_immutable = Some(ClearImmutable(cache.path().join("scratch-0")));
        },
        &hooks_with_short_checks(),
    )
    .await;
    assert!(first.attributed_teardown.is_some(), "{first:?}");
    assert!(cache.path().join("scratch-0").exists());
    let guard = clear_immutable.expect("guard was installed before observation");

    let (_next_temp, next) = observe_hooked(
        &[("test -f data/value", "project_root")],
        |_, policy| policy.build_cache = Some(cache.path().to_path_buf()),
        &hooks_with_short_checks(),
    )
    .await;
    assert!(next.operational_errors.is_empty(), "{next:?}");
    assert!(next.teardown_verified, "{next:?}");
    drop(guard);
}

#[cfg(target_os = "macos")]
struct ClearImmutable(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for ClearImmutable {
    fn drop(&mut self) {
        let _ = std::process::Command::new("chflags")
            .args(["-R", "nouchg"])
            .arg(&self.0)
            .status();
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_unremovable_repository_path_is_attributed_once() {
    let mut clear_immutable = None;
    let (_temp, result) = observe_hooked(
        &[
            ("sleep 30", "repo_root"),
            (
                "mkdir blocked && touch blocked/keep && chflags uchg blocked/keep && sleep 30",
                "repo_root",
            ),
            ("test -f data/value", "project_root"),
        ],
        |root, _| clear_immutable = Some(ClearImmutable(root.join("scratch"))),
        &hooks_with_short_checks(),
    )
    .await;
    let _clear = clear_immutable.unwrap();
    // D-A moves the attributed teardown note out of operational_errors.
    assert_eq!(
        result.checks.len(),
        3,
        "one result per planned check: {result:?}"
    );
    assert!(
        result.checks[0]
            .operational_error
            .as_deref()
            .is_some_and(|error| error.contains("timed out")),
        "{result:?}"
    );
    assert_eq!(
        result.checks[1].fault,
        Some(archon_workflow::acceptance_scratch::faults::FaultCause::SiteTeardownFailed),
        "{result:?}"
    );
    assert_one_attributed_teardown(&result);
    assert!(
        result.checks[2]
            .operational_error
            .as_deref()
            .is_some_and(|e| e.contains("host-unproven")),
        "{result:?}"
    );
}

/// H3: in the combined view the project copy's `.git` links the site's own
/// worktree, whose administrative directory name the live repository picks.
/// Here the first site gets a suffixed name (another worktree holds the
/// plain one), that worktree goes away while the observation runs, and the
/// restored site gets the plain name. It is the same site: no host change,
/// and every check after the fault runs.
#[tokio::test]
async fn a_restored_site_given_another_worktree_name_is_not_a_host_change() {
    let other = tempfile::tempdir().unwrap();
    let holder = other.path().join("repo");
    let live = Arc::new(Mutex::new(None::<PathBuf>));
    let (seen, freed) = (live.clone(), holder.clone());
    let hooks = counting(move |finished| {
        if finished == 1 {
            let repository = seen.lock().unwrap().clone().unwrap();
            let freed = freed.to_str().unwrap();
            git(&repository, &["worktree", "remove", "--force", freed]);
        }
    });
    let (_t, result) = observe_hooked(
        &[
            ("rm -f target && test -f data/value", "project_root"),
            ("test -f data/value", "project_root"),
            ("test -f input", "project_root"),
        ],
        |_, policy| {
            let held = holder.to_str().unwrap();
            git(&policy.repository, &["worktree", "add", "--detach", held]);
            *live.lock().unwrap() = Some(policy.repository.clone());
        },
        &hooks,
    )
    .await;
    assert_all_ran_but_the_first(&result);
}

/// H4: an observation holds its build cache's lock for its whole life. A
/// second holder that starts waiting on the lock right after a check's
/// fault (before the site is restored) gets it only after the last check.
#[tokio::test]
async fn a_second_holder_cannot_take_the_build_cache_between_checks() {
    let lock = Arc::new(Mutex::new(None::<PathBuf>));
    let taken_after = Arc::new(Mutex::new(None::<usize>));
    let finished = Arc::new(AtomicUsize::new(0));
    let contender = Arc::new(Mutex::new(None::<std::thread::JoinHandle<()>>));
    let (path, taken, count, thread) = (
        lock.clone(),
        taken_after.clone(),
        finished.clone(),
        contender.clone(),
    );
    let hooks = counting(move |n| {
        count.store(n, Ordering::SeqCst);
        if n == 1 {
            let path = path.lock().unwrap().clone().unwrap();
            let (taken, count) = (taken.clone(), count.clone());
            *thread.lock().unwrap() = Some(std::thread::spawn(move || {
                use std::os::fd::AsRawFd;
                let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) }, 0);
                *taken.lock().unwrap() = Some(count.load(Ordering::SeqCst));
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            }));
        }
    });
    let (_t, result) = observe_hooked(
        &[
            ("rm -f target && test -f data/value", "project_root"),
            ("test -f data/value", "project_root"),
            ("test -f input", "project_root"),
        ],
        |root, policy| {
            policy.build_cache = Some(root.join("cache"));
            *lock.lock().unwrap() = Some(root.join("cache/lock"));
        },
        &hooks,
    )
    .await;
    let waiter = contender
        .lock()
        .unwrap()
        .take()
        .expect("the contender started");
    waiter.join().unwrap();
    assert_all_ran_but_the_first(&result);
    assert_eq!(
        *taken_after.lock().unwrap(),
        Some(3),
        "the second holder took the cache only after the observation's last check"
    );
}

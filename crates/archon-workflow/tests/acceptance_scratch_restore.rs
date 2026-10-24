// A site restored after one check's fault is the same site as the batch's
// first, and it is restored under the same build cache lock: what the live
// repository names differently, or another observation waiting on the cache,
// never changes the batch.
#![cfg(unix)]

#[path = "support/isolation_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::{CheckResult, ObserveHooks};
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

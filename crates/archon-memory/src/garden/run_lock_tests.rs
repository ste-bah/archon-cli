//! Proof that the single-run lock actually excludes.
//!
//! These tests are the reason the lock is worth having. A lock that is merely
//! *present* is indistinguishable from one that works until two consolidations
//! overlap on a real store and something is deleted twice, at which point there
//! is no evidence left to diagnose. So each layer is pinned separately, and each
//! against contention rather than against a happy path.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use super::{RunLockOutcome, run_lock_path, with_run_lock};

#[test]
fn an_uncontended_run_takes_the_lock_and_returns_its_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = run_lock_path(dir.path());

    let outcome = with_run_lock(&path, || 7usize).expect("lock available");

    assert_eq!(
        outcome,
        RunLockOutcome::Ran(7),
        "an uncontended pass must run and hand back what it produced"
    );
}

#[test]
fn a_second_run_is_declined_while_the_first_holds_the_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = run_lock_path(dir.path());

    // The inner attempt happens while the outer closure is still on the stack,
    // so the outer lock is provably still held. Nesting is the cheapest honest
    // way to express "at the same time" without a sleep.
    let outcome = with_run_lock(&path, || {
        with_run_lock(&path, || unreachable!("the inner pass must not run"))
            .expect("inner attempt is not an error")
    })
    .expect("outer lock available");

    let RunLockOutcome::Ran(inner) = outcome else {
        panic!("the outer pass should have run");
    };
    assert!(
        inner.is_busy(),
        "a second consolidation over one store must be declined, not queued and \
         not run concurrently"
    );
}

#[test]
fn exactly_one_of_many_concurrent_threads_consolidates() {
    // The failure this whole module exists to prevent: several passes deciding
    // the same memory is a merge loser or below the staleness floor, and all of
    // them acting.
    const THREADS: usize = 16;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = Arc::new(run_lock_path(dir.path()));
    let barrier = Arc::new(Barrier::new(THREADS));
    // Counts passes that were INSIDE the closure at once, not passes that ran at
    // all. A lock that serialises correctly still lets all sixteen run one after
    // another, so the number that matters is the concurrent peak.
    let inside = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let ran = Arc::new(AtomicUsize::new(0));

    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            let inside = Arc::clone(&inside);
            let peak = Arc::clone(&peak);
            let ran = Arc::clone(&ran);
            scope.spawn(move || {
                barrier.wait();
                let outcome = with_run_lock(&path, || {
                    let concurrent = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(concurrent, Ordering::SeqCst);
                    // Long enough that a broken lock overlaps observably rather
                    // than by luck of scheduling.
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
                .expect("lock file usable");
                if !outcome.is_busy() {
                    ran.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });

    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "two consolidations were inside the lock at once; this is the state that \
         destroys memories"
    );
    assert!(
        ran.load(Ordering::SeqCst) >= 1,
        "contention must not starve every pass -- at least one has to consolidate"
    );
}

#[test]
fn a_lock_held_by_another_process_declines_this_one() {
    // Layer 2, proven on its own. The registry of held paths is per-process, so
    // it cannot see a foreign holder; only the file lock can. Taking the OS lock
    // through an independent handle reproduces exactly the kernel state a second
    // Archon process would present, without needing to spawn one.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = run_lock_path(dir.path());

    let foreign = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .expect("open lock file");
    let mut foreign_lock = fd_lock::RwLock::new(foreign);
    let held = foreign_lock
        .try_write()
        .expect("foreign holder takes the lock");

    let outcome = with_run_lock(&path, || unreachable!("must not run while foreign-held"))
        .expect("a foreign holder is not an error");

    assert!(
        outcome.is_busy(),
        "a consolidation running in another process must exclude this one"
    );

    // Released, and the next attempt succeeds -- the interesting half, because a
    // lock that never frees looks identical to one that works until you need it.
    drop(held);
    let after = with_run_lock(&path, || 1usize).expect("lock free again");
    assert_eq!(after, RunLockOutcome::Ran(1));
}

#[test]
fn a_panicking_pass_still_releases_the_lock() {
    // A consolidation that panics must not take the store's ability to
    // consolidate down with it for the rest of the process's life. That failure
    // would present as "the garden silently stopped running", which is the
    // hardest kind to notice.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = run_lock_path(dir.path());

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = with_run_lock(&path, || panic!("consolidation blew up"));
    }));
    assert!(
        panicked.is_err(),
        "the panic must propagate, not be swallowed"
    );

    let after = with_run_lock(&path, || 2usize).expect("lock reusable after a panic");
    assert_eq!(
        after,
        RunLockOutcome::Ran(2),
        "the next pass must be able to proceed from a killed run's state"
    );
}

#[test]
fn two_different_stores_do_not_block_each_other() {
    // The lock is per store. One user consolidating a project store must not
    // stop another store's pass, or a single stuck run becomes a global outage.
    let a = tempfile::tempdir().expect("tempdir a");
    let b = tempfile::tempdir().expect("tempdir b");
    let path_a = run_lock_path(a.path());
    let path_b = run_lock_path(b.path());

    let outcome = with_run_lock(&path_a, || {
        with_run_lock(&path_b, || 3usize).expect("second store's lock is free")
    })
    .expect("first store's lock is free");

    assert_eq!(outcome, RunLockOutcome::Ran(RunLockOutcome::Ran(3)));
}

#[test]
fn the_lock_directory_is_created_if_it_does_not_exist() {
    // The first ever run on a fresh install finds no data dir. Failing there
    // would mean consolidation never starts on exactly the machines that have
    // never consolidated.
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = dir.path().join("does").join("not").join("exist");
    let path = run_lock_path(&nested);

    let outcome = with_run_lock(&path, || 4usize).expect("missing directory is created");

    assert_eq!(outcome, RunLockOutcome::Ran(4));
    assert!(path.exists(), "the lock file itself must be left behind");
}

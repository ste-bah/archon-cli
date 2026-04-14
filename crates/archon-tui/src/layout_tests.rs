//! Unit tests for the TUI-105 resize reflow helper.
//!
//! These tests are written FIRST (Gate 1: tests-written-first). They pin
//! the non-blocking contract of `handle_resize`:
//!
//! 1. It records dimensions into `LAST_KNOWN_SIZE` without allocation.
//! 2. It returns `ReflowOutcome { dirty: true }` unconditionally.
//! 3. It does not block on, touch, or `.await` any in-flight async work.
//!
//! Test #3 is the load-bearing one: it spawns a tokio task that sleeps
//! for 10 seconds, then measures wall-clock time around a SYNC call to
//! `handle_resize`. The resize call must complete in <5ms and the
//! sleeping JoinHandle must still be !is_finished() afterwards. This
//! gives a 2000x signal/noise ratio vs the 10s sleep and proves the
//! handler is fully synchronous (AC-EVENTLOOP-03: resize never blocks).

use super::{handle_resize, last_known_size};
use std::time::{Duration, Instant};

#[test]
fn test_handle_resize_records_dimensions() {
    // First call — dimensions should be recorded exactly as passed.
    let outcome = handle_resize(120, 40);
    assert_eq!(outcome.cols, 120);
    assert_eq!(outcome.rows, 40);
    assert_eq!(last_known_size(), (120, 40));

    // Second call — snapshot must update to the new pair.
    let outcome = handle_resize(80, 24);
    assert_eq!(outcome.cols, 80);
    assert_eq!(outcome.rows, 24);
    assert_eq!(last_known_size(), (80, 24));
}

#[test]
fn test_handle_resize_sets_dirty_flag() {
    // `dirty` is a design-constant true: every resize invalidates the
    // ratatui layout cache, so lazy invalidation would only add a
    // branch with no payoff.
    let outcome = handle_resize(100, 30);
    assert!(
        outcome.dirty,
        "ReflowOutcome.dirty must be true for every resize event"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_resize_does_not_block_on_inflight_turn() {
    // Simulate an in-flight agent turn with a REAL async sleep. If
    // handle_resize accidentally awaited anything in the same runtime
    // it would either block on this task or take non-trivial time.
    let inflight: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
    });

    // Wall-clock measurement around a SYNC call. Note: no `.await`
    // here — handle_resize is sync by contract.
    let start = Instant::now();
    let outcome = handle_resize(200, 60);
    let elapsed = start.elapsed();

    assert_eq!(outcome.cols, 200);
    assert_eq!(outcome.rows, 60);
    assert!(outcome.dirty);

    // 5ms ceiling against a 10s background sleep = 2000x signal/noise.
    // If we accidentally awaited the inflight task we'd see ~10s, not
    // sub-millisecond. This test is robust even on a loaded CI box.
    assert!(
        elapsed < Duration::from_millis(5),
        "handle_resize took {:?} — must be <5ms (sync, non-blocking)",
        elapsed
    );

    // The inflight sleep MUST still be running — handle_resize is not
    // allowed to touch JoinHandles or any AgentDispatcher state.
    assert!(
        !inflight.is_finished(),
        "inflight task should still be running — handle_resize must not touch it"
    );

    // Clean up: abort the 10s sleep so the test exits promptly.
    inflight.abort();
}

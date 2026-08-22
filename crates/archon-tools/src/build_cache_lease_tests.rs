use super::*;

/// The property the whole design rests on: a slot's directory survives its
/// occupant, so the next agent to hold it finds the previous build intact.
/// Without this every agent rebuilds the dependency graph from nothing, which
/// is what fifteen sequential tasks cost before the pool existed.
#[tokio::test]
async fn a_released_slot_keeps_what_the_previous_holder_built() {
    let root = tempfile::tempdir().expect("tempdir");
    let pool = BuildCachePool::new(root.path(), 2);

    let first = pool.acquire().await.expect("first lease");
    std::fs::write(first.dir().join("artifact.rlib"), b"built").expect("write");
    let first_dir = first.dir().to_path_buf();
    drop(first);

    let second = pool.acquire().await.expect("second lease");

    assert_eq!(second.dir(), first_dir, "the slot must be reused");
    assert!(
        second.dir().join("artifact.rlib").exists(),
        "the previous build must survive the handover"
    );
}

/// The other half: two agents holding leases at the same time must never be
/// pointed at one directory. Concurrent builds of divergent checkouts into a
/// shared cache is the case Cargo has no safe answer for.
#[tokio::test]
async fn concurrent_holders_never_share_a_directory() {
    let root = tempfile::tempdir().expect("tempdir");
    let pool = BuildCachePool::new(root.path(), 3);

    let a = pool.acquire().await.expect("a");
    let b = pool.acquire().await.expect("b");
    let c = pool.acquire().await.expect("c");

    let dirs = [a.dir(), b.dir(), c.dir()];
    for (i, left) in dirs.iter().enumerate() {
        for right in dirs.iter().skip(i + 1) {
            assert_ne!(left, right, "concurrent leases must be distinct");
        }
    }
}

/// A full pool makes the next agent wait rather than inventing a directory,
/// because inventing one restores the unbounded disk growth the pool exists to
/// bound — and does it exactly when the machine is busiest.
#[tokio::test]
async fn a_full_pool_makes_the_next_agent_wait() {
    let root = tempfile::tempdir().expect("tempdir");
    let pool = BuildCachePool::new(root.path(), 1);

    let held = pool.acquire().await.expect("held");

    let waiting = tokio::time::timeout(std::time::Duration::from_millis(50), pool.acquire()).await;
    assert!(waiting.is_err(), "a full pool must not hand out a slot");

    drop(held);
    let after = tokio::time::timeout(std::time::Duration::from_millis(500), pool.acquire()).await;
    assert!(after.is_ok(), "releasing must let the waiter through");
}

/// A dropped lease returns its slot even when the agent never released it
/// cleanly — a cancelled or panicking agent must not shrink the pool.
#[tokio::test]
async fn a_slot_returns_when_its_holder_is_dropped_abruptly() {
    let root = tempfile::tempdir().expect("tempdir");
    let pool = BuildCachePool::new(root.path(), 1);

    let slot = {
        let lease = pool.acquire().await.expect("lease");
        lease.slot()
    };

    let again = pool.acquire().await.expect("reacquire");
    assert_eq!(again.slot(), slot, "the slot must come back");
}

/// Reuse prefers the warmest directory: a run that never reaches its
/// concurrency limit should keep returning to slot 0 rather than spreading cold
/// caches across the pool.
#[tokio::test]
async fn sequential_agents_keep_returning_to_the_warmest_slot() {
    let root = tempfile::tempdir().expect("tempdir");
    let pool = BuildCachePool::new(root.path(), 4);

    for _ in 0..3 {
        let lease = pool.acquire().await.expect("lease");
        assert_eq!(lease.slot(), 0, "sequential work must reuse one slot");
    }
}

/// A pool of zero would block every build forever. Clamping to one serialises
/// them instead, which is slow but survivable.
#[tokio::test]
async fn a_zero_sized_pool_is_clamped_rather_than_deadlocking() {
    let root = tempfile::tempdir().expect("tempdir");
    let pool = BuildCachePool::new(root.path(), 0);

    assert_eq!(pool.slots(), 1);
    let lease = tokio::time::timeout(std::time::Duration::from_millis(500), pool.acquire()).await;
    assert!(lease.is_ok(), "a clamped pool must still hand out a slot");
}

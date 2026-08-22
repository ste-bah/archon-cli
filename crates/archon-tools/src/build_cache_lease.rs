//! Leasing a build-cache slot to one agent at a time.
//!
//! The pool holds a fixed number of slots. An agent takes one for the length of
//! its work and gives it back; while it holds the slot nothing else may use it,
//! and when it lets go the directory stays exactly as it left it.
//!
//! Both halves matter and they pull in opposite directions. Exclusivity is what
//! makes the cache safe — two agents compiling divergent checkouts into one
//! directory is the case every build tool warns about, and Cargo says outright
//! there is no safe way to do it. Persistence is what makes the cache useful —
//! a directory thrown away with the agent that made it means the next agent
//! rebuilds everything, which is what fifteen sequential tasks each paying a
//! full dependency build looked like in practice.
//!
//! A slot gives both, because a slot outlives its occupants without ever having
//! two at once.
//!
//! Slots are handed out by the lowest free index rather than round-robin, so a
//! run that never reaches its concurrency limit keeps reusing the warmest
//! directories instead of spreading cold ones across the pool.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A held slot. Releases on drop, so an agent that panics or is cancelled does
/// not strand its slot — the pool would shrink silently until nothing could
/// build.
#[derive(Debug)]
pub struct BuildCacheLease {
    slot: usize,
    dir: PathBuf,
    taken: Arc<Mutex<BTreeSet<usize>>>,
    _permit: OwnedSemaphorePermit,
}

impl BuildCacheLease {
    /// The directory this lease owns, stable for the slot across occupants.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Which slot is held. Exposed for observability — an operator reading logs
    /// should be able to see which agent had which cache.
    pub fn slot(&self) -> usize {
        self.slot
    }
}

impl Drop for BuildCacheLease {
    fn drop(&mut self) {
        if let Ok(mut taken) = self.taken.lock() {
            taken.remove(&self.slot);
        }
    }
}

/// A fixed set of reusable build-cache directories.
#[derive(Debug, Clone)]
pub struct BuildCachePool {
    root: PathBuf,
    permits: Arc<Semaphore>,
    taken: Arc<Mutex<BTreeSet<usize>>>,
    slots: usize,
}

impl BuildCachePool {
    /// A pool of `slots` directories under `root`.
    ///
    /// `slots` is clamped to at least one: a pool that can hand out nothing
    /// would block every build forever, which is a worse failure than a pool of
    /// one that merely serialises them.
    pub fn new(root: impl Into<PathBuf>, slots: usize) -> Self {
        let slots = slots.max(1);
        Self {
            root: root.into(),
            permits: Arc::new(Semaphore::new(slots)),
            taken: Arc::new(Mutex::new(BTreeSet::new())),
            slots,
        }
    }

    /// How many agents may build at once.
    pub fn slots(&self) -> usize {
        self.slots
    }

    /// Take a slot, waiting if every one is busy.
    ///
    /// Waiting is deliberate. The alternative — inventing an extra directory
    /// when the pool is full — would silently restore the unbounded disk growth
    /// the pool exists to stop, and would do it exactly when the machine is
    /// already at its busiest.
    pub async fn acquire(&self) -> std::io::Result<BuildCacheLease> {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| std::io::Error::other("build cache pool closed while acquiring a slot"))?;
        let slot = self.claim_lowest_free_slot();
        let dir = crate::build_cache_env::lease_slot_dir(&self.root, slot);
        std::fs::create_dir_all(&dir)?;
        Ok(BuildCacheLease {
            slot,
            dir,
            taken: Arc::clone(&self.taken),
            _permit: permit,
        })
    }

    /// The lowest index not currently held.
    ///
    /// The permit already guarantees a free slot exists, so the scan terminates
    /// — but it is bounded by the pool size anyway rather than trusting that
    /// invariant to hold after some future edit.
    fn claim_lowest_free_slot(&self) -> usize {
        let mut taken = self.taken.lock().unwrap_or_else(|err| err.into_inner());
        let slot = (0..self.slots)
            .find(|slot| !taken.contains(slot))
            .unwrap_or(0);
        taken.insert(slot);
        slot
    }
}

/// The pool every isolated agent in this process leases from.
///
/// Process-wide because sharing is the entire mechanism: a pool per agent would
/// give each one slot 0 of its own private pool, which is the per-agent
/// directory this replaced under a new name. One pool, and slots move between
/// agents as they finish.
static SHARED_POOL: std::sync::OnceLock<BuildCachePool> = std::sync::OnceLock::new();

/// Install the shared pool, if nothing has installed one yet.
///
/// Returns whether this call was the one that set it. Later calls do not
/// resize: agents may already be holding slots from the existing pool, and
/// shrinking underneath them would hand two agents one directory — the exact
/// thing the pool exists to prevent.
pub fn install_shared_build_cache_pool(root: impl Into<PathBuf>, slots: usize) -> bool {
    let pool = BuildCachePool::new(root, slots);
    SHARED_POOL.set(pool).is_ok()
}

/// The shared pool, or `None` when nothing installed one.
///
/// `None` is a real answer rather than a reason to build a default: an
/// interactive session has no use for a leased cache, and inventing a pool for
/// it would put build output somewhere the user did not ask for.
pub fn shared_build_cache_pool() -> Option<BuildCachePool> {
    SHARED_POOL.get().cloned()
}

#[cfg(test)]
#[path = "build_cache_lease_tests.rs"]
mod tests;

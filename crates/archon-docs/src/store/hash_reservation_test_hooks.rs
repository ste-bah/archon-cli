//! Test-only instrumentation for the content-hash reservation window.
//!
//! The duplicate-document defect needs two ingests that have each read
//! `doc_sources` before either has written to it. That window is invisible from
//! the outside — both callers just return, and whether a duplicate appears is
//! decided by scheduling. So the window is measured from inside instead: a
//! rendezvous parked between the read and the write records how many ingests
//! were ever in there at once, which is a direct statement of the invariant
//! rather than an inference from the row count.
//!
//! Keyed by content hash so arming it for one test cannot perturb the other
//! tests sharing this binary: an ingest of any other content walks straight
//! past.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Default)]
struct RendezvousState {
    /// Ingests currently parked between their read and their write.
    in_flight: usize,
    /// High-water mark of `in_flight` — the thing actually under test.
    peak_in_flight: usize,
    /// Cumulative arrivals, so a test can tell "never overlapped" apart from
    /// "nobody ever got here".
    arrivals: usize,
}

/// A rendezvous that gives up rather than blocking forever.
///
/// It has to time out. Once reservations serialise on the database write lock
/// the second ingest is parked on that lock and can never arrive, so a plain
/// `Barrier` would make the first wait for it forever. The timeout is what lets
/// the same hook express both outcomes: unserialised peers meet in
/// milliseconds, serialised ones never meet at all.
pub(crate) struct ReservationRendezvous {
    state: Mutex<RendezvousState>,
    ready: Condvar,
    parties: usize,
    timeout: Duration,
}

impl ReservationRendezvous {
    pub(crate) fn new(parties: usize, timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RendezvousState::default()),
            ready: Condvar::new(),
            parties,
            timeout,
        })
    }

    /// Highest number of ingests ever simultaneously inside the window.
    pub(crate) fn peak_in_flight(&self) -> usize {
        self.locked().peak_in_flight
    }

    /// How many ingests reached the window at all.
    pub(crate) fn arrivals(&self) -> usize {
        self.locked().arrivals
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, RendezvousState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn arrive(&self) {
        let mut state = self.locked();
        state.arrivals += 1;
        state.in_flight += 1;
        state.peak_in_flight = state.peak_in_flight.max(state.in_flight);

        if state.in_flight >= self.parties {
            self.ready.notify_all();
        } else {
            let deadline = Instant::now() + self.timeout;
            while state.in_flight < self.parties {
                let Some(remaining) = deadline
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                else {
                    break;
                };
                let (next, wait) = self
                    .ready
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state = next;
                if wait.timed_out() {
                    break;
                }
            }
        }

        state.in_flight -= 1;
    }
}

type PauseRegistry = Mutex<HashMap<String, Arc<ReservationRendezvous>>>;

static PAUSES: OnceLock<PauseRegistry> = OnceLock::new();

fn pauses() -> &'static PauseRegistry {
    PAUSES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn registry() -> std::sync::MutexGuard<'static, HashMap<String, Arc<ReservationRendezvous>>> {
    pauses()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Disarms the pause for one content hash when the test drops it.
pub(crate) struct ReservationPauseGuard(String);

impl Drop for ReservationPauseGuard {
    fn drop(&mut self) {
        registry().remove(&self.0);
    }
}

/// Park every reservation of `content_hash` on `rendezvous` until the guard drops.
pub(crate) fn pause_before_reservation_for_tests(
    content_hash: &str,
    rendezvous: Arc<ReservationRendezvous>,
) -> ReservationPauseGuard {
    registry().insert(content_hash.to_owned(), rendezvous);
    ReservationPauseGuard(content_hash.to_owned())
}

/// Called from the reservation itself, after the read and before the write.
pub(crate) fn wait_before_reservation(content_hash: &str) {
    // Clone the handle out before arriving: the rendezvous parks for up to its
    // timeout, and holding the registry lock across that would serialise every
    // other ingest in the binary behind it.
    let rendezvous = registry().get(content_hash).map(Arc::clone);
    if let Some(rendezvous) = rendezvous {
        rendezvous.arrive();
    }
}

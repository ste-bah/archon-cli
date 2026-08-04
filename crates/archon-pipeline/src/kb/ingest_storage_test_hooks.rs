use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::kb::ingest_storage::ChunkStorage;

const RESERVATION_CONFLICT: &str = "reserve content hashes failed: when executing against relation 'kb_content_hashes' :: key exists in database";
type ConflictWriter = Box<dyn FnOnce(String) + Send>;
type ConflictWriterSlot = Arc<Mutex<Option<ConflictWriter>>>;

#[derive(Default)]
struct RendezvousState {
    /// Threads currently parked between their stale read and their reservation.
    in_flight: usize,
    /// High-water mark of `in_flight` — the thing actually under test.
    peak_in_flight: usize,
    /// Cumulative arrivals, so a test can tell "never overlapped" apart from
    /// "one of them never got here".
    arrivals: usize,
}

/// A rendezvous that gives up rather than blocking forever, and records how
/// many ingest threads were ever inside the reservation window at once.
///
/// The duplicate-node defect needs two writers holding reads taken before
/// either committed. This hook sits at exactly that point, so its high-water
/// mark is a direct measurement: a peak of 1 means the reservation window was
/// mutually exclusive, a peak of 2 means two writers were racing on stale
/// snapshots and the outcome was left to chance.
///
/// It has to time out rather than block. A plain `Barrier` used to express the
/// rendezvous and cannot any more: now that reservations serialise on the
/// database write lock, the second thread is parked on that lock and can never
/// arrive, so the first would wait for it forever.
pub(super) struct ReservationRendezvous {
    state: Mutex<RendezvousState>,
    ready: Condvar,
    parties: usize,
    timeout: Duration,
}

impl ReservationRendezvous {
    pub(super) fn new(parties: usize, timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RendezvousState::default()),
            ready: Condvar::new(),
            parties,
            timeout,
        })
    }

    /// Highest number of threads ever simultaneously inside the reservation.
    pub(super) fn peak_in_flight(&self) -> usize {
        self.state
            .lock()
            .expect("reservation rendezvous lock")
            .peak_in_flight
    }

    /// How many threads reached the reservation at all.
    pub(super) fn arrivals(&self) -> usize {
        self.state
            .lock()
            .expect("reservation rendezvous lock")
            .arrivals
    }

    fn arrive(&self) {
        let mut state = self.state.lock().expect("reservation rendezvous lock");
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
                    .expect("reservation rendezvous lock");
                state = next;
                if wait.timed_out() {
                    break;
                }
            }
        }

        state.in_flight -= 1;
    }
}

#[derive(Default)]
pub(super) struct ReservationTestHooks {
    rendezvous: Arc<Mutex<Option<Arc<ReservationRendezvous>>>>,
    failure: Arc<Mutex<Option<String>>>,
    conflict_writer: Arc<Mutex<Option<ConflictWriterSlot>>>,
    pending_conflict_writer: Arc<Mutex<Option<ConflictWriterSlot>>>,
}

impl ReservationTestHooks {
    pub(super) fn inject_failure(&self) -> Result<()> {
        if let Some(writer) = self.take_conflict_writer() {
            *self
                .pending_conflict_writer
                .lock()
                .expect("reservation hook lock") = Some(writer);
            anyhow::bail!(RESERVATION_CONFLICT);
        }
        if let Some(message) = self
            .failure
            .lock()
            .expect("reservation failure lock")
            .take()
        {
            anyhow::bail!(message);
        }
        Ok(())
    }

    pub(super) fn persist_conflict_after_abort(&self, hash: &str) {
        let writer = self
            .pending_conflict_writer
            .lock()
            .expect("reservation hook lock")
            .take();
        if let Some(writer) =
            writer.and_then(|writer| writer.lock().expect("reservation hook lock").take())
        {
            writer(hash.to_owned());
        }
    }

    fn take_conflict_writer(&self) -> Option<ConflictWriterSlot> {
        self.conflict_writer
            .lock()
            .expect("reservation hook lock")
            .take()
    }

    pub(super) fn wait_before_reservation(&self) {
        let rendezvous = self
            .rendezvous
            .lock()
            .expect("reservation rendezvous slot")
            .take();
        if let Some(rendezvous) = rendezvous {
            rendezvous.arrive();
        }
    }
}

pub(super) struct ReservationConflictHook(ConflictWriterSlot);

impl ReservationConflictHook {
    pub(super) fn was_consumed(&self) -> bool {
        self.0.lock().expect("reservation hook lock").is_none()
    }
}

pub(super) struct ReservationFailureGuard(Arc<Mutex<Option<String>>>);

impl Drop for ReservationFailureGuard {
    fn drop(&mut self) {
        *self.0.lock().expect("reservation failure lock") = None;
    }
}

pub(super) struct ReservationRendezvousGuard(Arc<Mutex<Option<Arc<ReservationRendezvous>>>>);

impl Drop for ReservationRendezvousGuard {
    fn drop(&mut self) {
        *self.0.lock().expect("reservation rendezvous slot") = None;
    }
}

impl ChunkStorage {
    pub(super) fn fail_hash_reservation_for_tests(
        &self,
        message: impl Into<String>,
        _rendezvous: Option<Arc<ReservationRendezvous>>,
    ) -> ReservationFailureGuard {
        let slot = Arc::clone(&self.test_hooks.failure);
        *slot.lock().expect("reservation failure lock") = Some(message.into());
        ReservationFailureGuard(slot)
    }

    pub(super) fn pause_before_hash_reservation_for_tests(
        &self,
        rendezvous: Arc<ReservationRendezvous>,
    ) -> ReservationRendezvousGuard {
        let slot = Arc::clone(&self.test_hooks.rendezvous);
        *slot.lock().expect("reservation rendezvous slot") = Some(rendezvous);
        ReservationRendezvousGuard(slot)
    }

    pub(super) fn persist_conflict_then_fail_hash_reservation_for_tests<F>(
        &self,
        writer: F,
    ) -> ReservationConflictHook
    where
        F: FnOnce(String) + Send + 'static,
    {
        let slot = Arc::new(Mutex::new(Some(Box::new(writer) as ConflictWriter)));
        *self
            .test_hooks
            .conflict_writer
            .lock()
            .expect("reservation hook lock") = Some(Arc::clone(&slot));
        ReservationConflictHook(slot)
    }
}

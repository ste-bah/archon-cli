use std::sync::{Arc, Barrier, Mutex};

use anyhow::Result;

use crate::kb::ingest_storage::ChunkStorage;

const RESERVATION_CONFLICT: &str = "reserve content hashes failed: when executing against relation 'kb_content_hashes' :: key exists in database";
type ConflictWriter = Box<dyn FnOnce(String) + Send>;
type ConflictWriterSlot = Arc<Mutex<Option<ConflictWriter>>>;

#[derive(Default)]
pub(super) struct ReservationTestHooks {
    barrier: Arc<Mutex<Option<Arc<Barrier>>>>,
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
        if let Some(barrier) = self
            .barrier
            .lock()
            .expect("reservation barrier lock")
            .take()
        {
            barrier.wait();
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

pub(super) struct ReservationBarrierGuard(Arc<Mutex<Option<Arc<Barrier>>>>);

impl Drop for ReservationBarrierGuard {
    fn drop(&mut self) {
        *self.0.lock().expect("reservation barrier lock") = None;
    }
}

impl ChunkStorage {
    pub(super) fn fail_hash_reservation_for_tests(
        &self,
        message: impl Into<String>,
        _barrier: Option<Arc<Barrier>>,
    ) -> ReservationFailureGuard {
        let slot = Arc::clone(&self.test_hooks.failure);
        *slot.lock().expect("reservation failure lock") = Some(message.into());
        ReservationFailureGuard(slot)
    }

    pub(super) fn pause_before_hash_reservation_for_tests(
        &self,
        barrier: Arc<Barrier>,
    ) -> ReservationBarrierGuard {
        let slot = Arc::clone(&self.test_hooks.barrier);
        *slot.lock().expect("reservation barrier lock") = Some(barrier);
        ReservationBarrierGuard(slot)
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

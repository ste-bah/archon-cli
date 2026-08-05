use std::sync::{Arc, RwLock};

/// The session row that persistence and usage updates must land in.
///
/// This exists because the id is minted unconditionally at startup
/// (`main_bootstrap`), *before* any resume is considered, and the TUI picker
/// resumes mid-session. Every consumer that copied the id at setup therefore
/// went stale the moment a user resumed, and a single conversation ended up
/// split across two rows: `post_turn` wrote history under one id while
/// `update_usage` wrote cost under another. Observed live -- resuming a
/// 2-turn session and running one turn left the resumed row untouched and
/// moved all six messages into the launch row.
///
/// Holding one shared cell instead of N copies makes "which session am I
/// writing to" answerable at write time rather than at startup.
#[derive(Clone)]
pub(crate) struct ActiveSessionId(Arc<RwLock<String>>);

impl ActiveSessionId {
    pub(crate) fn new(id: impl Into<String>) -> Self {
        Self(Arc::new(RwLock::new(id.into())))
    }

    /// The id writes should currently target.
    ///
    /// Returns an owned `String` rather than a guard on purpose: callers hold
    /// this across `.await` points, and a lock guard held across an await is
    /// both a `Send` problem and a deadlock waiting to happen.
    pub(crate) fn get(&self) -> String {
        match self.0.read() {
            Ok(id) => id.clone(),
            // A poisoned lock still holds a valid id -- the writer panicked
            // between two `String` assignments, it did not corrupt the value.
            // Refusing to read here would lose the session instead.
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Point subsequent writes at `id`.
    ///
    /// Called when a resume adopts an existing session, so that continued work
    /// lands in the session the user picked rather than in the row this
    /// process happened to create at launch.
    pub(crate) fn set(&self, id: impl Into<String>) {
        let id = id.into();
        match self.0.write() {
            Ok(mut slot) => *slot = id,
            Err(poisoned) => *poisoned.into_inner() = id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_observe_a_later_set() {
        let original = ActiveSessionId::new("launch-row");
        let shared = original.clone();
        assert_eq!(shared.get(), "launch-row");

        // This is the whole point: the forwarder's copy must follow a resume
        // that happens after it was constructed.
        original.set("resumed-row");

        assert_eq!(shared.get(), "resumed-row");
        assert_eq!(original.get(), "resumed-row");
    }

    #[test]
    fn get_survives_a_poisoned_lock() {
        let id = ActiveSessionId::new("kept");
        let poisoner = id.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.0.write().expect("write lock");
            panic!("poison the lock");
        })
        .join();

        assert_eq!(id.get(), "kept", "a poisoned lock must not lose the id");
    }
}

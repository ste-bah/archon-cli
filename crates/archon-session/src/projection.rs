//! Derived session state, folded once (#193 Phase B).
//!
//! The TUI, the web workbench and the SDK each derive their own view of a
//! session by walking the message log themselves. Three walks, three sets of
//! rules, three places for them to disagree — and every one of them starts from
//! zero on resume.
//!
//! A projection is one pure unit per derived value: `init` for the empty state,
//! `apply` for one event, `view` for what a client should receive. The driver
//! subscribes to the log once and folds every event through every unit.
//! Domains hold no subscriptions and clients never fold; they receive finished
//! values.
//!
//! Two rules carry most of the value:
//!
//! - **`apply` is synchronous.** An async unit would tear the consistency cut
//!   across carriers: two clients could observe the same session at different
//!   points in the fold and both be told they are current.
//! - **A unit uninterested in an event returns the same state.** Rust makes
//!   that natural — hand back the `Arc` you were given and the driver compares
//!   pointers. Unchanged identity means provably zero downstream work, which is
//!   what makes folding every event through every unit affordable.

use std::collections::BTreeMap;
use std::sync::Arc;

use cozo::{DataValue, ScriptMutability};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::storage::{SessionError, SessionStore, db_err, extract_str};

/// Remove every cached projection belonging to one session.
///
/// Shared with `delete_session`, which needs it inside its own transaction
/// rather than as a standalone write.
pub(crate) const PROJECTIONS_RM_FOR_SESSION: &str = "?[session_id, projection_key] := *session_projections{session_id, projection_key},      session_id = $sid :rm session_projections {session_id, projection_key}";

/// One committed entry in a session log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEvent {
    /// Position in the log. Monotonic, and the resume point of the fold.
    pub seq: u64,
    /// The stored message, verbatim.
    pub payload: String,
}

/// A pure derivation from a session's event stream.
///
/// Implementors hold no state and no subscriptions. The driver owns both.
pub trait SessionProjection {
    /// Plain serialisable data — the precondition for the persisted cache.
    /// A state holding a handle or a channel could not be resumed.
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Stable identity, used as the cache key. Renaming it discards the cache
    /// rather than corrupting it, which is the safe direction.
    fn key(&self) -> &'static str;

    /// The state of a session with no events.
    fn init(&self) -> Self::State;

    /// Fold one event.
    ///
    /// Return `state` unchanged when the event is not interesting. The driver
    /// checks with `Arc::ptr_eq`, so an uninterested unit costs one pointer
    /// comparison and writes nothing.
    fn apply(&self, state: Arc<Self::State>, event: &SessionEvent) -> Arc<Self::State>;

    /// What a client receives. Deliberately separate from `State`: the stored
    /// shape is free to change without changing the wire.
    fn view(&self, state: &Self::State) -> serde_json::Value;
}

/// A projection's state and how far it has been folded.
#[derive(Debug, Clone)]
pub struct Projected<S> {
    pub state: Arc<S>,
    /// The seq of the last event folded in, or `None` for an empty session.
    pub through_seq: Option<u64>,
    /// Whether this fold advanced the cache. False means the cached state was
    /// already current, which is the common case on a resumed session.
    pub advanced: bool,
}

impl SessionStore {
    /// Fold `unit` over the session's log and return the derived state.
    ///
    /// Resumes from the persisted cache rather than refolding from zero. The
    /// cache is written only when the fold actually advanced: an unchanged
    /// state is not worth a round trip, and rewriting it would make "advanced"
    /// unobservable.
    pub fn project<P: SessionProjection>(
        &self,
        session_id: &str,
        unit: &P,
    ) -> Result<Projected<P::State>, SessionError> {
        let cached = self.load_projection_cache::<P>(session_id, unit.key())?;
        let (mut state, resume_after) = match cached {
            Some((state, seq)) => (Arc::new(state), Some(seq)),
            None => (Arc::new(unit.init()), None),
        };

        let messages = self.load_messages(session_id)?;
        let start = resume_after.map_or(0, |seq| seq.saturating_add(1) as usize);

        let mut through_seq = resume_after;
        let mut advanced = false;
        for (offset, payload) in messages.iter().enumerate().skip(start) {
            let event = SessionEvent {
                seq: offset as u64,
                payload: payload.clone(),
            };
            let next = unit.apply(Arc::clone(&state), &event);
            // Identity, not equality: a unit says "not interested" by handing
            // back what it was given, and that must not require the state to be
            // comparable or cheap to compare.
            if !Arc::ptr_eq(&next, &state) {
                state = next;
                advanced = true;
            }
            through_seq = Some(event.seq);
        }

        if advanced {
            self.save_projection_cache(session_id, unit.key(), state.as_ref(), through_seq)?;
        }

        Ok(Projected {
            state,
            through_seq,
            advanced,
        })
    }

    /// Drop every projection cached for a session.
    ///
    /// The driver resumes strictly *after* the cached `event_seq`, so a cache
    /// only ever moves forward. That is correct while the log only grows, and
    /// wrong the moment it does not: `replace_messages` rewrites it,
    /// `truncate_messages_after` shortens it and `delete_all_messages` empties
    /// it, and after any of those the cached state describes messages the store
    /// no longer holds — with no seq the fold would ever revisit to notice.
    ///
    /// Every log-rewriting method calls this, rather than each projection
    /// guarding itself. A unit is a pure fold over the events it was given; it
    /// has no way to learn that the events were withdrawn, and making that
    /// every author's problem would mean rediscovering this bug once per
    /// projection.
    pub fn invalidate_all_projections(&self, session_id: &str) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        self.db()
            .run_mutable(
                PROJECTIONS_RM_FOR_SESSION,
                params,
                "session store: invalidate all projections",
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Drop a projection's cache, so the next fold starts from `init`.
    ///
    /// For a unit whose logic changed: the cached state was produced by the old
    /// rules and refolding is the only way to be sure it agrees with the new
    /// ones.
    pub fn invalidate_projection(
        &self,
        session_id: &str,
        projection_key: &str,
    ) -> Result<(), SessionError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert(
            "projection_key".to_string(),
            DataValue::from(projection_key),
        );
        self.db()
            .run_mutable(
                "?[session_id, projection_key] <- [[$session_id, $projection_key]] \
                 :rm session_projections {session_id, projection_key}",
                params,
                "session store: invalidate projection",
            )
            .map_err(db_err)?;
        Ok(())
    }

    fn load_projection_cache<P: SessionProjection>(
        &self,
        session_id: &str,
        projection_key: &str,
    ) -> Result<Option<(P::State, u64)>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pkey".to_string(), DataValue::from(projection_key));
        let result = self
            .db()
            .run_script(
                "?[event_seq, state] := *session_projections{session_id, projection_key, event_seq, state}, \
                 session_id = $sid, projection_key = $pkey",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let Some(row) = result.rows.first() else {
            return Ok(None);
        };
        let Some(seq) = row[0]
            .get_int()
            .filter(|seq| *seq >= 0)
            .map(|seq| seq as u64)
        else {
            return Ok(None);
        };
        // A cache that will not deserialise is treated as absent rather than
        // fatal: the state can always be rebuilt from the log, and refusing to
        // open a session because a derived value went stale would be the wrong
        // trade entirely.
        Ok(serde_json::from_str(&extract_str(&row[1]))
            .ok()
            .map(|state| (state, seq)))
    }

    fn save_projection_cache<S: Serialize>(
        &self,
        session_id: &str,
        projection_key: &str,
        state: &S,
        through_seq: Option<u64>,
    ) -> Result<(), SessionError> {
        let Some(seq) = through_seq else {
            return Ok(());
        };
        let Ok(encoded) = serde_json::to_string(state) else {
            // A state that will not serialise cannot be cached, but the fold
            // that produced it is still valid. Refolding next time is slower,
            // not wrong.
            return Ok(());
        };

        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert(
            "projection_key".to_string(),
            DataValue::from(projection_key),
        );
        params.insert("event_seq".to_string(), DataValue::from(seq as i64));
        params.insert("state".to_string(), DataValue::from(encoded));
        self.db()
            .run_mutable(
                "?[session_id, projection_key, event_seq, state] <- \
                 [[$session_id, $projection_key, $event_seq, $state]] \
                 :put session_projections {session_id, projection_key => event_seq, state}",
                params,
                "session store: save projection cache",
            )
            .map_err(db_err)?;
        Ok(())
    }
}

//! A session's *current surface*, as a projection (#193 Phase B, #200 Phase 4).
//!
//! The stored log and the context a session actually carries are not the same
//! thing, and the gap is not small. Staged segment compaction never rewrites
//! `state.messages`: it closes a span of the log into a `compaction_segments`
//! row, has it summarised, and swaps the summary in at *request assembly*
//! time (`archon_core::agent::segment_compaction::assemble_compacted_messages`).
//! The verbatim messages stay in the log forever. So the log holds content the
//! session itself has already decided is not worth carrying, and anything that
//! reads the log and calls the result "that session's context" is wrong by
//! exactly the amount that session compacted.
//!
//! This unit folds the log into what the session would carry now: every
//! message that is still live, and one stand-in per closed segment in place of
//! the span it replaced.
//!
//! Two things about the shape, both following the rules in [`crate::projection`]:
//!
//! - The state stores *indices*, not payloads. A projection whose state is a
//!   copy of the log would double the store and make the cache as expensive to
//!   write as the log is to read. [`SessionSurfaceState::resolve`] pairs the
//!   indices back with the messages at the point of use.
//! - A message inside a closed segment, past that segment's first index, adds
//!   nothing to the surface — the stand-in is already there. That is the
//!   "uninterested" case, and it hands back the same `Arc`. It is also why
//!   `replaced_messages` is derived from the entries rather than counted into
//!   the state: a counter would make every such event a change.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::projection::{Projected, SessionEvent, SessionProjection};
use crate::storage::{CompactionSegment, CompactionSummaryStatus, SessionError, SessionStore};

/// One position on a session's current surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceEntry {
    /// A stored message the session still carries verbatim.
    Live { index: u64 },
    /// A closed compaction segment, standing in for `first_index..=last_index`
    /// of the log.
    Compacted {
        first_index: u64,
        last_index: u64,
        segment_id: String,
        /// The text the source session puts in the span's place.
        stand_in: String,
    },
}

/// The surface of a session with no events, grown one event at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSurfaceState {
    /// Which segment set this state was folded under.
    ///
    /// The cache key is a `&'static str`, so it cannot carry the segments the
    /// fold depended on. Recording them in the state is what lets a reader
    /// notice that a segment closed since the cache was written and refold
    /// instead of serving a surface that is one compaction out of date.
    pub segments_fingerprint: String,
    /// The surface, oldest first.
    pub entries: Vec<SurfaceEntry>,
}

impl SessionSurfaceState {
    /// Stored messages the surface no longer carries verbatim.
    #[must_use]
    pub fn replaced_messages(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| match entry {
                SurfaceEntry::Live { .. } => 0,
                SurfaceEntry::Compacted {
                    first_index,
                    last_index,
                    ..
                } => last_index.saturating_sub(*first_index).saturating_add(1),
            })
            .sum()
    }

    /// Pair the surface's indices back with the log they refer to.
    ///
    /// An index past the end of `messages` is dropped rather than panicking:
    /// the fold and this read are two separate queries, and a source session
    /// that rewrote its log between them would otherwise take the reader down
    /// with it.
    #[must_use]
    pub fn resolve(&self, messages: &[String]) -> Vec<SurfaceMessage> {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                SurfaceEntry::Live { index } => {
                    messages.get(*index as usize).map(|payload| SurfaceMessage {
                        first_index: *index,
                        last_index: *index,
                        compacted: false,
                        payload: payload.clone(),
                    })
                }
                SurfaceEntry::Compacted {
                    first_index,
                    last_index,
                    stand_in,
                    ..
                } => Some(SurfaceMessage {
                    first_index: *first_index,
                    last_index: *last_index,
                    compacted: true,
                    // The same shape the live agent assembles, so a reader
                    // renders a stand-in exactly as it renders a message.
                    payload: serde_json::json!({ "role": "user", "content": stand_in }).to_string(),
                }),
            })
            .collect()
    }
}

/// One resolved position on the surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceMessage {
    /// Where in the raw log this entry starts.
    pub first_index: u64,
    /// Where it ends. Equal to `first_index` for a live message.
    pub last_index: u64,
    /// Whether this is a compaction stand-in rather than a stored message.
    pub compacted: bool,
    /// The stored message, or the stand-in rendered as one.
    pub payload: String,
}

/// A session's surface, resolved against its log.
#[derive(Debug, Clone)]
pub struct SessionSurface {
    /// Messages the log holds, live and compacted-away alike.
    pub messages_total: usize,
    /// Stored messages the surface no longer carries verbatim.
    pub replaced_messages: u64,
    /// The surface itself, oldest first.
    pub messages: Vec<SurfaceMessage>,
}

/// Folds a session log into [`SessionSurfaceState`] under a fixed segment set.
#[derive(Debug, Clone)]
pub struct SessionSurfaceProjection {
    /// Closed segments, sorted, non-overlapping.
    spans: Vec<SurfaceEntry>,
    fingerprint: String,
}

impl SessionSurfaceProjection {
    /// Read the session's closed compaction segments and build the unit.
    pub fn for_session(store: &SessionStore, session_id: &str) -> Result<Self, SessionError> {
        let mut spans: Vec<SurfaceEntry> = Vec::new();
        let mut cursor: Option<u64> = None;
        // `list_compaction_segments` already sorts by start_index.
        for segment in store.list_compaction_segments(session_id)? {
            if segment.start_index > segment.end_index {
                continue;
            }
            // Overlap is not expected — segments are closed strictly after the
            // previous one's end — but a surface built from overlapping spans
            // would double-count, so the later one is dropped rather than
            // trusted.
            if cursor.is_some_and(|end| segment.start_index <= end) {
                continue;
            }
            cursor = Some(segment.end_index);
            spans.push(SurfaceEntry::Compacted {
                first_index: segment.start_index,
                last_index: segment.end_index,
                stand_in: stand_in_for(&segment),
                segment_id: segment.id,
            });
        }
        let fingerprint = fingerprint(&spans);
        Ok(Self { spans, fingerprint })
    }

    /// The span covering `index`, if any.
    fn span_covering(&self, index: u64) -> Option<&SurfaceEntry> {
        self.spans.iter().find(|span| match span {
            SurfaceEntry::Compacted {
                first_index,
                last_index,
                ..
            } => (*first_index..=*last_index).contains(&index),
            SurfaceEntry::Live { .. } => false,
        })
    }

    /// Whether a fold's result was produced under different inputs than these.
    fn is_stale(&self, projected: &Projected<SessionSurfaceState>, log_len: usize) -> bool {
        if projected.state.segments_fingerprint != self.fingerprint {
            return true;
        }
        // A log that shrank under the cache — `replace_messages` after a
        // compaction does exactly that — leaves the cached state folded
        // through events that no longer exist. The driver resumes *after* the
        // cached seq, so it would never notice on its own.
        projected
            .through_seq
            .is_some_and(|seq| seq as usize + 1 != log_len)
    }
}

impl SessionProjection for SessionSurfaceProjection {
    type State = SessionSurfaceState;

    fn key(&self) -> &'static str {
        "session_surface.v1"
    }

    fn init(&self) -> Self::State {
        SessionSurfaceState {
            segments_fingerprint: self.fingerprint.clone(),
            entries: Vec::new(),
        }
    }

    fn apply(&self, state: Arc<Self::State>, event: &SessionEvent) -> Arc<Self::State> {
        match self.span_covering(event.seq) {
            // Inside a closed segment, past its head: the stand-in is already
            // on the surface and this message is not. Nothing to record.
            Some(SurfaceEntry::Compacted { first_index, .. }) if *first_index != event.seq => state,
            Some(span) => {
                let mut next = (*state).clone();
                next.entries.push(span.clone());
                Arc::new(next)
            }
            None => {
                let mut next = (*state).clone();
                next.entries.push(SurfaceEntry::Live { index: event.seq });
                Arc::new(next)
            }
        }
    }

    fn view(&self, state: &Self::State) -> serde_json::Value {
        let compacted = state
            .entries
            .iter()
            .filter(|entry| matches!(entry, SurfaceEntry::Compacted { .. }))
            .count();
        serde_json::json!({
            "entries": state.entries.len(),
            "live": state.entries.len() - compacted,
            "compacted_spans": compacted,
            "replaced_messages": state.replaced_messages(),
        })
    }
}

/// The session's current surface: what it would carry into its next turn.
///
/// Refolds rather than serving a cache that was written under a different
/// segment set or a different log — see [`SessionSurfaceProjection::is_stale`].
pub fn session_surface(
    store: &SessionStore,
    session_id: &str,
) -> Result<SessionSurface, SessionError> {
    let unit = SessionSurfaceProjection::for_session(store, session_id)?;
    let mut projected = store.project(session_id, &unit)?;
    let messages = store.load_messages(session_id)?;
    if unit.is_stale(&projected, messages.len()) {
        store.invalidate_projection(session_id, unit.key())?;
        projected = store.project(session_id, &unit)?;
    }
    Ok(SessionSurface {
        messages_total: messages.len(),
        replaced_messages: projected.state.replaced_messages(),
        messages: projected.state.resolve(&messages),
    })
}

/// What the source session puts in a closed segment's place.
///
/// The succeeded case mirrors `assemble_compacted_messages`. The others do not
/// reconstruct the span's content the way the live agent's deterministic
/// digest does — that lives above this crate, and for a *cross-session* read
/// naming the span is the safer answer anyway: the source has already decided
/// those messages are not part of its context.
fn stand_in_for(segment: &CompactionSegment) -> String {
    match segment.summary_status {
        CompactionSummaryStatus::Redacted => {
            format!("[Compacted segment {}: redacted]", segment.id)
        }
        CompactionSummaryStatus::Succeeded => match segment
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
        {
            Some(summary) => format!("[Compacted segment {}]\n{summary}", segment.id),
            None => unsummarised(segment),
        },
        _ => unsummarised(segment),
    }
}

fn unsummarised(segment: &CompactionSegment) -> String {
    format!(
        "[Compacted segment {}: covers messages {}–{} of that session's log, which it \
         compacted away; no summary is stored for them]",
        segment.id, segment.start_index, segment.end_index
    )
}

/// A stable digest of the segment set a fold depended on.
///
/// FNV-1a rather than `DefaultHasher`: this value is written to disk and
/// compared against on a later run, and `DefaultHasher`'s output is explicitly
/// not guaranteed stable across builds.
fn fingerprint(spans: &[SurfaceEntry]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    };
    for span in spans {
        if let SurfaceEntry::Compacted {
            first_index,
            last_index,
            segment_id,
            stand_in,
        } = span
        {
            feed(&first_index.to_le_bytes());
            feed(&last_index.to_le_bytes());
            feed(segment_id.as_bytes());
            feed(stand_in.as_bytes());
        }
    }
    format!("fnv1a64:{spans_len}:{hash:016x}", spans_len = spans.len())
}

#[cfg(test)]
#[path = "projection_surface_tests.rs"]
mod tests;

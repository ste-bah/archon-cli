//! Per-message human feedback (#193 Phase C).
//!
//! Archon has eight learning subsystems, reasoning-quality events, trust
//! scores, completion evidence and false-completion incidents — all of it
//! machine-derived. They can observe what happened and infer whether it worked;
//! they cannot observe whether the person reading it thought the answer was any
//! good. That is the one input they cannot synthesise, and it is cheap to
//! collect.
//!
//! A sidecar relation, deliberately not a session-log event. Two reasons:
//!
//! - Feedback is editable. A rating can be changed or withdrawn, and the
//!   session log is the record of what happened, not of what someone later
//!   thought about it.
//! - Keeping it out of the log keeps it out of model context, which is correct:
//!   this is a signal for the learning layer, not for the next turn. A model
//!   that could see its last answer was rated badly would start writing for the
//!   rating.
//!
//! Nothing about the agent loop changes. A session with no feedback is
//! indistinguishable from one before this existed.

use std::collections::BTreeMap;

use cozo::{DataValue, ScriptMutability};

use crate::storage::{SessionError, SessionStore, db_err, extract_str};

/// How the reader found one assistant message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Positive,
    Negative,
}

impl Rating {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "positive" => Some(Self::Positive),
            "negative" => Some(Self::Negative),
            _ => None,
        }
    }
}

/// One rating, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFeedback {
    pub message_id: String,
    pub rating: Rating,
    pub note: Option<String>,
    /// Opaque compare-and-set token, replaced on every material update.
    ///
    /// The TUI and the web workbench can both be open on one session, so
    /// last-write-wins would silently discard whichever edit lost the race.
    /// A writer must present the version it read.
    pub version: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackError {
    /// Someone else changed this rating since it was read.
    ///
    /// Carries what is there now, so a caller can show both rather than just
    /// reporting a collision.
    Conflict(Box<MessageFeedback>),
    /// The token presented does not match anything, and no rating exists to
    /// have produced it.
    NotFound,
}

impl std::fmt::Display for FeedbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(current) => write!(
                f,
                "this rating was changed elsewhere (now {})",
                current.rating.as_str()
            ),
            Self::NotFound => write!(f, "there is no rating here to change"),
        }
    }
}

impl std::error::Error for FeedbackError {}

impl SessionStore {
    /// Record or replace the rating on one assistant message.
    ///
    /// `expected_version` is `None` for the first rating on a message and the
    /// version last read for any change. A mismatch is refused rather than
    /// applied: the alternative is one of two open windows silently winning.
    pub fn set_feedback(
        &self,
        session_id: &str,
        message_id: &str,
        rating: Rating,
        note: Option<&str>,
        expected_version: Option<&str>,
    ) -> Result<MessageFeedback, SessionError> {
        let existing = self.feedback(session_id, message_id)?;
        check_version(existing.as_ref(), expected_version)?;

        let now = chrono::Utc::now().to_rfc3339();
        let record = MessageFeedback {
            message_id: message_id.to_string(),
            rating,
            note: note.map(str::to_string).filter(|note| !note.is_empty()),
            version: uuid::Uuid::new_v4().simple().to_string(),
            created_at: existing.map_or_else(|| now.clone(), |prior| prior.created_at),
            updated_at: now,
        };

        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("message_id".to_string(), DataValue::from(message_id));
        params.insert("rating".to_string(), DataValue::from(rating.as_str()));
        params.insert(
            "note".to_string(),
            DataValue::from(record.note.clone().unwrap_or_default()),
        );
        params.insert(
            "version".to_string(),
            DataValue::from(record.version.clone()),
        );
        params.insert(
            "created_at".to_string(),
            DataValue::from(record.created_at.clone()),
        );
        params.insert(
            "updated_at".to_string(),
            DataValue::from(record.updated_at.clone()),
        );

        self.db()
            .run_mutable(
                "?[session_id, message_id, rating, note, version, created_at, updated_at] <- \
                 [[$session_id, $message_id, $rating, $note, $version, $created_at, $updated_at]] \
                 :put message_feedback {session_id, message_id => rating, note, version, created_at, updated_at}",
                params,
                "session store: set message feedback",
            )
            .map_err(db_err)?;
        Ok(record)
    }

    /// Withdraw a rating.
    ///
    /// Withdrawing is not the same as rating something neutral, so the row goes
    /// rather than gaining a third state. Same compare-and-set rule.
    pub fn clear_feedback(
        &self,
        session_id: &str,
        message_id: &str,
        expected_version: Option<&str>,
    ) -> Result<(), SessionError> {
        let existing = self.feedback(session_id, message_id)?;
        if existing.is_none() {
            // Already absent. Withdrawing nothing is what the caller wanted.
            return Ok(());
        }
        check_version(existing.as_ref(), expected_version)?;

        let mut params = BTreeMap::new();
        params.insert("session_id".to_string(), DataValue::from(session_id));
        params.insert("message_id".to_string(), DataValue::from(message_id));
        self.db()
            .run_mutable(
                "?[session_id, message_id] <- [[$session_id, $message_id]] \
                 :rm message_feedback {session_id, message_id}",
                params,
                "session store: clear message feedback",
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// The rating on one message, if there is one.
    pub fn feedback(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<MessageFeedback>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("mid".to_string(), DataValue::from(message_id));
        let result = self
            .db()
            .run_script(
                "?[rating, note, version, created_at, updated_at] := \
                 *message_feedback{session_id, message_id, rating, note, version, created_at, updated_at}, \
                 session_id = $sid, message_id = $mid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        Ok(result.rows.first().and_then(|row| {
            Some(MessageFeedback {
                message_id: message_id.to_string(),
                rating: Rating::parse(&extract_str(&row[0]))?,
                note: Some(extract_str(&row[1])).filter(|note| !note.is_empty()),
                version: extract_str(&row[2]),
                created_at: extract_str(&row[3]),
                updated_at: extract_str(&row[4]),
            })
        }))
    }

    /// Every rating in a session, for the learning layer to consume.
    pub fn all_feedback(&self, session_id: &str) -> Result<Vec<MessageFeedback>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        let result = self
            .db()
            .run_script(
                "?[message_id, rating, note, version, created_at, updated_at] := \
                 *message_feedback{session_id, message_id, rating, note, version, created_at, updated_at}, \
                 session_id = $sid",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let mut rows: Vec<MessageFeedback> = result
            .rows
            .iter()
            .filter_map(|row| {
                Some(MessageFeedback {
                    message_id: extract_str(&row[0]),
                    rating: Rating::parse(&extract_str(&row[1]))?,
                    note: Some(extract_str(&row[2])).filter(|note| !note.is_empty()),
                    version: extract_str(&row[3]),
                    created_at: extract_str(&row[4]),
                    updated_at: extract_str(&row[5]),
                })
            })
            .collect();
        // Cozo returns rows in relation order; sort so two readers agree.
        rows.sort_by(|a, b| a.message_id.cmp(&b.message_id));
        Ok(rows)
    }
}

/// Refuse a write whose token does not match what is stored.
fn check_version(
    existing: Option<&MessageFeedback>,
    expected: Option<&str>,
) -> Result<(), SessionError> {
    match (existing, expected) {
        // First rating on this message, and the caller agrees there was none.
        (None, None) => Ok(()),
        // The caller thinks it is updating something that is no longer there.
        (None, Some(_)) => Err(SessionError::Feedback(FeedbackError::NotFound)),
        // Something is already here and the caller did not know.
        (Some(current), None) => Err(SessionError::Feedback(FeedbackError::Conflict(Box::new(
            current.clone(),
        )))),
        (Some(current), Some(expected)) if current.version == expected => Ok(()),
        (Some(current), Some(_)) => Err(SessionError::Feedback(FeedbackError::Conflict(Box::new(
            current.clone(),
        )))),
    }
}

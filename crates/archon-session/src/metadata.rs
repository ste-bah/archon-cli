//! Tag management for sessions.

use crate::storage::{SessionError, SessionStore};

/// Add a tag to a session.
///
/// Duplicate tags are silently ignored (CozoDB put is idempotent on the key).
pub fn add_tag(store: &SessionStore, session_id: &str, tag: &str) -> Result<(), SessionError> {
    store.put_tag(session_id, tag)
}

/// Remove a tag from a session.
pub fn remove_tag(store: &SessionStore, session_id: &str, tag: &str) -> Result<(), SessionError> {
    store.delete_tag(session_id, tag)
}

/// Get all tags for a session.
pub fn get_tags(store: &SessionStore, session_id: &str) -> Result<Vec<String>, SessionError> {
    store.list_tags(session_id)
}

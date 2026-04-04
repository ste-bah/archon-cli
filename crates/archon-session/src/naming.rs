//! Session naming: resolve, validate, and assign human-readable names.

use crate::storage::{SessionError, SessionMetadata, SessionStore};

/// Resolve a session by name.
///
/// Tries exact match first. If no exact match, falls back to unique prefix
/// matching. Returns `None` when zero or multiple sessions match a prefix.
pub fn resolve_by_name(
    store: &SessionStore,
    name: &str,
) -> Result<Option<SessionMetadata>, SessionError> {
    let matches = store.find_sessions_by_name_prefix(name)?;

    // Exact match takes priority
    for (sid, matched_name) in &matches {
        if matched_name == name {
            let meta = store.get_session(sid)?;
            return Ok(Some(meta));
        }
    }

    // Unique prefix match
    if matches.len() == 1 {
        let meta = store.get_session(&matches[0].0)?;
        return Ok(Some(meta));
    }

    // Zero matches or ambiguous prefix
    Ok(None)
}

/// Validate that a name is not already in use by another session.
pub fn validate_name(store: &SessionStore, name: &str) -> Result<(), String> {
    let matches = store
        .find_sessions_by_name_prefix(name)
        .map_err(|e| e.to_string())?;

    for (_sid, matched_name) in &matches {
        if matched_name == name {
            return Err(format!("session name '{name}' is already in use"));
        }
    }

    Ok(())
}

/// Assign a human-readable name to a session.
///
/// Overwrites any previous name for this session.
pub fn set_session_name(
    store: &SessionStore,
    session_id: &str,
    name: &str,
) -> Result<(), SessionError> {
    store.set_name(session_id, name)
}

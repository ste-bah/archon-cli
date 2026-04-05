use crate::storage::{SessionError, SessionMetadata, SessionStore};

/// Resume a session by ID or name.
///
/// Resolution order:
/// 1. Exact UUID match.
/// 2. UUID prefix match (unique).
/// 3. Exact session name match.
/// 4. Session name prefix match (unique).
///
/// Returns `SessionError::NotFound` if nothing matches, or if a prefix is
/// ambiguous across multiple sessions.
pub fn resume_session(
    store: &SessionStore,
    query: &str,
) -> Result<(SessionMetadata, Vec<String>), SessionError> {
    // Steps 1 + 2: exact / prefix UUID (get_session handles both).
    let meta = match store.get_session(query) {
        Ok(m) => m,
        Err(SessionError::NotFound(_)) => {
            // Step 3 + 4: try name lookup.
            let matches = store.find_sessions_by_name_prefix(query)?;
            match matches.len() {
                0 => {
                    return Err(SessionError::NotFound(format!(
                        "no session found matching '{query}' (tried ID and name)"
                    )));
                }
                1 => store.get_session(&matches[0].0)?,
                n => {
                    let names: Vec<String> = matches.iter().map(|(_, name)| name.clone()).collect();
                    return Err(SessionError::NotFound(format!(
                        "ambiguous name '{query}' matches {n} sessions: {}",
                        names.join(", ")
                    )));
                }
            }
        }
        Err(e) => return Err(e),
    };

    let messages = store.load_messages(&meta.id)?;
    Ok((meta, messages))
}

/// Format a session for display in the resume list.
pub fn format_session_line(session: &SessionMetadata) -> String {
    let id_short = if session.id.len() > 8 {
        &session.id[..8]
    } else {
        &session.id
    };

    let branch = session.git_branch.as_deref().unwrap_or("no branch");

    format!(
        "{id_short}  {dir}  ({branch})  {msgs} msgs  {tokens} tokens",
        dir = session.working_directory,
        msgs = session.message_count,
        tokens = session.total_tokens,
    )
}

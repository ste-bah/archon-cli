use crate::storage::{SessionError, SessionMetadata, SessionStore};

/// Resume a session by ID -- loads metadata and all messages.
pub fn resume_session(
    store: &SessionStore,
    session_id: &str,
) -> Result<(SessionMetadata, Vec<String>), SessionError> {
    let meta = store.get_session(session_id)?;
    let messages = store.load_messages(session_id)?;
    Ok((meta, messages))
}

/// Format a session for display in the resume list.
pub fn format_session_line(session: &SessionMetadata) -> String {
    let id_short = if session.id.len() > 8 {
        &session.id[..8]
    } else {
        &session.id
    };

    let branch = session
        .git_branch
        .as_deref()
        .unwrap_or("no branch");

    format!(
        "{id_short}  {dir}  ({branch})  {msgs} msgs  {tokens} tokens",
        dir = session.working_directory,
        msgs = session.message_count,
        tokens = session.total_tokens,
    )
}

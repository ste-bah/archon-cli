//! Session forking: create a new session from an existing one, copying all
//! messages and linking the parent.

use uuid::Uuid;

use crate::naming::set_session_name;
use crate::storage::{SessionError, SessionStore};

/// Fork a session, creating a new session with a copy of all messages.
///
/// The new session receives a fresh UUID and a link to the source session
/// via `parent_session_id`. If `new_name` is provided, the forked session
/// is assigned that human-readable name.
///
/// Returns the new session ID.
pub fn fork_session(
    store: &SessionStore,
    source_id: &str,
    new_name: Option<&str>,
) -> Result<String, SessionError> {
    let source = store.get_session(source_id)?;
    let new_id = Uuid::new_v4().to_string();

    // Create the new session with the same metadata as the source.
    store.register_session(
        &new_id,
        &source.working_directory,
        source.git_branch.as_deref(),
        &source.model,
    )?;

    // Copy all messages from source to the new session.
    let messages = store.load_messages(source_id)?;
    for (idx, content) in messages.iter().enumerate() {
        store.save_message(&new_id, idx as u64, content)?;
    }

    // Link the parent.
    store.set_parent(&new_id, source_id)?;

    // Optionally assign a name.
    if let Some(name) = new_name {
        set_session_name(store, &new_id, name)?;
    }

    Ok(new_id)
}

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
/// Fork a session keeping only the messages up to and including `through`.
///
/// `fork_session` copies the whole log, which answers "carry on from here in a
/// separate session". This answers the other question — "go back to before that
/// and try something else" — which had no implementation at all, so the branch
/// picker built for it had nothing to call (#192).
///
/// An index past the end keeps everything, which is the same as a plain fork
/// rather than an error: asking to branch after the last message is asking for
/// all of it.
pub fn fork_session_at(
    store: &SessionStore,
    source_id: &str,
    through: usize,
    new_name: Option<&str>,
) -> Result<String, SessionError> {
    fork_session_inner(store, source_id, Some(through), new_name)
}

pub fn fork_session(
    store: &SessionStore,
    source_id: &str,
    new_name: Option<&str>,
) -> Result<String, SessionError> {
    fork_session_inner(store, source_id, None, new_name)
}

fn fork_session_inner(
    store: &SessionStore,
    source_id: &str,
    through: Option<usize>,
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

    // Copy the messages the fork should carry. `through` is inclusive, so
    // branching at message 0 keeps the first message and nothing after it.
    let messages = store.load_messages(source_id)?;
    let keep = through.map_or(messages.len(), |index| {
        index.saturating_add(1).min(messages.len())
    });
    for (idx, content) in messages.iter().take(keep).enumerate() {
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

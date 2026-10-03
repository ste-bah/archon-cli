//! Conversation transcripts and descriptive sidecars live here.
//! These files are model input and presentation metadata, never resume authority.

use std::path::PathBuf;

/// `~/.archon/sessions`: every session's agent records. `None` when no home
/// directory is known.
pub fn sessions_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".archon").join("sessions"))
}

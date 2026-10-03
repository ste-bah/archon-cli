//! Where the host keeps the records of the agents it spawned (#241).
//!
//! Each spawned agent's transcript and metadata live under
//! `~/.archon/sessions/{session}/subagents/`. The metadata records the
//! confinement the agent was spawned with, and a resume restores the agent
//! from it. So the directory is the host's, never an agent's: an agent that
//! could write it could widen its own next resume, or a sibling's. The
//! transcript store and the path guard both take the directory from here, so
//! they cannot disagree about where it is.

use std::path::PathBuf;

/// `~/.archon/sessions`: every session's agent records. `None` when no home
/// directory is known.
pub fn sessions_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".archon").join("sessions"))
}

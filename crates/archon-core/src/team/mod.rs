//! Agent team support for TASK-CLI-312.
//!
//! Types live in archon-tools to avoid circular dependencies.
//! archon-core re-exports them here for the archon_core::team namespace.
//!
//! `TeamManager` used to keep its own idea of where teams live — `<project>/
//! teams/` against the tools' `<project>/.archon/teams/` — so `archon team
//! list` could never see a team `TeamCreate` had made. It now delegates to
//! `archon_tools::team_roster`, which is the single writer (#184 M5).

// Re-export shared types from archon-tools
pub use archon_tools::team_config;
pub use archon_tools::team_message as message;
pub use archon_tools::team_roster as roster;

use std::path::PathBuf;

use archon_tools::team_config::TeamConfig;

// ---------------------------------------------------------------------------
// TeamManager
// ---------------------------------------------------------------------------

/// Manages team lifecycle: load and list.
///
/// All team state lives under `<project_dir>/.archon/teams/<team-id>/`.
pub struct TeamManager {
    project_dir: PathBuf,
}

impl TeamManager {
    /// Create a manager rooted at `project_dir`.
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    /// The directory teams are read from — worth printing, because "no teams
    /// found" is otherwise indistinguishable from "looked in the wrong place".
    pub fn teams_dir(&self) -> PathBuf {
        roster::teams_dir(&self.project_dir)
    }

    /// Load a team configuration from disk.
    pub fn load_team(&self, team_id: &str) -> Result<TeamConfig, TeamError> {
        roster::load(&self.project_dir, team_id).map_err(TeamError::Roster)
    }

    /// List all team IDs currently on disk.
    pub fn list_teams(&self) -> Result<Vec<String>, TeamError> {
        Ok(roster::list(&self.project_dir))
    }
}

// ---------------------------------------------------------------------------
// TeamError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum TeamError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Roster(String),
    #[error("Team not found: {0}")]
    NotFound(String),
}

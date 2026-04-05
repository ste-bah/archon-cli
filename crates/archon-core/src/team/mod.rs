//! Agent team support for TASK-CLI-312.
//!
//! Types live in archon-tools to avoid circular dependencies.
//! archon-core re-exports them here for the archon_core::team namespace.

// Re-export shared types from archon-tools
pub use archon_tools::team_backend as backend;
pub use archon_tools::team_config;
pub use archon_tools::team_message as message;

use std::path::PathBuf;

use archon_tools::team_backend::FileBasedBackend;
use archon_tools::team_config::TeamConfig;

// ---------------------------------------------------------------------------
// TeamManager
// ---------------------------------------------------------------------------

/// Manages team lifecycle: create, load, delete.
///
/// All team state lives under `<teams_root>/teams/<team-id>/`.
pub struct TeamManager {
    /// Root directory containing all team subdirectories.
    teams_root: PathBuf,
}

impl TeamManager {
    /// Create a manager rooted at `project_dir`.
    ///
    /// Teams are stored in `<project_dir>/teams/`.
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            teams_root: project_dir.join("teams"),
        }
    }

    /// Create a new team: write `team.json` and per-member inbox files.
    ///
    /// Does NOT spawn agent processes — creates config files only.
    pub fn create_team(&mut self, config: TeamConfig) -> Result<(), TeamError> {
        let team_dir = self.teams_root.join(&config.id);
        std::fs::create_dir_all(&team_dir)?;

        // Write team.json
        let json = serde_json::to_string_pretty(&config).map_err(TeamError::Serde)?;
        std::fs::write(team_dir.join("team.json"), json)?;

        // Create empty inbox files for each member
        for member in &config.members {
            let inbox_path = team_dir.join(format!("inbox-{}.jsonl", member.role));
            if !inbox_path.exists() {
                std::fs::write(&inbox_path, "")?;
            }
        }

        Ok(())
    }

    /// Load a team configuration from disk.
    pub fn load_team(&self, team_id: &str) -> Result<TeamConfig, TeamError> {
        let config_path = self.teams_root.join(team_id).join("team.json");
        let json = std::fs::read_to_string(&config_path)?;
        let config: TeamConfig = serde_json::from_str(&json).map_err(TeamError::Serde)?;
        Ok(config)
    }

    /// Delete a team and all associated files.
    pub fn delete_team(&mut self, team_id: &str) -> Result<(), TeamError> {
        let team_dir = self.teams_root.join(team_id);
        if team_dir.exists() {
            std::fs::remove_dir_all(&team_dir)?;
        }
        Ok(())
    }

    /// List all team IDs currently on disk.
    pub fn list_teams(&self) -> Result<Vec<String>, TeamError> {
        if !self.teams_root.exists() {
            return Ok(vec![]);
        }
        let ids = std::fs::read_dir(&self.teams_root)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if entry.file_type().ok()?.is_dir() {
                    entry.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        Ok(ids)
    }

    /// Get a `FileBasedBackend` for a specific team.
    pub fn file_backend(&self, team_id: &str) -> FileBasedBackend {
        FileBasedBackend::new(self.teams_root.join(team_id))
    }
}

// ---------------------------------------------------------------------------
// TeamError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum TeamError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(serde_json::Error),
    #[error("Team not found: {0}")]
    NotFound(String),
}

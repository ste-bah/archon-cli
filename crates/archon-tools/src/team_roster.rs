//! The live team roster: who is actually on the team right now (#184 M5).
//!
//! `TeamCreate` used to write a `team.json` that nothing read and inbox files
//! nothing drained — an honest-looking stub in the sense of #153, because the
//! tool reported success and no coordination layer existed behind it. This is
//! the layer.
//!
//! ONE PATH, ONE WRITER
//!
//! The roster lives at `<project>/.archon/teams/<team-id>/team.json`. It had two
//! paths before this: the tools wrote `.archon/teams/` and `TeamManager` read
//! `teams/`, so `archon team list` could never see a team `TeamCreate` made.
//! Everything now goes through this module, including `TeamManager`.
//!
//! SEATS, NOT NAMES
//!
//! A spawn has no instance name — the `Agent` tool takes `subagent_type`, and
//! that is what the router's name registry resolves. So the roster is a list of
//! seats: a role, and the id of the agent filling it. Two concurrent spawns of
//! the same role take two seats, which a role-keyed map could not represent.
//!
//! The active team is process-global because the session is one process and one
//! lead establishes it. The same reasoning as `write_claims`.

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, RwLock};

use crate::team_config::{MemberConfig, TeamConfig};

/// The team this session established, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTeam {
    pub team_id: String,
    pub project_dir: PathBuf,
}

static ACTIVE_TEAM: LazyLock<RwLock<Option<ActiveTeam>>> = LazyLock::new(|| RwLock::new(None));

/// Serializes read-modify-write of `team.json`.
///
/// Joins and leaves arrive from independent agent tasks, and each is a load,
/// mutate, store. Without this two simultaneous joins lose one of the seats.
static ROSTER_WRITE: Mutex<()> = Mutex::new(());

/// Where every team on `project_dir` lives.
pub fn teams_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(".archon").join("teams")
}

/// Where one team lives.
pub fn team_dir(project_dir: &Path, team_id: &str) -> PathBuf {
    teams_dir(project_dir).join(team_id)
}

fn config_path(project_dir: &Path, team_id: &str) -> PathBuf {
    team_dir(project_dir, team_id).join("team.json")
}

/// Make `team_id` the session's team, so named spawns join it.
pub fn activate(project_dir: PathBuf, team_id: String) {
    if let Ok(mut slot) = ACTIVE_TEAM.write() {
        *slot = Some(ActiveTeam {
            team_id,
            project_dir,
        });
    }
}

/// Forget the session's team. Spawns stop joining anything.
pub fn deactivate() {
    if let Ok(mut slot) = ACTIVE_TEAM.write() {
        *slot = None;
    }
}

/// The session's team, if one was established.
pub fn active() -> Option<ActiveTeam> {
    ACTIVE_TEAM.read().ok().and_then(|slot| slot.clone())
}

/// Read a team's roster from disk.
pub fn load(project_dir: &Path, team_id: &str) -> Result<TeamConfig, String> {
    let path = config_path(project_dir, team_id);
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&json).map_err(|e| format!("cannot parse {}: {e}", path.display()))
}

/// Write a team's roster to disk, creating its directory if needed.
pub fn save(project_dir: &Path, config: &TeamConfig) -> Result<(), String> {
    let dir = team_dir(project_dir, &config.id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("cannot serialize team '{}': {e}", config.id))?;
    std::fs::write(dir.join("team.json"), json)
        .map_err(|e| format!("cannot write {}: {e}", dir.join("team.json").display()))
}

/// Every team id on disk under `project_dir`.
pub fn list(project_dir: &Path) -> Vec<String> {
    let root = teams_dir(project_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry
                .file_type()
                .ok()?
                .is_dir()
                .then(|| entry.file_name().into_string().ok())?
        })
        .collect();
    ids.sort();
    ids
}

/// The active team's roster, or an empty one when there is no team.
pub fn members() -> Vec<MemberConfig> {
    let Some(team) = active() else {
        return Vec::new();
    };
    load(&team.project_dir, &team.team_id)
        .map(|config| config.members)
        .unwrap_or_default()
}

/// Seat `agent_id` on the active team under `role`.
///
/// Prefers a declared-but-vacant seat of that role, so an agent spawned to fill
/// a role the team was created with inherits that seat's prompt and tools rather
/// than appearing as a stranger beside it. Falls back to appending a seat: a
/// team can gain a member it did not declare, and pretending otherwise would
/// mean a running agent absent from the roster it belongs to.
///
/// Returns the team id it joined, or `None` when no team is active — which is
/// the ordinary case and not an error.
pub fn join(agent_id: &str, role: &str) -> Option<String> {
    let team = active()?;
    let _guard = ROSTER_WRITE.lock().ok()?;

    let mut config = load(&team.project_dir, &team.team_id).ok()?;

    // Already seated — a resume re-registers under the same id, and seating it
    // twice would show one agent as two members.
    if config
        .members
        .iter()
        .any(|m| m.agent_id.as_deref() == Some(agent_id))
    {
        return Some(team.team_id);
    }

    match config
        .members
        .iter_mut()
        .find(|m| m.role == role && !m.is_filled())
    {
        Some(seat) => seat.agent_id = Some(agent_id.to_string()),
        None => {
            // Every declared seat of this role is taken, so this is a second
            // agent in a declared role, or an agent in a role nobody declared.
            // Either way the seat is undeclared: nothing asked for it, so
            // nothing keeps it once it empties.
            let mut seat = match config.members.iter().find(|m| m.role == role) {
                Some(template) => MemberConfig {
                    declared: false,
                    agent_id: None,
                    ..template.clone()
                },
                None => MemberConfig::undeclared(role),
            };
            seat.agent_id = Some(agent_id.to_string());
            config.members.push(seat);
        }
    }

    if let Err(error) = save(&team.project_dir, &config) {
        tracing::warn!(agent_id, role, %error, "could not record the roster join");
        return None;
    }
    Some(team.team_id)
}

/// Vacate whatever seat `agent_id` held.
///
/// A seat the team declared stays on the roster, vacant — the team still wants
/// a reviewer even when no reviewer is running. A seat that was appended by a
/// join is removed, because nothing declared it.
pub fn leave(agent_id: &str) {
    let Some(team) = active() else {
        return;
    };
    let Ok(_guard) = ROSTER_WRITE.lock() else {
        return;
    };
    let Ok(mut config) = load(&team.project_dir, &team.team_id) else {
        return;
    };

    let mut changed = false;
    for seat in &mut config.members {
        if seat.agent_id.as_deref() == Some(agent_id) {
            seat.agent_id = None;
            changed = true;
        }
    }
    if !changed {
        return;
    }

    // Drop the seats nothing asked for.
    config
        .members
        .retain(|seat| seat.declared || seat.is_filled());

    if let Err(error) = save(&team.project_dir, &config) {
        tracing::warn!(agent_id, %error, "could not record the roster leave");
    }
}

/// The agent ids currently seated on the active team.
pub fn seated_agent_ids() -> Vec<String> {
    members()
        .into_iter()
        .filter_map(|member| member.agent_id)
        .collect()
}

/// Serializes the team test suites against the process-global active team.
///
/// Shared rather than per-file: `team_create` and `team_delete` activate teams
/// too, so a lock private to one suite would not stop the other from swapping
/// the team out mid-test.
#[cfg(test)]
pub(crate) mod test_lock {
    use std::sync::{Mutex, MutexGuard};

    static TEAM_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Recovers from poisoning: a failing test leaves the lock poisoned, and
    /// the rest of the suite failing for that reason hides the real failure.
    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        TEAM_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
#[path = "team_roster_tests.rs"]
mod tests;

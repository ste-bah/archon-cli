//! The team roster section of `/agent` (#184 M5).
//!
//! Split out of `agent_slash.rs` to keep both under the 500-line FileSizeGuard
//! threshold. The definitions listing lives there; who is actually running lives
//! here.

use archon_tools::team_roster;

use super::{COL_NAME, truncate_chars};

/// How wide the status column renders.
const COL_STATUS: usize = 10;

/// The session's team, if one is active.
///
/// Everything here is read synchronously — the roster from disk, liveness from
/// the background-agent registry, claims and tasks from their process-global
/// managers — because `CommandHandler::execute` is sync, and blocking the
/// dispatch path on an async lock would stall the TUI.
///
/// Empty when no team is active, which is most of the time.
pub(super) fn render_roster() -> String {
    let Some(team) = team_roster::active() else {
        return String::new();
    };
    let Ok(config) = team_roster::load(&team.project_dir, &team.team_id) else {
        return format!("\nTeam '{}': roster unreadable.\n", team.team_id);
    };

    let claims = archon_tools::write_claims::live_claims();
    let tasks = archon_tools::task_manager::TASK_MANAGER.list_tasks();

    let mut out = format!(
        "\nTeam '{}' ({}) — {} of {} seat(s) filled\n",
        config.name,
        team.team_id,
        config.members.iter().filter(|m| m.is_filled()).count(),
        config.members.len(),
    );

    for member in &config.members {
        let Some(agent_id) = member.agent_id.as_deref() else {
            out.push_str(&format!(
                "  {:<name$}  vacant\n",
                member.role,
                name = COL_NAME
            ));
            continue;
        };

        // Absent from the liveness registry means it went terminal between the
        // roster read and here, not that it is healthy.
        let status = match archon_tools::background_agents::BACKGROUND_AGENTS.status_of(agent_id) {
            Some(status) => format!("{status:?}").to_lowercase(),
            None => "finishing".to_string(),
        };

        out.push_str(&format!(
            "  {:<name$}  {:<status$}  {}\n",
            member.role,
            status,
            agent_id,
            name = COL_NAME,
            status = COL_STATUS,
        ));

        for task in tasks
            .iter()
            .filter(|t| t.agent_id.as_deref() == Some(agent_id))
        {
            out.push_str(&indented(&format!(
                "task: {}",
                truncate_chars(&task.description, 48)
            )));
        }
        for claim in claims
            .iter()
            .filter(|c| c.agent_id == agent_id && !c.declared.is_empty())
        {
            out.push_str(&indented(&format!(
                "writing: {}",
                truncate_chars(&claim.declared.join(", "), 48)
            )));
        }
    }

    out.push_str("\nTip: members address each other by role with SendMessage.\n");
    out
}

/// A continuation line under a member, aligned past the role column.
fn indented(text: &str) -> String {
    format!("  {:<name$}    {text}\n", "", name = COL_NAME)
}

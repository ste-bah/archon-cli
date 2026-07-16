//! Enhanced session listing and directory-based lookup.

use crate::storage::{SessionError, SessionMetadata, SessionStore};

/// Format a list of sessions into a human-readable table string.
///
/// Includes: short ID, name (if set), date, working directory, turns, cost.
pub fn format_session_list(sessions: &[SessionMetadata]) -> String {
    if sessions.is_empty() {
        return String::from("No sessions found.");
    }

    let mut lines = Vec::with_capacity(sessions.len() + 1);
    lines.push(format!(
        "{:<10} {:<20} {:<20} {:<30} {:>6} {:>10}",
        "ID", "NAME", "LAST ACTIVE", "DIRECTORY", "TURNS", "COST"
    ));

    for s in sessions {
        let id_short: String = s.id.chars().take(8).collect();
        let name_display = s.name.as_deref().unwrap_or("-");
        // Truncate name to 18 chars for column alignment
        let name_truncated = if name_display.chars().count() > 18 {
            format!("{}...", name_display.chars().take(15).collect::<String>())
        } else {
            name_display.to_string()
        };
        // Show just the date portion from RFC 3339
        let date: String = s.last_active.chars().take(10).collect();
        // Truncate directory
        let directory_chars = s.working_directory.chars().count();
        let dir = if directory_chars > 28 {
            format!(
                "...{}",
                s.working_directory
                    .chars()
                    .skip(directory_chars - 25)
                    .collect::<String>()
            )
        } else {
            s.working_directory.clone()
        };
        let cost_str = format!("${:.2}", s.total_cost);

        lines.push(format!(
            "{:<10} {:<20} {:<20} {:<30} {:>6} {:>10}",
            id_short, name_truncated, date, dir, s.message_count, cost_str
        ));
    }

    lines.join("\n")
}

/// Find the most recently active session in a specific working directory.
///
/// Returns `None` if no sessions exist for that directory.
pub fn most_recent_in_directory(
    store: &SessionStore,
    directory: &str,
) -> Result<Option<SessionMetadata>, SessionError> {
    // Load a generous number of sessions and filter by directory.
    // The store returns them sorted by last_active descending, so the
    // first match is the most recent.
    let sessions = store.list_sessions(1000)?;

    for s in sessions {
        if s.working_directory == directory {
            return Ok(Some(s));
        }
    }

    Ok(None)
}

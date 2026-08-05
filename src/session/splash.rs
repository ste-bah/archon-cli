use std::path::Path;

pub(super) fn splash_config(
    bare_mode: bool,
    active_model: &str,
    working_dir: &Path,
    session_store: &archon_session::storage::SessionStore,
    session_id: &str,
    garden_summary: Option<String>,
) -> Option<archon_tui::app::SplashConfig> {
    if bare_mode {
        return None;
    }
    Some(archon_tui::app::SplashConfig {
        model: active_model.to_string(),
        working_dir: working_dir.display().to_string(),
        activity: with_garden_summary(
            recent_activity(working_dir, session_store, session_id),
            garden_summary,
        ),
    })
}

/// Put what automatic consolidation just did at the top of the activity panel.
///
/// First, because it describes something that happened to your memories moments
/// ago. It goes here rather than the output buffer because the splash is drawn
/// INSTEAD of that buffer at startup -- an emission there is real, queued, and
/// invisible until the splash clears, which defeats the point of reporting it.
fn with_garden_summary(
    mut activity: Vec<archon_tui::splash::ActivityEntry>,
    garden_summary: Option<String>,
) -> Vec<archon_tui::splash::ActivityEntry> {
    if let Some(summary) = garden_summary {
        activity.insert(
            0,
            archon_tui::splash::ActivityEntry {
                when: "just now".to_string(),
                description: format!("Memory garden: {summary}"),
            },
        );
    }
    activity
}

#[cfg(test)]
mod tests {
    use archon_tui::splash::ActivityEntry;

    fn entry(when: &str) -> ActivityEntry {
        ActivityEntry {
            when: when.to_string(),
            description: "Empty session".to_string(),
        }
    }

    #[test]
    fn a_summary_is_shown_first() {
        let out = super::with_garden_summary(
            vec![entry("2m ago"), entry("5m ago")],
            Some("2 duplicate(s) merged".to_string()),
        );

        assert_eq!(out.len(), 3, "existing activity is kept");
        assert_eq!(out[0].when, "just now");
        assert_eq!(out[0].description, "Memory garden: 2 duplicate(s) merged");
        assert_eq!(out[1].when, "2m ago", "prior entries keep their order");
    }

    #[test]
    fn no_summary_leaves_the_panel_untouched() {
        let out = super::with_garden_summary(vec![entry("2m ago")], None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].when, "2m ago");
    }

    /// The panel is drawn even with no prior session history, so a summary must
    /// not depend on other entries existing.
    #[test]
    fn a_summary_shows_on_an_empty_panel() {
        let out = super::with_garden_summary(Vec::new(), Some("1 stale pruned".to_string()));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].description, "Memory garden: 1 stale pruned");
    }
}

fn recent_activity(
    working_dir: &Path,
    session_store: &archon_session::storage::SessionStore,
    session_id: &str,
) -> Vec<archon_tui::splash::ActivityEntry> {
    let cwd = working_dir.display().to_string();
    session_store
        .list_sessions(10)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.working_directory == cwd)
        .filter(|s| s.id != session_id)
        .take(3)
        .map(|s| {
            let when = archon_tui::splash::format_relative_time(&s.last_active);
            let msgs = s.message_count;
            let description = if msgs == 0 {
                "Empty session".to_string()
            } else {
                format!("{msgs} messages, {}", s.model)
            };
            archon_tui::splash::ActivityEntry { when, description }
        })
        .collect()
}

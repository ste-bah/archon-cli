use std::path::Path;

pub(super) fn splash_config(
    bare_mode: bool,
    active_model: &str,
    working_dir: &Path,
    session_store: &archon_session::storage::SessionStore,
    session_id: &str,
) -> Option<archon_tui::app::SplashConfig> {
    if bare_mode {
        return None;
    }
    Some(archon_tui::app::SplashConfig {
        model: active_model.to_string(),
        working_dir: working_dir.display().to_string(),
        activity: recent_activity(working_dir, session_store, session_id),
    })
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

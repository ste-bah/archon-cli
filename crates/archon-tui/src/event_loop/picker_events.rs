//! Construction of the `/model` and `/theme` picker overlays (#192).
//!
//! Split from `event_loop/tui_events.rs` for the 500-line ceiling, the same
//! reason `picker_input.rs` was split from `input.rs`.
//!
//! Both are opened by a slash command that also prints its usual text, so a
//! print-mode run drops the event and keeps exactly the output it had.

use crate::app::App;

/// Open the message-selector overlay (TASK-TUI-620, `/rewind`).
///
/// Moved here with the other overlay constructors when `tui_events.rs` hit the
/// 500-line ceiling.
pub(crate) fn open_message_selector(app: &mut App, messages: Vec<crate::app::MessageSummary>) {
    app.message_selector = Some(crate::screens::message_selector::MessageSelector::new(
        messages,
    ));
}

/// Open the skills-menu overlay (TASK-TUI-627, `/skills`).
pub(crate) fn open_skills_menu(app: &mut App, skills: Vec<crate::app::SkillEntry>) {
    app.skills_menu = Some(crate::screens::skills_menu::SkillsMenu::new(skills));
}

/// Populate and open the model picker.
///
/// Entries arrive as `(provider_id, model_id, label)` already resolved at the
/// dispatch site, because the command handler is sync and the model config
/// lives behind an async lock.
pub(crate) fn open_model_picker(app: &mut App, entries: Vec<(String, String, String)>) {
    let mut picker = crate::screens::model_picker::ModelPicker::new();
    picker.set_providers(
        entries
            .into_iter()
            .map(
                |(provider_id, model_id, label)| crate::screens::model_picker::ProviderEntry {
                    provider_id,
                    model_id,
                    label,
                },
            )
            .collect(),
    );
    app.model_picker = Some(picker);
}

/// Populate and open the theme picker, marking the applied theme.
///
/// The dispatch site sends every available name with no active flag: which
/// theme is applied lives on the `App`, and a sync command handler cannot see
/// it. Filling that in here is what lets the picker answer "which one am I
/// on", which is the question it exists for.
pub(crate) fn open_theme_picker(app: &mut App, entries: Vec<(String, bool)>) {
    let mut screen = crate::screens::theme_screen::ThemeScreen::new();
    screen.set_themes(
        entries
            .into_iter()
            .map(|(name, _)| crate::screens::theme_screen::ThemeEntry {
                is_active: name == app.theme_name,
                name,
            })
            .collect(),
    );
    // Park the cursor on the applied theme rather than the top of the list.
    screen.select_theme(&app.theme_name);
    app.theme_screen = Some(screen);
}

/// Populate and open the settings overlay (`/config` with no arguments).
pub(crate) fn open_settings(app: &mut App, entries: Vec<(String, String, bool, bool)>) {
    let mut screen = crate::screens::settings_screen::SettingsScreen::new();
    screen.set_fields(
        entries
            .into_iter()
            .map(
                |(key, value, is_bool, read_only)| crate::screens::settings_screen::SettingField {
                    key,
                    value,
                    is_bool,
                    read_only,
                },
            )
            .collect(),
    );
    app.settings_screen = Some(screen);
}

/// Populate and open the hooks overlay (`/hooks` with no subcommand).
pub(crate) fn open_hooks(app: &mut App, entries: Vec<(String, String, String, String, bool)>) {
    let mut menu = crate::screens::hooks_config_menu::HooksMenu::new();
    menu.set_hooks(
        entries
            .into_iter()
            .map(|(id, event, command, source, enabled)| {
                crate::screens::hooks_config_menu::HookRow {
                    id,
                    event,
                    command,
                    source,
                    enabled,
                }
            })
            .collect(),
    );
    app.hooks_menu = Some(menu);
}

/// Populate and open the permission-rules overlay (`/permissions`).
///
/// An unrecognised effect string is dropped rather than guessed at: showing a
/// rule under the wrong effect is worse than not showing it, because the three
/// effects are exactly what the reader is there to check.
pub(crate) fn open_permissions(app: &mut App, mode: String, rules: Vec<(String, String, String)>) {
    use crate::screens::permissions_browser::{PermissionsBrowser, RuleEffect, ToolPermission};

    let mut browser = PermissionsBrowser::new(mode);
    browser.set_permissions(
        rules
            .into_iter()
            .filter_map(|(effect, tool, pattern)| {
                let effect = match effect.as_str() {
                    "deny" => RuleEffect::Deny,
                    "allow" => RuleEffect::Allow,
                    "ask" => RuleEffect::Ask,
                    _ => return None,
                };
                Some(ToolPermission {
                    effect,
                    tool,
                    pattern,
                })
            })
            .collect(),
    );
    app.permissions_browser = Some(browser);
}

/// Populate and open the memory-files overlay (`/memory files`).
pub(crate) fn open_memory_files(app: &mut App, entries: Vec<(String, String, u64)>) {
    let mut browser = crate::screens::memory_file_selector::MemoryBrowser::new();
    browser.set_entries(
        entries
            .into_iter()
            .map(
                |(scope, path, size_bytes)| crate::screens::memory_file_selector::MemoryEntry {
                    path,
                    size_bytes,
                    scope,
                },
            )
            .collect(),
    );
    app.memory_browser = Some(browser);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_picker_opens_with_the_rows_it_was_given() {
        let mut app = App::default();
        open_model_picker(
            &mut app,
            vec![(
                "anthropic".to_string(),
                "claude-opus-5".to_string(),
                "opus".to_string(),
            )],
        );
        let picker = app.model_picker.as_ref().expect("opened");
        assert_eq!(picker.len(), 1);
        assert_eq!(
            picker.selected().map(|entry| entry.model_id.as_str()),
            Some("claude-opus-5")
        );
    }

    #[test]
    fn the_theme_picker_marks_and_selects_the_applied_theme() {
        let mut app = App::default();
        app.theme_name = "ocean".to_string();
        open_theme_picker(
            &mut app,
            vec![
                ("intj".to_string(), false),
                ("ocean".to_string(), false),
                ("fire".to_string(), false),
            ],
        );

        let screen = app.theme_screen.as_ref().expect("opened");
        let selected = screen.selected().expect("a row is selected");
        assert_eq!(
            selected.name, "ocean",
            "the cursor must start on the applied theme"
        );
        assert!(
            selected.is_active,
            "the applied theme must be marked, whatever the dispatch site sent"
        );
    }
}

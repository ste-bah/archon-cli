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

/// Populate and open the branch picker (`/branch` with no arguments).
pub(crate) fn open_branch_picker(app: &mut App, entries: Vec<(usize, String, String)>) {
    let mut picker = crate::screens::session_branching::BranchPicker::new();
    picker.set_candidates(
        entries
            .into_iter()
            .map(
                |(index, role, summary)| crate::screens::session_branching::MessageRef {
                    index,
                    role,
                    summary,
                },
            )
            .collect(),
    );
    app.branch_picker = Some(picker);
}

/// Open the token attribution overlay (`/context` with no arguments).
///
/// Joins the two halves: the ranking the agent measured, already on the `App`
/// from the last `ContextPressureUpdated`, and the message text `/context` read
/// out of the session log. A ranked message the log has no line for keeps its
/// index and its cost — the number is the actionable part, and dropping the row
/// because its label is missing would hide the very thing being ranked.
pub(crate) fn open_token_attribution(app: &mut App, previews: Vec<(usize, String, String)>) {
    use crate::screens::token_attribution::{Contributor, TokenAttributionOverlay};

    let attribution = app.status.token_attribution.clone();
    let mut overlay = TokenAttributionOverlay::new(attribution.total);
    overlay.set_contributors(
        attribution
            .contributors
            .iter()
            .map(|&(message_index, tokens)| {
                let preview = previews
                    .iter()
                    .find(|(index, _, _)| *index == message_index);
                Contributor {
                    message_index,
                    tokens,
                    share_percent: attribution.share_percent(tokens),
                    role: preview.map(|(_, role, _)| role.clone()).unwrap_or_default(),
                    summary: preview
                        .map(|(_, _, summary)| summary.clone())
                        .unwrap_or_default(),
                }
            })
            .collect(),
    );
    app.token_attribution = Some(overlay);
}

/// Open the voice capture overlay (`/voice` with no arguments).
///
/// Opening it does not start a recording — `/voice` is how you look at the
/// meter and the last transcription; the hotkey is how you record.
pub(crate) fn open_voice_capture(app: &mut App, vad_threshold: f32) {
    app.voice_capture =
        Some(crate::screens::voice_capture::VoiceCaptureOverlay::with_threshold(vad_threshold));
}

/// A recording started or ended.
///
/// A start opens the overlay if it is not already open: the user pressed the
/// record hotkey, and a recording with no visible indicator is how you end up
/// talking to a microphone that is not listening.
pub(crate) fn set_voice_recording(app: &mut App, recording: bool) {
    match app.voice_capture.as_mut() {
        Some(overlay) if recording => overlay.start(),
        Some(overlay) => overlay.stop(),
        None if recording => {
            let mut overlay = crate::screens::voice_capture::VoiceCaptureOverlay::new();
            overlay.start();
            app.voice_capture = Some(overlay);
        }
        // Nothing open and nothing recording: nothing to show.
        None => {}
    }
}

/// One level reading from the capture thread.
///
/// Dropped when the overlay is closed rather than buffered — the meter is a
/// live view, and levels from a recording nobody is watching have no reader.
pub(crate) fn push_voice_level(app: &mut App, level: f32) {
    if let Some(overlay) = app.voice_capture.as_mut() {
        overlay.push_sample(level);
    }
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

    /// The overlay has to appear on its own: the user pressed a hotkey, and a
    /// recording with no visible indicator is how you end up talking to a
    /// microphone that is not listening.
    #[test]
    fn a_recording_opens_the_overlay_when_none_is_open() {
        let mut app = App::default();
        set_voice_recording(&mut app, true);
        assert!(
            app.voice_capture
                .as_ref()
                .expect("the recording opened no overlay")
                .is_recording()
        );
    }

    #[test]
    fn ending_a_recording_leaves_the_overlay_open_but_stopped() {
        let mut app = App::default();
        set_voice_recording(&mut app, true);
        set_voice_recording(&mut app, false);
        let overlay = app.voice_capture.as_ref().expect("the overlay closed");
        assert!(!overlay.is_recording());
    }

    /// Otherwise every recording that ends would pop a window open.
    #[test]
    fn the_end_of_a_recording_opens_nothing() {
        let mut app = App::default();
        set_voice_recording(&mut app, false);
        assert!(app.voice_capture.is_none());
    }

    #[test]
    fn levels_reach_the_meter_while_the_overlay_is_open() {
        let mut app = App::default();
        open_voice_capture(&mut app, 0.05);
        push_voice_level(&mut app, 0.3);
        push_voice_level(&mut app, 0.1);
        let overlay = app.voice_capture.as_ref().expect("opened");
        assert_eq!(overlay.waveform_slice(), vec![0.3, 0.1]);
        assert!((overlay.vad_threshold() - 0.05).abs() < 1e-6);
    }

    /// A closed overlay has no reader; buffering levels for it would only grow
    /// a queue nobody drains.
    #[test]
    fn levels_are_dropped_when_the_overlay_is_closed() {
        let mut app = App::default();
        push_voice_level(&mut app, 0.3);
        assert!(app.voice_capture.is_none());
    }

    /// `/voice` is for looking at the meter, not for recording — the hotkey
    /// does that, and conflating them would start a recording every time
    /// someone read the configuration.
    #[test]
    fn opening_the_overlay_does_not_start_a_recording() {
        let mut app = App::default();
        open_voice_capture(&mut app, 0.02);
        assert!(!app.voice_capture.as_ref().expect("opened").is_recording());
    }

    fn app_with_ranking() -> App {
        let mut app = App::default();
        app.status.token_attribution = crate::status::TokenAttribution {
            contributors: vec![(12, 42_000), (3, 8_000)],
            total: 100_000,
        };
        app
    }

    /// The join this overlay exists to perform: the agent knows the cost, the
    /// session log knows the text, and neither can produce the other half.
    #[test]
    fn the_ranking_and_the_message_text_are_joined_by_index() {
        let mut app = app_with_ranking();
        open_token_attribution(
            &mut app,
            vec![
                (3, "user".into(), "pasted the config".into()),
                (12, "assistant".into(), "the enormous build log".into()),
            ],
        );

        let overlay = app.token_attribution.as_ref().expect("opened");
        assert_eq!(overlay.len(), 2);
        let first = overlay.selected().expect("a row is selected");
        assert_eq!(first.message_index, 12, "the ranking order must survive");
        assert_eq!(first.summary, "the enormous build log");
        assert!((first.share_percent - 42.0).abs() < 1e-6);
    }

    /// A ranked message the log has no line for keeps its index and its cost:
    /// the number is the actionable part.
    #[test]
    fn a_contributor_with_no_preview_still_appears() {
        let mut app = app_with_ranking();
        open_token_attribution(&mut app, Vec::new());

        let overlay = app.token_attribution.as_ref().expect("opened");
        assert_eq!(overlay.len(), 2);
        let first = overlay.selected().expect("a row is selected");
        assert_eq!(first.tokens, 42_000);
        assert!(first.summary.is_empty());
        assert!(first.role.is_empty());
    }

    /// Before the first request there is nothing measured, and the overlay has
    /// to say so rather than draw an empty box.
    #[test]
    fn nothing_measured_yet_opens_an_empty_overlay_rather_than_none() {
        let mut app = App::default();
        open_token_attribution(&mut app, Vec::new());
        let overlay = app.token_attribution.as_ref().expect("opened");
        assert!(overlay.is_empty());
        assert_eq!(overlay.total(), 0);
    }
}

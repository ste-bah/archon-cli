//! Key-routing assertions for the overlays added by #192.
//!
//! Split from `input_routing_tests.rs` when that file reached the 500-line
//! ceiling. Same contract as its parent: these assert that a keystroke
//! actually reaches a given overlay through the `if app.<overlay>.is_some()`
//! chain in `input.rs`, which no screen-level test can tell you.

use super::input::handle_key_event;
use crate::app::App;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn channels() -> (
    tokio::sync::mpsc::Sender<String>,
    crate::keybindings::KeyMap,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    // Held open so `send` in a routed branch does not fail the test for the
    // wrong reason.
    std::mem::forget(rx);
    (tx, crate::keybindings::KeyMap::default())
}

#[tokio::test]
async fn down_reaches_the_permissions_overlay() {
    use crate::screens::permissions_browser::{PermissionsBrowser, RuleEffect, ToolPermission};

    let mut app = App::default();
    let mut browser = PermissionsBrowser::new("default".into());
    browser.set_permissions(vec![
        ToolPermission {
            effect: RuleEffect::Deny,
            tool: "Bash".into(),
            pattern: "rm:*".into(),
        },
        ToolPermission {
            effect: RuleEffect::Allow,
            tool: "Read".into(),
            pattern: "*".into(),
        },
    ]);
    app.permissions_browser = Some(browser);
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Down),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert_eq!(
        app.permissions_browser
            .as_ref()
            .expect("still open")
            .selected_index(),
        1,
        "Down did not reach the permissions overlay"
    );
}

/// Typing filters the memory-file list rather than landing in the prompt.
#[tokio::test]
async fn typing_filters_the_memory_files_overlay() {
    use crate::screens::memory_file_selector::{MemoryBrowser, MemoryEntry};

    let mut app = App::default();
    let mut browser = MemoryBrowser::new();
    browser.set_entries(vec![
        MemoryEntry {
            path: "/home/me/.archon/ARCHON.md".into(),
            size_bytes: 10,
            scope: "global".into(),
        },
        MemoryEntry {
            path: "/work/repo/ARCHON.md".into(),
            size_bytes: 20,
            scope: "project".into(),
        },
    ]);
    app.memory_browser = Some(browser);
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Char('r')),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    let browser = app.memory_browser.as_ref().expect("still open");
    assert_eq!(browser.query(), "r");
    assert_eq!(browser.len(), 2, "both paths contain an r");
    assert!(
        app.input.text().is_empty(),
        "the keystroke leaked into the prompt behind the overlay"
    );
}

fn hooks_with_two() -> crate::screens::hooks_config_menu::HooksMenu {
    let mut menu = crate::screens::hooks_config_menu::HooksMenu::new();
    menu.set_hooks(vec![
        crate::screens::hooks_config_menu::HookRow {
            id: "abc123".into(),
            event: "PreToolUse".into(),
            command: "bash scripts/self-check-file.sh Edit".into(),
            source: "project".into(),
            enabled: true,
        },
        crate::screens::hooks_config_menu::HookRow {
            id: "def456".into(),
            event: "PostToolUse".into(),
            command: "bash scripts/self-check-file.sh Write".into(),
            source: "user".into(),
            enabled: false,
        },
    ]);
    menu
}

#[tokio::test]
async fn down_reaches_the_hooks_overlay() {
    let mut app = App::default();
    app.hooks_menu = Some(hooks_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Down),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert_eq!(
        app.hooks_menu
            .as_ref()
            .expect("still open")
            .selected_index(),
        1,
        "Down did not reach the hooks overlay"
    );
}

/// A disabled hook offers `enable`, an enabled one offers `disable`, and
/// either way it goes through the command that writes the override file.
#[tokio::test]
async fn enter_injects_the_opposite_hooks_command() {
    let mut app = App::default();
    app.hooks_menu = Some(hooks_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Enter),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;
    assert_eq!(app.input.text(), "/hooks disable abc123");

    app.input.set_text("");
    app.hooks_menu = Some(hooks_with_two());
    handle_key_event(
        &mut app,
        press(KeyCode::Down),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;
    handle_key_event(
        &mut app,
        press(KeyCode::Enter),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;
    assert_eq!(app.input.text(), "/hooks enable def456");
}

fn settings_with_two() -> crate::screens::settings_screen::SettingsScreen {
    let mut screen = crate::screens::settings_screen::SettingsScreen::new();
    screen.set_fields(vec![
        crate::screens::settings_screen::SettingField {
            key: "api.default_effort".into(),
            value: "high".into(),
            is_bool: false,
            read_only: false,
        },
        crate::screens::settings_screen::SettingField {
            key: "tools.cargo.incremental".into(),
            value: "false".into(),
            is_bool: true,
            read_only: false,
        },
    ]);
    screen
}

#[tokio::test]
async fn down_reaches_the_settings_overlay() {
    let mut app = App::default();
    app.settings_screen = Some(settings_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Down),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert_eq!(
        app.settings_screen
            .as_ref()
            .expect("still open")
            .selected_index(),
        1,
        "Down did not reach the settings overlay"
    );
}

/// The whole reason `toggle_selected` was removed: pressing Enter has to put
/// the change through `/config`, which is what actually validates and applies
/// it. A boolean arrives flipped so the user is not made to type the only
/// other value it could have.
#[tokio::test]
async fn enter_injects_the_config_command_for_the_selected_key() {
    let mut app = App::default();
    app.settings_screen = Some(settings_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Down),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;
    handle_key_event(
        &mut app,
        press(KeyCode::Enter),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert_eq!(app.input.text(), "/config tools.cargo.incremental true");
    assert!(
        app.settings_screen.is_none(),
        "the overlay must close once it has handed over the command"
    );
}

/// Anything not acted on must be swallowed, or it types into the prompt
/// hidden behind the overlay.
#[tokio::test]
async fn printable_characters_do_not_reach_the_prompt_behind_the_settings_overlay() {
    let mut app = App::default();
    app.settings_screen = Some(settings_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Char('q')),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert!(app.input.text().is_empty(), "typed behind the overlay");
    assert!(app.settings_screen.is_some(), "an unhandled key closed it");
}

// ---------------------------------------------------------------------------
// Voice capture overlay (#192)
// ---------------------------------------------------------------------------

/// Esc must reach the overlay, or a live microphone keeps recording behind a
/// window the user thinks they closed.
#[tokio::test]
async fn esc_reaches_the_voice_overlay_and_closes_it() {
    let mut app = App::default();
    app.voice_capture = Some(crate::screens::voice_capture::VoiceCaptureOverlay::new());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Esc),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert!(app.voice_capture.is_none(), "Esc did not reach the overlay");
}

#[tokio::test]
async fn enter_closes_the_voice_overlay() {
    let mut app = App::default();
    app.voice_capture = Some(crate::screens::voice_capture::VoiceCaptureOverlay::new());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Enter),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert!(app.voice_capture.is_none());
    assert!(
        app.input.text().is_empty(),
        "Enter must not submit the prompt behind the overlay"
    );
}

/// Same rule as every other overlay: unhandled keys are swallowed rather than
/// typed into the prompt behind it.
#[tokio::test]
async fn printable_characters_do_not_reach_the_prompt_behind_the_voice_overlay() {
    let mut app = App::default();
    app.voice_capture = Some(crate::screens::voice_capture::VoiceCaptureOverlay::new());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Char('q')),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert!(app.input.text().is_empty(), "typed behind the overlay");
    assert!(app.voice_capture.is_some(), "an unhandled key closed it");
}

// ---------------------------------------------------------------------------
// Token attribution overlay (#192 scope B)
// ---------------------------------------------------------------------------

fn attribution_with_two() -> crate::screens::token_attribution::TokenAttributionOverlay {
    use crate::screens::token_attribution::{Contributor, TokenAttributionOverlay};

    let mut overlay = TokenAttributionOverlay::new(100_000);
    overlay.set_contributors(vec![
        Contributor {
            message_index: 12,
            tokens: 42_000,
            share_percent: 42.0,
            role: "assistant".into(),
            summary: "the enormous build log".into(),
        },
        Contributor {
            message_index: 3,
            tokens: 8_000,
            share_percent: 8.0,
            role: "user".into(),
            summary: "pasted the config".into(),
        },
    ]);
    overlay
}

#[tokio::test]
async fn down_reaches_the_attribution_overlay() {
    let mut app = App::default();
    app.token_attribution = Some(attribution_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Down),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert_eq!(
        app.token_attribution
            .as_ref()
            .expect("still open")
            .selected_index(),
        1,
        "Down never reached the overlay"
    );
}

#[tokio::test]
async fn esc_closes_the_attribution_overlay() {
    let mut app = App::default();
    app.token_attribution = Some(attribution_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Esc),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert!(app.token_attribution.is_none());
}

/// It is a read-only ranking. Nothing here drops a message, so nothing here
/// may look like it does.
#[tokio::test]
async fn printable_characters_do_not_reach_the_prompt_behind_the_attribution_overlay() {
    let mut app = App::default();
    app.token_attribution = Some(attribution_with_two());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Char('d')),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert!(app.input.text().is_empty(), "typed behind the overlay");
    assert!(
        app.token_attribution.is_some(),
        "an unhandled key closed it"
    );
}

//! Key-routing assertions for `handle_key_event`.
//!
//! # Why this file exists
//!
//! `input.rs` had no tests. Every overlay's keys are routed by a chain of
//! `if app.<overlay>.is_some()` blocks in one 570-line function, and whether a
//! given key reaches a given overlay depends on the order of that chain and on
//! what each earlier block swallows. Nothing asserted any of it.
//!
//! Screen-level tests cannot cover this. `model_picker`'s own render tests
//! prove that moving the selection changes the drawn frame, and
//! `task_overlay`'s prove the same — both pass whether or not a keystroke ever
//! reaches them. The gap between "the screen works" and "the key gets there"
//! is exactly where an overlay ends up looking like it has dead arrow keys.

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

fn model_picker_with_three() -> crate::screens::model_picker::ModelPicker {
    let mut picker = crate::screens::model_picker::ModelPicker::new();
    picker.set_providers(vec![
        crate::screens::model_picker::ProviderEntry {
            provider_id: "anthropic".into(),
            model_id: "claude-opus-5".into(),
            label: "opus".into(),
        },
        crate::screens::model_picker::ProviderEntry {
            provider_id: "anthropic".into(),
            model_id: "claude-sonnet-5".into(),
            label: "sonnet".into(),
        },
        crate::screens::model_picker::ProviderEntry {
            provider_id: "anthropic".into(),
            model_id: "claude-fable-5".into(),
            label: "fable".into(),
        },
    ]);
    picker
}

#[tokio::test]
async fn down_reaches_the_model_picker() {
    let mut app = App::default();
    app.model_picker = Some(model_picker_with_three());
    let (tx, keymap) = channels();

    assert_eq!(
        app.model_picker.as_ref().expect("open").selected_index(),
        0,
        "precondition"
    );

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
        app.model_picker
            .as_ref()
            .expect("still open")
            .selected_index(),
        1,
        "Down did not reach the model picker — some earlier branch consumed it"
    );
}

#[tokio::test]
async fn up_reaches_the_model_picker() {
    let mut app = App::default();
    app.model_picker = Some(model_picker_with_three());
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
    handle_key_event(&mut app, press(KeyCode::Up), &tx, None, None, None, &keymap).await;

    assert_eq!(
        app.model_picker
            .as_ref()
            .expect("still open")
            .selected_index(),
        0,
        "Up did not reach the model picker"
    );
}

#[tokio::test]
async fn enter_injects_the_slash_command_and_closes_the_picker() {
    let mut app = App::default();
    app.model_picker = Some(model_picker_with_three());
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

    assert!(app.model_picker.is_none(), "Enter left the picker open");
    assert_eq!(
        app.input.text(),
        "/model claude-sonnet-5",
        "Enter must inject the command for the SELECTED row, not the first"
    );
}

#[tokio::test]
async fn escape_closes_the_model_picker() {
    let mut app = App::default();
    app.model_picker = Some(model_picker_with_three());
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
    assert!(app.model_picker.is_none());
}

/// Typing filters rather than falling through to the prompt behind the overlay.
#[tokio::test]
async fn printable_characters_filter_instead_of_typing_behind_the_overlay() {
    let mut app = App::default();
    app.model_picker = Some(model_picker_with_three());
    let (tx, keymap) = channels();

    handle_key_event(
        &mut app,
        press(KeyCode::Char('s')),
        &tx,
        None,
        None,
        None,
        &keymap,
    )
    .await;

    assert_eq!(app.model_picker.as_ref().expect("open").query(), "s");
    assert!(
        app.input.text().is_empty(),
        "the keystroke leaked into the input buffer behind the overlay"
    );
}

#[tokio::test]
async fn down_reaches_the_theme_picker() {
    let mut app = App::default();
    let mut screen = crate::screens::theme_screen::ThemeScreen::new();
    screen.set_themes(vec![
        crate::screens::theme_screen::ThemeEntry {
            name: "intj".into(),
            is_active: true,
        },
        crate::screens::theme_screen::ThemeEntry {
            name: "ocean".into(),
            is_active: false,
        },
    ]);
    app.theme_screen = Some(screen);
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
        app.theme_screen.as_ref().expect("open").selected_index(),
        1,
        "Down did not reach the theme picker"
    );
}

/// The tasks overlay routes through its own module; this pins that it is still
/// reachable from the same chain, since #189 Phase 9 shipped it looking dead.
#[tokio::test]
async fn down_reaches_the_tasks_overlay() {
    use crate::screens::task_overlay::{TaskOverlay, TaskRow};

    let mut app = App::default();
    app.task_overlay = Some(TaskOverlay::new(vec![
        TaskRow {
            id: "a".into(),
            elapsed_secs: 1,
            status: "running".into(),
        },
        TaskRow {
            id: "b".into(),
            elapsed_secs: 2,
            status: "running".into(),
        },
    ]));
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
        app.task_overlay.as_ref().expect("open").selected_index(),
        1,
        "Down did not reach the tasks overlay"
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

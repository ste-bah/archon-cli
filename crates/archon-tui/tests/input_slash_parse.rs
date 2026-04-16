use archon_tui::input::{handle_key, KeyResult};
use archon_tui::app::App;
use archon_tui::keybindings::KeyMap;
use crossterm::event::{KeyEvent, KeyCode, KeyModifiers};

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn ctrl_d_returns_quit() {
    let mut app = App::new();
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert!(matches!(handle_key(&mut app, key, &keymap), KeyResult::Quit));
}

#[test]
fn ctrl_c_not_generating_returns_quit() {
    let mut app = App::new();
    app.is_generating = false;
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(matches!(handle_key(&mut app, key, &keymap), KeyResult::Quit));
}

#[test]
fn ctrl_c_during_generation_returns_send_cancel() {
    let mut app = App::new();
    app.is_generating = true;
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(matches!(handle_key(&mut app, key, &keymap), KeyResult::SendCancel));
}

#[test]
fn enter_submits_text() {
    let mut app = App::new();
    for c in "hello".chars() {
        app.input.insert(c);
    }
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Enter, KeyModifiers::NONE);
    let result = handle_key(&mut app, key, &keymap);
    assert!(matches!(result, KeyResult::SendInput(_)));
}

#[test]
fn tab_accepts_suggestion() {
    let mut app = App::new();
    app.input.suggestions.active = true;
    app.input.suggestions.suggestions.push(archon_tui::commands::CommandInfo {
        name: "test",
        description: "test cmd",
    });
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Tab, KeyModifiers::NONE);
    let result = handle_key(&mut app, key, &keymap);
    assert!(matches!(result, KeyResult::Nothing));
    assert!(!app.input.suggestions.active);
}

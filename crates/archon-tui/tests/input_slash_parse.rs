use archon_tui::app::App;
use archon_tui::input::{KeyResult, handle_key};
use archon_tui::keybindings::KeyMap;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn ctrl_d_returns_quit() {
    let mut app = App::new();
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert!(matches!(
        handle_key(&mut app, key, &keymap),
        KeyResult::Quit
    ));
}

#[test]
fn ctrl_c_not_generating_returns_quit() {
    let mut app = App::new();
    app.is_generating = false;
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(matches!(
        handle_key(&mut app, key, &keymap),
        KeyResult::Quit
    ));
}

#[test]
fn ctrl_c_during_generation_returns_send_cancel() {
    let mut app = App::new();
    app.is_generating = true;
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(matches!(
        handle_key(&mut app, key, &keymap),
        KeyResult::SendCancel
    ));
}

#[test]
fn double_esc_during_generation_returns_send_cancel() {
    // Regression: prior to 2026-05-12 the Esc handler flipped is_generating
    // and printed "[interrupted]" but returned KeyResult::Nothing — the
    // cancel chain never fired and the agent kept running. Double-Esc must
    // emit SendCancel exactly like Ctrl+C.
    let mut app = App::new();
    app.is_generating = true;
    let keymap = KeyMap::default();
    let esc = make_key(KeyCode::Esc, KeyModifiers::NONE);

    // First Esc: dismisses suggestions / records timestamp. No cancel yet.
    let first = handle_key(&mut app, esc, &keymap);
    assert!(matches!(first, KeyResult::Nothing));
    assert!(app.is_generating, "single Esc must not flip is_generating");

    // Second Esc within 500ms: real cancel.
    let second = handle_key(&mut app, esc, &keymap);
    assert!(
        matches!(second, KeyResult::SendCancel),
        "double-Esc during generation must return SendCancel"
    );
    assert!(!app.is_generating, "double-Esc must flip is_generating");
}

#[test]
fn double_esc_when_not_generating_does_not_send_cancel() {
    // Double-Esc when no turn is in flight must NOT emit a cancel signal —
    // there's nothing to cancel, and an unsolicited "__cancel__" message
    // would log a spurious "no in-flight turn to cancel" warning.
    let mut app = App::new();
    app.is_generating = false;
    let keymap = KeyMap::default();
    let esc = make_key(KeyCode::Esc, KeyModifiers::NONE);

    let _ = handle_key(&mut app, esc, &keymap);
    let second = handle_key(&mut app, esc, &keymap);
    assert!(
        matches!(second, KeyResult::Nothing),
        "double-Esc with is_generating=false must return Nothing"
    );
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
    app.input
        .suggestions
        .suggestions
        .push(archon_tui::commands::CommandInfo {
            name: "test".into(),
            description: "test cmd".into(),
        });
    let keymap = KeyMap::default();
    let key = make_key(KeyCode::Tab, KeyModifiers::NONE);
    let result = handle_key(&mut app, key, &keymap);
    assert!(matches!(result, KeyResult::Nothing));
    assert!(!app.input.suggestions.active);
}

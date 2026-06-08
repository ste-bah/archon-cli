use archon_tui::keybindings::{Action, KeyMap};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn enter_submits() {
    let km = KeyMap::default();
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(km.resolve(enter), Some(&Action::Submit));
}

#[test]
fn ctrl_c_quits() {
    let km = KeyMap::default();
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(km.resolve(ctrl_c), Some(&Action::Quit));
}

#[test]
fn page_up_scrolls_up() {
    let km = KeyMap::default();
    let pgup = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
    assert_eq!(km.resolve(pgup), Some(&Action::ScrollUp));
}

#[test]
fn ctrl_arrows_scroll_output_for_wsl_terminals() {
    let km = KeyMap::default();
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL)),
        Some(&Action::ScrollUp)
    );
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL)),
        Some(&Action::ScrollDown)
    );
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        Some(&Action::ScrollTop)
    );
    assert_eq!(
        km.resolve(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)),
        Some(&Action::ScrollBottom)
    );
}

#[test]
fn slash_opens_command() {
    let km = KeyMap::default();
    let slash = KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(matches!(km.resolve(slash), Some(&Action::SlashCommand(_))));
}

#[test]
fn escape_is_escape() {
    let km = KeyMap::default();
    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(km.resolve(esc), Some(&Action::Escape));
}

#[test]
fn unknown_key_returns_none() {
    let km = KeyMap::default();
    let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
    assert_eq!(km.resolve(f1), None);
}

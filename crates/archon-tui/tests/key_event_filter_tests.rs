use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

// Helpers to construct KeyEvents with a specific kind

fn key_with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind,
        state: KeyEventState::NONE,
    }
}

fn ctrl_key_with_kind(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::CONTROL,
        kind,
        state: KeyEventState::NONE,
    }
}

// ---- Press events return true ----

#[test]
fn press_char_keys_are_processed() {
    assert!(archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Char('a'), KeyEventKind::Press)
    ));
    assert!(archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Char('z'), KeyEventKind::Press)
    ));
}

#[test]
fn press_ctrl_combos_are_processed() {
    assert!(archon_tui::should_process_key_event(
        &ctrl_key_with_kind(KeyCode::Char('c'), KeyEventKind::Press)
    ));
    assert!(archon_tui::should_process_key_event(
        &ctrl_key_with_kind(KeyCode::Char('d'), KeyEventKind::Press)
    ));
}

#[test]
fn press_special_keys_are_processed() {
    for code in [
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::Up,
        KeyCode::Down,
    ] {
        assert!(
            archon_tui::should_process_key_event(&key_with_kind(code, KeyEventKind::Press)),
            "Press {:?} should be processed",
            code,
        );
    }
}

// ---- Release events return false ----

#[test]
fn release_char_keys_are_ignored() {
    assert!(!archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Char('a'), KeyEventKind::Release)
    ));
    assert!(!archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Char('z'), KeyEventKind::Release)
    ));
}

#[test]
fn release_ctrl_combos_are_ignored() {
    assert!(!archon_tui::should_process_key_event(
        &ctrl_key_with_kind(KeyCode::Char('c'), KeyEventKind::Release)
    ));
}

#[test]
fn release_special_keys_are_ignored() {
    for code in [
        KeyCode::Enter,
        KeyCode::Backspace,
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::Up,
        KeyCode::Down,
    ] {
        assert!(
            !archon_tui::should_process_key_event(&key_with_kind(code, KeyEventKind::Release)),
            "Release {:?} should be ignored",
            code,
        );
    }
}

// ---- Repeat events return true (held keys like backspace/arrows) ----

#[test]
fn repeat_events_are_processed() {
    assert!(archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Backspace, KeyEventKind::Repeat)
    ));
    assert!(archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Up, KeyEventKind::Repeat)
    ));
    assert!(archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Down, KeyEventKind::Repeat)
    ));
    assert!(archon_tui::should_process_key_event(
        &key_with_kind(KeyCode::Char('a'), KeyEventKind::Repeat)
    ));
}

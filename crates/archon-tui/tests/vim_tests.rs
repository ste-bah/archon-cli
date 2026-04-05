use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

// Helper to create a key event for testing
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn char_key(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

// ---- Mode transitions ----

#[test]
fn initial_mode_is_normal() {
    let state = archon_tui::vim::VimState::new();
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Normal);
}

#[test]
fn i_enters_insert() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key('i'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Insert);
}

#[test]
fn a_enters_insert_after() {
    let mut state = archon_tui::vim::VimState::from_text("ab");
    // cursor starts at 0; 'a' should enter insert after cursor (col 1)
    state.handle_key(char_key('a'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Insert);
    let (_row, col) = state.cursor();
    assert_eq!(col, 1);
}

#[test]
fn esc_returns_to_normal() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key('i'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Insert);
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Normal);
}

#[test]
fn v_enters_visual() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key('v'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Visual);
}

#[test]
fn esc_from_visual() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key('v'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Visual);
    state.handle_key(key(KeyCode::Esc));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Normal);
}

#[test]
fn colon_enters_command() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key(':'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Command);
}

// ---- Movement ----

#[test]
fn hjkl_movement() {
    let mut state = archon_tui::vim::VimState::from_text("abcd\nefgh\nijkl");
    // Start at (0,0). Move right with 'l'
    state.handle_key(char_key('l'));
    assert_eq!(state.cursor(), (0, 1));
    // Move down with 'j'
    state.handle_key(char_key('j'));
    assert_eq!(state.cursor(), (1, 1));
    // Move left with 'h'
    state.handle_key(char_key('h'));
    assert_eq!(state.cursor(), (1, 0));
    // Move up with 'k'
    state.handle_key(char_key('k'));
    assert_eq!(state.cursor(), (0, 0));
}

#[test]
fn w_moves_word_forward() {
    let mut state = archon_tui::vim::VimState::from_text("hello world");
    // cursor at 0; 'w' should go to start of "world" at col 6
    state.handle_key(char_key('w'));
    assert_eq!(state.cursor(), (0, 6));
}

#[test]
fn b_moves_word_backward() {
    let mut state = archon_tui::vim::VimState::from_text("hello world");
    // Move to "world" first
    state.handle_key(char_key('w'));
    assert_eq!(state.cursor(), (0, 6));
    // 'b' should go back to start of "hello"
    state.handle_key(char_key('b'));
    assert_eq!(state.cursor(), (0, 0));
}

#[test]
fn zero_moves_to_line_start() {
    let mut state = archon_tui::vim::VimState::from_text("hello world");
    // Move to middle
    state.handle_key(char_key('w'));
    assert_ne!(state.cursor().1, 0);
    // '0' → col 0
    state.handle_key(char_key('0'));
    assert_eq!(state.cursor(), (0, 0));
}

#[test]
fn dollar_moves_to_line_end() {
    let mut state = archon_tui::vim::VimState::from_text("hello");
    // '$' should go to last col (len - 1 = 4)
    state.handle_key(char_key('$'));
    assert_eq!(state.cursor(), (0, 4));
}

// ---- Editing ----

#[test]
fn dd_deletes_line() {
    let mut state = archon_tui::vim::VimState::from_text("line1\nline2");
    state.handle_key(char_key('d'));
    state.handle_key(char_key('d'));
    assert_eq!(state.text(), "line2");
}

#[test]
fn yy_p_duplicates_line() {
    let mut state = archon_tui::vim::VimState::from_text("line1\nline2");
    // yy yanks first line
    state.handle_key(char_key('y'));
    state.handle_key(char_key('y'));
    // p pastes after current line
    state.handle_key(char_key('p'));
    assert_eq!(state.text(), "line1\nline1\nline2");
}

#[test]
fn x_deletes_char() {
    let mut state = archon_tui::vim::VimState::from_text("abc");
    state.handle_key(char_key('x'));
    assert_eq!(state.text(), "bc");
}

#[test]
fn insert_typing() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key('i'));
    state.handle_key(char_key('h'));
    state.handle_key(char_key('i'));
    assert!(state.text().contains("hi"));
}

// ---- Undo/Redo ----

#[test]
fn undo_restores_state() {
    let mut state = archon_tui::vim::VimState::from_text("original");
    // Enter insert, type something, go back to normal
    state.handle_key(char_key('i'));
    state.handle_key(char_key('X'));
    state.handle_key(key(KeyCode::Esc));
    // Now undo
    state.handle_key(char_key('u'));
    assert_eq!(state.text(), "original");
}

#[test]
fn redo_after_undo() {
    let mut state = archon_tui::vim::VimState::from_text("original");
    state.handle_key(char_key('i'));
    state.handle_key(char_key('X'));
    state.handle_key(key(KeyCode::Esc));
    let after_edit = state.text().to_string();
    // Undo
    state.handle_key(char_key('u'));
    assert_eq!(state.text(), "original");
    // Redo with Ctrl-R
    state.handle_key(key_ctrl(KeyCode::Char('r')));
    assert_eq!(state.text(), after_edit);
}

// ---- Count prefix ----

#[test]
fn three_dd_deletes_three_lines() {
    let mut state = archon_tui::vim::VimState::from_text("a\nb\nc\nd");
    state.handle_key(char_key('3'));
    state.handle_key(char_key('d'));
    state.handle_key(char_key('d'));
    assert_eq!(state.text(), "d");
}

// ---- Command mode ----

#[test]
fn colon_w_submits() {
    use archon_tui::vim::VimAction;
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key(':'));
    state.handle_key(char_key('w'));
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(action, VimAction::Submit);
}

#[test]
fn colon_q_quits() {
    use archon_tui::vim::VimAction;
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key(':'));
    state.handle_key(char_key('q'));
    let action = state.handle_key(key(KeyCode::Enter));
    assert_eq!(action, VimAction::Quit);
}

// ---- Config ----

#[test]
fn vim_mode_default_false() {
    let cfg = archon_tui::vim::TuiConfig::default();
    assert!(!cfg.vim_mode);
}

// ---- Text API ----

#[test]
fn text_returns_buffer() {
    let mut state = archon_tui::vim::VimState::new();
    state.handle_key(char_key('i'));
    for c in "hello".chars() {
        state.handle_key(char_key(c));
    }
    assert_eq!(state.text(), "hello");
}

#[test]
fn replace_char_works() {
    let mut state = archon_tui::vim::VimState::from_text("hello");
    // 'r' then 'X' should replace 'h' with 'X'
    state.handle_key(char_key('r'));
    state.handle_key(char_key('X'));
    assert_eq!(state.text(), "Xello", "r should replace char under cursor");
    assert_eq!(
        state.mode(),
        archon_tui::vim::VimMode::Normal,
        "should stay in normal mode after r"
    );
}

#[test]
fn gg_moves_to_first_line() {
    let mut state = archon_tui::vim::VimState::from_text("line1\nline2\nline3");
    // Move to line 3
    state.handle_key(char_key('j'));
    state.handle_key(char_key('j'));
    assert_eq!(state.cursor().0, 2);
    // gg should go to first line
    state.handle_key(char_key('g'));
    state.handle_key(char_key('g'));
    assert_eq!(state.cursor().0, 0, "gg should move to first line");
}

#[test]
fn big_g_moves_to_last_line() {
    let mut state = archon_tui::vim::VimState::from_text("line1\nline2\nline3");
    assert_eq!(state.cursor().0, 0);
    // G should go to last line
    state.handle_key(char_key('G'));
    assert_eq!(state.cursor().0, 2, "G should move to last line");
}

#[test]
fn visual_yank_works() {
    let mut state = archon_tui::vim::VimState::from_text("hello world");
    // Enter visual, select 5 chars, yank
    state.handle_key(char_key('v'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Visual);
    for _ in 0..4 {
        state.handle_key(char_key('l'));
    }
    state.handle_key(char_key('y'));
    // Should be back in normal mode
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Normal);
    // Paste after should produce the yanked text
    state.handle_key(char_key('$')); // go to end
    state.handle_key(char_key('p'));
    assert!(
        state.text().contains("hello"),
        "visual yank+paste should work"
    );
}

#[test]
fn visual_delete_works() {
    let mut state = archon_tui::vim::VimState::from_text("hello world");
    // Enter visual, select 5 chars (hello), delete
    state.handle_key(char_key('v'));
    for _ in 0..4 {
        state.handle_key(char_key('l'));
    }
    state.handle_key(char_key('d'));
    assert_eq!(state.mode(), archon_tui::vim::VimMode::Normal);
    assert_eq!(
        state.text(),
        " world",
        "visual delete should remove selected text"
    );
}

#[test]
fn mode_display_strings() {
    let mut state = archon_tui::vim::VimState::new();
    assert_eq!(state.mode_display(), "-- NORMAL --");

    state.handle_key(char_key('i'));
    assert_eq!(state.mode_display(), "-- INSERT --");

    state.handle_key(key(KeyCode::Esc));
    state.handle_key(char_key('v'));
    assert_eq!(state.mode_display(), "-- VISUAL --");

    state.handle_key(key(KeyCode::Esc));
    state.handle_key(char_key(':'));
    assert_eq!(state.mode_display(), "-- COMMAND --");
}

use archon_tui::prompt_input::PromptBuffer;

#[test]
fn insert_chars() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_char('i');
    assert_eq!(buf.text(), "hi");
}

#[test]
fn insert_newline() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_char('i');
    buf.insert_newline();
    buf.insert_char('o');
    assert_eq!(buf.text(), "hi\no");
}

#[test]
fn backspace_at_line_start_joins_lines() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_newline();
    buf.insert_char('o');
    // Insert newline at col 1 of "h" then add 'o' → cursor at (1, 1)
    assert_eq!(buf.cursor(), (1, 1));
    buf.backspace();
    // Backspace at (1,1): removes char before cursor = 'o'
    assert_eq!(buf.cursor(), (1, 0));
    assert_eq!(buf.text(), "h\n");
}

#[test]
fn cursor_movement() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    buf.move_left();
    assert_eq!(buf.cursor(), (0, 1));
    buf.move_up();
    // move_up from row 0 does nothing (already at top)
    assert_eq!(buf.cursor(), (0, 1));
    buf.move_down();
    // move_down from last row does nothing
    assert_eq!(buf.cursor(), (0, 1));
}

#[test]
fn submit_returns_text_and_clears() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_char('i');
    let text = buf.submit();
    assert_eq!(text, "hi");
    assert!(buf.is_empty());
}

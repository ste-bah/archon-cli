//! Basic tests for PromptBuffer multiline editor.

use archon_tui::prompt_input::PromptBuffer;

#[test]
fn test_new_buffer_is_empty() {
    let buf = PromptBuffer::new();
    assert!(buf.is_empty());
    assert_eq!(buf.line_count(), 1);
    assert_eq!(buf.cursor(), (0, 0));
}

#[test]
fn test_insert_char() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_char('i');
    assert_eq!(buf.text(), "hi");
    assert_eq!(buf.cursor(), (0, 2));
}

#[test]
fn test_insert_newline() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_char('i');
    buf.insert_newline();
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.cursor(), (1, 0));
    // text() joins lines with \n, so line 0 = "hi", line 1 = "" gives "hi\n"
    assert_eq!(buf.text(), "hi\n");
}

#[test]
fn test_backspace_at_col_zero_joins_lines() {
    // Create buffer: line 0 = "ab", line 1 = "c"
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    buf.insert_newline();
    buf.insert_char('c');
    // cursor at (1, 1) - end of line "c"
    // Move cursor to col 0 using move_left
    buf.move_left();
    // Now cursor at (1, 0) - start of line "c"
    assert_eq!(buf.cursor(), (1, 0));
    buf.backspace();
    // Should join: "abc" on line 0
    assert_eq!(buf.line_count(), 1);
    assert_eq!(buf.text(), "abc");
    assert_eq!(buf.cursor(), (0, 2));
}

#[test]
fn test_backspace_middle_of_line() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    buf.insert_char('c');
    // cursor at (0, 3) - after 'c'
    buf.backspace();
    // removes 'c', cursor at (0, 2)
    assert_eq!(buf.text(), "ab");
    assert_eq!(buf.cursor(), (0, 2));
}

#[test]
fn test_backspace_at_col_zero_on_first_line_does_nothing() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('x');
    buf.move_left(); // cursor at (0, 0)
    // Now at col 0 of first line - backspace should do nothing
    buf.backspace();
    assert_eq!(buf.text(), "x");
    assert_eq!(buf.cursor(), (0, 0));
}

#[test]
fn test_delete() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    buf.insert_char('c');
    // cursor at (0, 3)
    buf.delete();
    // No char after cursor on same line, check next line behavior
    assert_eq!(buf.text(), "abc");
}

#[test]
fn test_delete_removes_char_at_cursor() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_newline();
    buf.insert_char('b');
    // cursor at (1, 1) - after 'b'
    buf.move_left();
    // cursor at (1, 0) - before 'b'
    buf.delete();
    // Should remove 'b', leaving empty line
    assert_eq!(buf.line_count(), 2);
    assert_eq!(buf.text(), "a\n");
}

#[test]
fn test_move_left() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    // cursor at (0, 2)
    buf.move_left();
    assert_eq!(buf.cursor(), (0, 1));
    buf.move_left();
    assert_eq!(buf.cursor(), (0, 0));
    buf.move_left();
    // Should stay at (0, 0) - cannot go further left
    assert_eq!(buf.cursor(), (0, 0));
}

#[test]
fn test_move_left_across_lines() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_newline();
    buf.insert_char('b');
    // cursor at (1, 1)
    buf.move_left();
    // Should move left within line 1 to (1, 0)
    assert_eq!(buf.cursor(), (1, 0));
    buf.move_left();
    // At start of line 1, should move to end of line 0
    assert_eq!(buf.cursor(), (0, 1));
}

#[test]
fn test_move_right() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_newline();
    buf.insert_char('b');
    // cursor at (1, 1)
    buf.move_left(); // to (1, 0)
    buf.move_right(); // to (1, 1)
    assert_eq!(buf.cursor(), (1, 1));
    buf.move_left(); // to (1, 0)
    // At start of line 1, move_left goes to end of line 0
    buf.move_left();
    // Now at (0, 1)
    buf.move_right();
    // Should move to (1, 0) - start of next line
    assert_eq!(buf.cursor(), (1, 0));
}

#[test]
fn test_move_right_at_end_of_last_line() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_newline();
    buf.insert_char('b');
    // cursor at (1, 1) - end of buffer
    buf.move_right();
    assert_eq!(buf.cursor(), (1, 1)); // should stay
}

#[test]
fn test_move_up() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    buf.insert_newline();
    buf.insert_char('c');
    // cursor at (1, 1)
    buf.move_up();
    assert_eq!(buf.cursor(), (0, 1));
    buf.move_up();
    // Already at top, should stay
    assert_eq!(buf.cursor(), (0, 1));
}

#[test]
fn test_move_down() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_newline();
    buf.insert_char('b');
    buf.insert_char('c');
    // cursor at (1, 2)
    buf.move_down();
    // Already at bottom, should stay
    assert_eq!(buf.cursor(), (1, 2));
}

#[test]
fn test_submit() {
    let mut buf = PromptBuffer::new();
    buf.insert_char('h');
    buf.insert_char('i');
    buf.insert_newline();
    buf.insert_char('t');
    buf.insert_char('h');
    buf.insert_char('e');
    buf.insert_char('r');
    let text = buf.submit();
    assert_eq!(text, "hi\nther");
    // Buffer should be reset
    assert!(buf.is_empty());
    assert_eq!(buf.cursor(), (0, 0));
}

#[test]
fn test_text_empty() {
    let buf = PromptBuffer::new();
    assert_eq!(buf.text(), "");
}

#[test]
fn test_line_count() {
    let mut buf = PromptBuffer::new();
    assert_eq!(buf.line_count(), 1);
    buf.insert_newline();
    assert_eq!(buf.line_count(), 2);
    buf.insert_newline();
    assert_eq!(buf.line_count(), 3);
}

#[test]
fn test_cursor_position_clamp_on_move_up() {
    // When moving up from a shorter line to a longer line,
    // cursor should clamp to shorter length
    let mut buf = PromptBuffer::new();
    buf.insert_char('a');
    buf.insert_char('b');
    buf.insert_char('c');
    buf.insert_newline();
    buf.insert_char('x');
    // cursor at (1, 1)
    buf.move_up();
    // Line 0 has 3 chars, so cursor should be at (0, 1)
    assert_eq!(buf.cursor(), (0, 1));
}

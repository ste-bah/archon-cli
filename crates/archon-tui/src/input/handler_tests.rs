//! `InputHandler` unit tests, split out of `input/mod.rs` to keep that file
//! under the 500-line FileSizeGuard invariant.

use super::*;

#[test]
fn basic_input() {
    let mut input = InputHandler::new();
    input.insert('H');
    input.insert('i');
    assert_eq!(input.text(), "Hi");
    assert_eq!(input.cursor(), 2);
}

#[test]
fn backspace() {
    let mut input = InputHandler::new();
    input.insert('a');
    input.insert('b');
    input.backspace();
    assert_eq!(input.text(), "a");
}

#[test]
fn submit_clears_and_adds_history() {
    let mut input = InputHandler::new();
    input.insert('x');
    let text = input.submit();
    assert_eq!(text, "x");
    assert!(input.text().is_empty());
}

#[test]
fn init_test_catalog() {
    use crate::commands::{self, CommandKind};
    commands::set_catalog(vec![
        commands::CommandInfo {
            name: "/model".into(),
            description: "Switch model".into(),
            kind: CommandKind::Primary,
        },
        commands::CommandInfo {
            name: "/cost".into(),
            description: "Show cost".into(),
            kind: CommandKind::Primary,
        },
        commands::CommandInfo {
            name: "/help".into(),
            description: "Show help".into(),
            kind: CommandKind::Primary,
        },
    ]);
}

#[test]
fn suggestions_activate_on_slash() {
    init_test_catalog();
    let mut input = InputHandler::new();
    input.insert('/');
    assert!(input.suggestions.active);
    assert!(!input.suggestions.suggestions.is_empty());
}

#[test]
fn suggestions_deactivate_on_dismiss() {
    init_test_catalog();
    let mut input = InputHandler::new();
    input.insert('/');
    assert!(input.suggestions.active);
    input.dismiss_suggestions();
    assert!(!input.suggestions.active);
}

#[test]
fn tab_completes_selected_command() {
    init_test_catalog();
    let mut input = InputHandler::new();
    // Type "/mo" to filter to /model
    for ch in "/mo".chars() {
        input.insert(ch);
    }
    assert!(input.suggestions.active);
    assert_eq!(input.suggestions.suggestions.len(), 1);
    let accepted = input.accept_suggestion();
    assert!(accepted);
    assert!(input.text().starts_with("/model"));
    assert!(!input.suggestions.active);
}

#[test]
fn suggestions_deactivate_on_non_slash() {
    let mut input = InputHandler::new();
    input.insert('h');
    assert!(!input.suggestions.active);
}

#[test]
fn suggestions_deactivate_on_backspace_past_slash() {
    init_test_catalog();
    let mut input = InputHandler::new();
    input.insert('/');
    assert!(input.suggestions.active);
    input.backspace();
    assert!(!input.suggestions.active);
}

#[test]
fn suggestions_dismiss_when_argument_typed() {
    init_test_catalog();
    let mut input = InputHandler::new();
    // Type "/model" — suggestions active
    for ch in "/model".chars() {
        input.insert(ch);
    }
    assert!(input.suggestions.active);
    // Type space + "haiku" — suggestions should dismiss
    input.insert(' ');
    assert!(
        !input.suggestions.active,
        "suggestions stayed active after argument typed"
    );
    for ch in "haiku".chars() {
        input.insert(ch);
    }
    assert!(!input.suggestions.active);
    assert_eq!(input.text(), "/model haiku");
}

#[test]
fn suggestions_stay_active_for_partial_prefix() {
    init_test_catalog();
    let mut input = InputHandler::new();
    for ch in "/mo".chars() {
        input.insert(ch);
    }
    assert!(input.suggestions.active);
    // No space yet — still completing
    assert!(
        input
            .suggestions
            .suggestions
            .iter()
            .any(|c| c.name == "/model")
    );
}

#[test]
fn set_text_replaces_buffer_and_places_cursor_at_end() {
    let mut input = InputHandler::new();
    input.set_text("/skills ");
    assert_eq!(input.text(), "/skills ");
    assert_eq!(input.cursor(), "/skills ".len());
}

#[test]
fn set_text_after_existing_text_overwrites() {
    let mut input = InputHandler::new();
    for ch in "hello".chars() {
        input.insert(ch);
    }
    assert_eq!(input.text(), "hello");
    input.set_text("/foo ");
    assert_eq!(input.text(), "/foo ");
    assert_eq!(input.cursor(), "/foo ".len());
}

#[test]
fn inject_text_accepts_multiline_paste() {
    let mut input = InputHandler::new();
    input.inject_text("first\nsecond");
    assert_eq!(input.text(), "first\nsecond");
    assert_eq!(input.cursor(), "first\nsecond".len());
}

#[test]
fn history_navigation() {
    let mut input = InputHandler::new();
    input.insert('a');
    input.submit();
    input.insert('b');
    input.submit();

    input.history_up();
    assert_eq!(input.text(), "b");
    input.history_up();
    assert_eq!(input.text(), "a");
    input.history_down();
    assert_eq!(input.text(), "b");
    input.history_down();
    assert!(input.text().is_empty());
}

// ── Multi-line drafts (issue #174) ────────────────────────────────────────

#[test]
fn insert_newline_splits_the_draft_at_the_cursor() {
    let mut input = InputHandler::new();
    for ch in "ab".chars() {
        input.insert(ch);
    }
    input.move_left();
    input.insert_newline();
    assert_eq!(input.text(), "a\nb");
    assert_eq!(input.cursor(), 2, "cursor sits after the inserted newline");
}

#[test]
fn insert_newline_does_not_open_the_slash_popup() {
    init_test_catalog();
    let mut input = InputHandler::new();
    input.insert('/');
    assert!(input.suggestions.active);
    input.insert_newline();
    assert!(
        !input.suggestions.active,
        "a draft that has grown past its first line is no longer a slash command"
    );
}

#[test]
fn multiline_draft_submits_verbatim_including_newlines() {
    let mut input = InputHandler::new();
    for ch in "first".chars() {
        input.insert(ch);
    }
    input.insert_newline();
    for ch in "second".chars() {
        input.insert(ch);
    }
    assert_eq!(input.text(), "first\nsecond");
    assert_eq!(input.submit(), "first\nsecond");
    assert!(input.text().is_empty());
    input.history_up();
    assert_eq!(
        input.text(),
        "first\nsecond",
        "history keeps the newlines too"
    );
}

#[test]
fn backspace_removes_an_inserted_newline() {
    let mut input = InputHandler::new();
    input.insert('a');
    input.insert_newline();
    input.insert('b');
    input.move_left();
    input.backspace();
    assert_eq!(input.text(), "ab");
}

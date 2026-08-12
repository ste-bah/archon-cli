//! Issue #174 part 1 — keyboard path to multi-line input.
//!
//! Covers, in order, the acceptance criteria that are about *behaviour under
//! synthesized key events* rather than terminal teardown (which lives in
//! `keyboard_enhancement_teardown.rs`):
//!
//! 1. Shift+Enter and Alt+Enter insert a newline; Enter submits.
//! 2. A multi-line draft renders with growth and internal scroll, and the
//!    cursor lands on the right cell for wrapped *and* multi-line drafts.
//! 3. Submitting a multi-line draft puts the text on the input channel
//!    verbatim, newlines included.
//! 4. Bracketed multi-line paste still works.

use archon_tui::app::App;
use archon_tui::event_loop::dispatch_terminal_event;
use archon_tui::keybindings::KeyMap;
use archon_tui::render;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

/// Type `text` into the draft one synthesized keypress at a time, so the test
/// exercises the same dispatch a real keyboard would.
async fn type_text(
    app: &mut App,
    tx: &tokio::sync::mpsc::Sender<String>,
    keymap: &KeyMap,
    text: &str,
) {
    for ch in text.chars() {
        dispatch_terminal_event(app, key(KeyCode::Char(ch), KeyModifiers::NONE), tx, keymap).await;
    }
}

fn harness() -> (
    App,
    KeyMap,
    tokio::sync::mpsc::Sender<String>,
    tokio::sync::mpsc::Receiver<String>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(16);
    let mut app = App::new();
    app.show_splash = false;
    (app, KeyMap::default(), tx, rx)
}

// ── 1. Newline chords vs submit ───────────────────────────────────────────

#[tokio::test]
async fn shift_enter_inserts_a_newline_instead_of_submitting() {
    let (mut app, keymap, tx, mut rx) = harness();
    type_text(&mut app, &tx, &keymap, "first").await;
    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::SHIFT),
        &tx,
        &keymap,
    )
    .await;
    type_text(&mut app, &tx, &keymap, "second").await;

    assert_eq!(app.input.text(), "first\nsecond");
    assert!(
        rx.try_recv().is_err(),
        "Shift+Enter must not put anything on the input channel"
    );
}

#[tokio::test]
async fn alt_enter_inserts_a_newline_on_every_terminal() {
    let (mut app, keymap, tx, mut rx) = harness();
    type_text(&mut app, &tx, &keymap, "first").await;
    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::ALT),
        &tx,
        &keymap,
    )
    .await;
    type_text(&mut app, &tx, &keymap, "second").await;

    assert_eq!(app.input.text(), "first\nsecond");
    assert!(rx.try_recv().is_err());
}

/// Without keyboard enhancement the terminal hands us a bare `Enter` even
/// when Shift was held — that is the wire-level reason Shift+Enter "sent
/// anyway". Enter must keep submitting, which is what makes Alt+Enter the
/// necessary fallback rather than a nicety.
#[tokio::test]
async fn plain_enter_still_submits() {
    let (mut app, keymap, tx, mut rx) = harness();
    type_text(&mut app, &tx, &keymap, "hello").await;
    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::NONE),
        &tx,
        &keymap,
    )
    .await;

    assert_eq!(app.input.text(), "");
    assert_eq!(rx.try_recv().expect("Enter must submit"), "hello");
}

/// `DISAMBIGUATE_ESCAPE_CODES` tags keypad Enter with `KeyEventState::KEYPAD`.
/// Pushing the flags must not cost the user their keypad Enter.
#[tokio::test]
async fn keypad_enter_still_submits_under_disambiguation() {
    let (mut app, keymap, tx, mut rx) = harness();
    type_text(&mut app, &tx, &keymap, "hello").await;
    let keypad_enter = Event::Key(KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::KEYPAD,
    });
    dispatch_terminal_event(&mut app, keypad_enter, &tx, &keymap).await;

    assert_eq!(rx.try_recv().expect("keypad Enter must submit"), "hello");
}

// ── 3. Verbatim submission over the input channel ─────────────────────────

#[tokio::test]
async fn submitting_a_multiline_draft_sends_the_newlines_through() {
    let (mut app, keymap, tx, mut rx) = harness();
    type_text(&mut app, &tx, &keymap, "line one").await;
    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::ALT),
        &tx,
        &keymap,
    )
    .await;
    type_text(&mut app, &tx, &keymap, "line two").await;
    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::SHIFT),
        &tx,
        &keymap,
    )
    .await;
    type_text(&mut app, &tx, &keymap, "line three").await;
    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::NONE),
        &tx,
        &keymap,
    )
    .await;

    assert_eq!(
        rx.try_recv().expect("Enter must submit the whole draft"),
        "line one\nline two\nline three",
        "the agent must receive the draft byte-for-byte, newlines included"
    );
}

// ── 4. Bracketed paste is unchanged ───────────────────────────────────────

#[tokio::test]
async fn bracketed_multiline_paste_still_lands_in_the_draft() {
    let (mut app, keymap, tx, mut rx) = harness();
    dispatch_terminal_event(
        &mut app,
        Event::Paste("pasted one\npasted two\n".to_string()),
        &tx,
        &keymap,
    )
    .await;
    assert_eq!(app.input.text(), "pasted one\npasted two\n");
    assert!(rx.try_recv().is_err(), "a paste never submits by itself");

    dispatch_terminal_event(
        &mut app,
        key(KeyCode::Enter, KeyModifiers::NONE),
        &tx,
        &keymap,
    )
    .await;
    assert_eq!(
        rx.try_recv().expect("Enter submits the pasted draft"),
        "pasted one\npasted two\n"
    );
}

// ── 2. Rendering: growth, internal scroll, cursor ─────────────────────────

fn render(app: &mut App, width: u16, height: u16) -> (Vec<String>, (u16, u16)) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("TestBackend");
    terminal
        .draw(|frame| render::draw(frame, app))
        .expect("draw");
    // `Terminal::draw` forwards the frame's cursor position to the backend,
    // so this is the position a real terminal would have been told.
    let position = terminal.get_cursor_position().expect("cursor position");
    let cursor = (position.x, position.y);
    let buffer = terminal.backend().buffer().clone();
    let rows = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect();
    (rows, cursor)
}

fn draft(app: &mut App, text: &str) {
    app.input.set_text(text);
}

#[test]
fn multiline_draft_grows_the_input_area() {
    let mut app = App::new();
    app.show_splash = false;
    draft(&mut app, "a\nb\nc\nd\ne\nf");
    let (rows, _) = render(&mut app, 40, 30);

    for expected in ["> a", "b", "c", "d", "e", "f"] {
        assert!(
            rows.iter().any(|row| row.trim_end() == expected),
            "row {expected:?} missing from:\n{}",
            rows.join("\n")
        );
    }
}

/// Past the growth cap the region stops expanding and scrolls instead: the
/// tail of the draft stays visible, the head scrolls off.
#[test]
fn draft_taller_than_the_cap_scrolls_internally() {
    let mut app = App::new();
    app.show_splash = false;
    let lines: Vec<String> = (0..20).map(|index| format!("draft{index:02}")).collect();
    draft(&mut app, &lines.join("\n"));
    let (rows, _) = render(&mut app, 40, 30);

    let visible = |needle: &str| rows.iter().any(|row| row.contains(needle));
    assert!(
        visible("draft19"),
        "tail must stay in view:\n{}",
        rows.join("\n")
    );
    assert!(
        !visible("draft00"),
        "head must have scrolled out of the fixed-height region:\n{}",
        rows.join("\n")
    );
}

#[test]
fn cursor_follows_the_last_row_of_a_multiline_draft() {
    let mut app = App::new();
    app.show_splash = false;
    draft(&mut app, "aa\nbb\ncc");
    let (rows, cursor) = render(&mut app, 40, 30);

    let cursor_row = rows
        .get(cursor.1 as usize)
        .unwrap_or_else(|| panic!("cursor row {} out of range", cursor.1));
    assert!(
        cursor_row.starts_with("cc"),
        "cursor must sit on the draft's last line, got row {:?} at {cursor:?}",
        cursor_row
    );
    assert_eq!(cursor.0, 2, "cursor column follows the last line's length");
}

#[test]
fn cursor_follows_a_wrapped_single_line_draft() {
    let mut app = App::new();
    app.show_splash = false;
    // 30 columns wide; "> " plus this text wraps onto a second row.
    draft(&mut app, "wrap me across the input area please");
    let (rows, cursor) = render(&mut app, 30, 30);

    let cursor_row = rows
        .get(cursor.1 as usize)
        .expect("cursor row in range")
        .trim_end();
    assert!(
        cursor_row.ends_with("please"),
        "cursor must sit on the wrapped tail row, got {cursor_row:?} at {cursor:?}"
    );
    assert_eq!(cursor.0 as usize, cursor_row.chars().count());
}

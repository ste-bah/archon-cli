//! Trigger and routing tests for the `@`-mention picker (#200 Phase 4).
//!
//! Everything here goes through `handle_key_event`, the same entry the
//! terminal drives, one keystroke at a time. That is deliberate. A test that
//! called `sync_session_mention` directly with a prepared string would pass
//! even if nothing in the real key chain ever called it — and "the screen
//! works but the key never gets there" is precisely the failure this crate's
//! routing tests exist to catch.

use super::super::input::handle_key_event;
use crate::app::App;
use crate::screens::session_mention::{MentionCandidate, SessionMentionSource};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

struct Fixed(Vec<MentionCandidate>);

impl SessionMentionSource for Fixed {
    fn candidates(&self) -> Vec<MentionCandidate> {
        self.0.clone()
    }
}

fn candidate(id: &str, label: &str) -> MentionCandidate {
    MentionCandidate {
        id: id.into(),
        label: label.into(),
        detail: "3 msgs · 1h ago".into(),
    }
}

fn app_with_sessions() -> App {
    let mut app = App::default();
    app.session_mention_source = Some(std::sync::Arc::new(Fixed(vec![
        candidate("sess-newest", "refactor the parser"),
        candidate("sess-older", "chase the flaky test"),
    ])));
    app
}

fn keymap() -> (
    tokio::sync::mpsc::Sender<String>,
    crate::keybindings::KeyMap,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    // Held open so a routed `send` cannot fail the test for the wrong reason.
    std::mem::forget(rx);
    (tx, crate::keybindings::KeyMap::default())
}

async fn press(
    app: &mut App,
    code: KeyCode,
    tx: &tokio::sync::mpsc::Sender<String>,
    keymap: &crate::keybindings::KeyMap,
) {
    handle_key_event(
        app,
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        tx,
        None,
        None,
        None,
        keymap,
    )
    .await;
}

/// Type `text` the way a person does: one key event per character.
async fn type_text(
    app: &mut App,
    text: &str,
    tx: &tokio::sync::mpsc::Sender<String>,
    keymap: &crate::keybindings::KeyMap,
) {
    for ch in text.chars() {
        press(app, KeyCode::Char(ch), tx, keymap).await;
    }
}

// ---------------------------------------------------------------------------
// Trigger
// ---------------------------------------------------------------------------

#[tokio::test]
async fn typing_an_at_opens_the_picker() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "look at @", &tx, &keymap).await;
    assert!(app.session_mention.is_some(), "no picker for a bare @");
    assert_eq!(app.session_mention.as_ref().expect("open").len(), 2);
}

/// The mutation-verified claim: the trigger must not fire on an `@` that is
/// part of a word. Break the word-boundary guard in
/// `archon_core::mention::sigil_offsets` and this test fails.
#[tokio::test]
async fn typing_an_email_address_never_opens_the_picker() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    for ch in "mail stevenbahia@gmail.com".chars() {
        press(&mut app, KeyCode::Char(ch), &tx, &keymap).await;
        assert!(
            app.session_mention.is_none(),
            "the picker opened while typing an email address, at {:?}",
            app.input.text()
        );
    }
}

#[tokio::test]
async fn a_quoted_at_never_opens_the_picker() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    for ch in r#"run "echo @here" now"#.chars() {
        press(&mut app, KeyCode::Char(ch), &tx, &keymap).await;
        assert!(
            app.session_mention.is_none(),
            "the picker opened inside a quoted string, at {:?}",
            app.input.text()
        );
    }
}

#[tokio::test]
async fn typing_narrows_the_list_without_the_keys_being_swallowed() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@sess-old", &tx, &keymap).await;
    let picker = app.session_mention.as_ref().expect("open");
    assert_eq!(
        picker.query(),
        "sess-old",
        "characters did not reach the prompt"
    );
    assert_eq!(picker.len(), 1);
    assert_eq!(app.input.text(), "@sess-old");
}

#[tokio::test]
async fn backspacing_over_the_sigil_closes_the_picker() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@se", &tx, &keymap).await;
    assert!(app.session_mention.is_some(), "precondition");
    for _ in 0..3 {
        press(&mut app, KeyCode::Backspace, &tx, &keymap).await;
    }
    assert!(app.session_mention.is_none());
}

#[tokio::test]
async fn a_space_ends_the_mention_and_closes_the_picker() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@se ", &tx, &keymap).await;
    assert!(app.session_mention.is_none());
}

#[tokio::test]
async fn esc_dismisses_the_picker_and_keeps_what_was_typed() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@se", &tx, &keymap).await;
    press(&mut app, KeyCode::Esc, &tx, &keymap).await;
    assert!(app.session_mention.is_none());
    assert_eq!(app.input.text(), "@se", "Esc must not eat the draft");
}

// ---------------------------------------------------------------------------
// Navigation and acceptance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn down_reaches_the_picker_rather_than_the_prompt() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@", &tx, &keymap).await;
    press(&mut app, KeyCode::Down, &tx, &keymap).await;
    assert_eq!(
        app.session_mention.as_ref().expect("open").selected_index(),
        1
    );
    assert_eq!(app.input.text(), "@", "Down must not type into the prompt");
}

/// The other mutation-verified claim, TUI half: what the user highlighted has
/// to become the token the send-time resolver will look for.
#[tokio::test]
async fn enter_writes_the_highlighted_session_into_the_buffer() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "compare @", &tx, &keymap).await;
    press(&mut app, KeyCode::Down, &tx, &keymap).await;
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    assert_eq!(app.input.text(), "compare @session:sess-older ");
    assert!(app.session_mention.is_none(), "the picker stayed open");
}

/// Resolution is in place, not a whole-buffer overwrite the way `/fork-at`
/// does it: the sentence around the mention survives, and so does the caret.
#[tokio::test]
async fn enter_keeps_the_rest_of_the_sentence_and_the_caret() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "compare @ with today", &tx, &keymap).await;
    for _ in 0.."with today".len() + 1 {
        press(&mut app, KeyCode::Left, &tx, &keymap).await;
    }
    assert!(
        app.session_mention.is_some(),
        "precondition: mention reopened"
    );
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    assert_eq!(app.input.text(), "compare @session:sess-newest with today");
    assert_eq!(
        &app.input.text()[..app.input.cursor()],
        "compare @session:sess-newest ",
        "the caret did not come back to the mention"
    );
}

#[tokio::test]
async fn enter_does_not_send_the_turn_while_the_picker_is_up() {
    let mut app = app_with_sessions();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    let keymap = crate::keybindings::KeyMap::default();
    type_text(&mut app, "compare @", &tx, &keymap).await;
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    assert!(
        rx.try_recv().is_err(),
        "Enter dispatched a turn instead of resolving the mention"
    );
}

/// A resolved token must be inert. If it re-triggered, every Enter would
/// reopen the picker on the token it had just written.
#[tokio::test]
async fn the_picker_does_not_reopen_on_the_token_it_just_wrote() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@", &tx, &keymap).await;
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    assert!(app.session_mention.is_none());
    type_text(&mut app, "please", &tx, &keymap).await;
    assert!(app.session_mention.is_none());
    assert_eq!(app.input.text(), "@session:sess-newest please");
}

#[tokio::test]
async fn a_second_mention_on_the_same_line_resolves_independently() {
    let mut app = app_with_sessions();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@", &tx, &keymap).await;
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    type_text(&mut app, "vs @", &tx, &keymap).await;
    press(&mut app, KeyCode::Down, &tx, &keymap).await;
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    assert_eq!(
        app.input.text(),
        "@session:sess-newest vs @session:sess-older "
    );
}

// ---------------------------------------------------------------------------
// No source injected
// ---------------------------------------------------------------------------

/// Without a source the picker still opens and explains itself. Silently not
/// opening would look identical to a broken `@` key.
#[tokio::test]
async fn without_a_source_the_picker_says_so_instead_of_staying_shut() {
    let mut app = App::default();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@", &tx, &keymap).await;
    let picker = app.session_mention.as_ref().expect("picker should open");
    assert!(picker.is_empty());
}

#[tokio::test]
async fn enter_with_nothing_to_choose_leaves_the_text_untouched() {
    let mut app = App::default();
    let (tx, keymap) = keymap();
    type_text(&mut app, "@ab", &tx, &keymap).await;
    press(&mut app, KeyCode::Enter, &tx, &keymap).await;
    assert_eq!(app.input.text(), "@ab");
    assert!(app.session_mention.is_none());
}

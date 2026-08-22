//! Tests for the `@`-mention grammar (#200 Phase 4).
//!
//! Every case here drives a real buffer. `Typed` is a two-line stand-in for
//! the TUI's input handler — a `String` and a byte cursor — because the bugs
//! this grammar exists to avoid are all *positional*: they happen when a
//! scanner is handed a tokenised line instead of the characters the user
//! actually pressed, and so never sees the `@` arrive mid-word, or the
//! backspace that walked out of a mention.

use super::*;

/// A buffer that is built by typing, the way the real one is.
#[derive(Debug, Default)]
struct Typed {
    text: String,
    cursor: usize,
}

impl Typed {
    fn of(text: &str) -> Self {
        let mut buffer = Self::default();
        buffer.type_str(text);
        buffer
    }

    fn type_str(&mut self, text: &str) -> &mut Self {
        for ch in text.chars() {
            self.text.insert(self.cursor, ch);
            self.cursor += ch.len_utf8();
        }
        self
    }

    fn backspace(&mut self, times: usize) -> &mut Self {
        for _ in 0..times {
            let Some(prev) = self.text[..self.cursor].chars().next_back() else {
                break;
            };
            self.cursor -= prev.len_utf8();
            self.text.remove(self.cursor);
        }
        self
    }

    fn left(&mut self, times: usize) -> &mut Self {
        for _ in 0..times {
            let Some(prev) = self.text[..self.cursor].chars().next_back() else {
                break;
            };
            self.cursor -= prev.len_utf8();
        }
        self
    }

    fn active(&self) -> Option<ActiveMention> {
        active_at_cursor(&self.text, self.cursor)
    }

    fn query(&self) -> Option<String> {
        self.active().map(|mention| mention.query)
    }
}

// ---------------------------------------------------------------------------
// Firing
// ---------------------------------------------------------------------------

#[test]
fn a_bare_at_opens_a_mention_with_an_empty_query() {
    assert_eq!(Typed::of("@").query().as_deref(), Some(""));
}

#[test]
fn an_at_after_a_space_opens_a_mention() {
    assert_eq!(
        Typed::of("compare this with @ab3").query().as_deref(),
        Some("ab3")
    );
}

#[test]
fn an_at_after_a_newline_opens_a_mention() {
    assert_eq!(Typed::of("first line\n@x").query().as_deref(), Some("x"));
}

// ---------------------------------------------------------------------------
// Not firing — the cases that make naive scanners wrong
// ---------------------------------------------------------------------------

/// The single most common `@` in a real prompt.
#[test]
fn an_email_address_does_not_open_a_mention() {
    assert_eq!(Typed::of("mail stevenbahia@gmail.com").query(), None);
}

#[test]
fn a_scp_style_path_does_not_open_a_mention() {
    assert_eq!(Typed::of("scp unixdude@192.168.1.27:/tmp/x").query(), None);
}

/// `HEAD@{1}`, `origin/main@{u}` — the `@` is mid-word, so it is not a sigil.
#[test]
fn a_git_revision_suffix_does_not_open_a_mention() {
    assert_eq!(Typed::of("git show origin/main@{u}").query(), None);
}

/// The shape `/files` injects. The `@` qualifies, but `:` is not a body
/// character, so the mention has already ended by the time the caret is here.
#[test]
fn a_windows_path_attachment_does_not_leave_a_mention_open() {
    assert_eq!(Typed::of("@C:\\Users\\steve\\notes.md ").query(), None);
    assert_eq!(Typed::of("@C:\\Users\\steve\\notes.md").query(), None);
}

#[test]
fn an_at_inside_a_double_quoted_string_does_not_open_a_mention() {
    assert_eq!(Typed::of(r#"run "echo @here" for me"#).query(), None);
    assert_eq!(Typed::of(r#"the flag is "@ab"#).query(), None);
}

#[test]
fn an_at_inside_backticks_does_not_open_a_mention() {
    assert_eq!(Typed::of("see `git log @ab").query(), None);
}

/// A closed quote must hand the rest of the line back.
#[test]
fn a_mention_after_a_closed_quote_still_opens() {
    assert_eq!(
        Typed::of(r#"run "echo hi" then read @ab3"#)
            .query()
            .as_deref(),
        Some("ab3")
    );
}

/// The apostrophe decision, stated as a test: contractions are prose, not
/// quoting, and treating them as an open quote would kill mentions for the
/// rest of the line.
#[test]
fn an_apostrophe_does_not_suppress_a_later_mention() {
    assert_eq!(
        Typed::of("don't forget @ab3").query().as_deref(),
        Some("ab3")
    );
}

/// One stray quote must not poison the whole draft.
#[test]
fn quote_state_resets_at_the_end_of_a_line() {
    assert_eq!(
        Typed::of("he said \"hello\nnow read @ab3")
            .query()
            .as_deref(),
        Some("ab3")
    );
}

// ---------------------------------------------------------------------------
// Editing into and out of a mention
// ---------------------------------------------------------------------------

#[test]
fn backspacing_inside_a_mention_shortens_the_query() {
    let mut buffer = Typed::of("look at @abc123");
    assert_eq!(buffer.query().as_deref(), Some("abc123"));
    buffer.backspace(3);
    assert_eq!(buffer.query().as_deref(), Some("abc"));
}

#[test]
fn backspacing_over_the_sigil_ends_the_mention() {
    let mut buffer = Typed::of("look at @ab");
    assert!(buffer.query().is_some());
    buffer.backspace(3);
    assert_eq!(
        buffer.query(),
        None,
        "the @ is gone; nothing is being typed"
    );
}

#[test]
fn typing_a_character_the_body_forbids_ends_the_mention() {
    let mut buffer = Typed::of("@ab");
    assert!(buffer.query().is_some());
    buffer.type_str(".");
    assert_eq!(buffer.query(), None);
}

/// The other half of the previous test: an ended mention must be able to come
/// back when the offending character is removed again.
#[test]
fn removing_that_character_reopens_the_mention() {
    let mut buffer = Typed::of("@ab.");
    assert_eq!(buffer.query(), None);
    buffer.backspace(1);
    assert_eq!(buffer.query().as_deref(), Some("ab"));
}

#[test]
fn moving_the_caret_out_of_a_mention_ends_it() {
    let mut buffer = Typed::of("@ab and more");
    assert_eq!(buffer.query(), None, "the caret is past the mention");
    buffer.left("and more".len() + 1);
    assert_eq!(buffer.query().as_deref(), Some("ab"));
}

/// A mention stays live with text to the right of the caret, which is what
/// happens when the user goes back to fix a reference mid-sentence.
#[test]
fn a_mention_edited_in_the_middle_of_a_line_is_still_active() {
    let mut buffer = Typed::of("compare @ab with the other one");
    buffer.left("with the other one".len() + 1);
    buffer.type_str("3");
    assert_eq!(buffer.query().as_deref(), Some("ab3"));
    assert_eq!(buffer.text, "compare @ab3 with the other one");
}

// ---------------------------------------------------------------------------
// More than one mention on a line
// ---------------------------------------------------------------------------

#[test]
fn the_second_mention_on_a_line_is_the_active_one() {
    let buffer = Typed::of("diff @aaa against @bb");
    let mention = buffer.active().expect("a mention is being typed");
    assert_eq!(mention.query, "bb");
    assert_eq!(
        &buffer.text[mention.span.clone()],
        "@bb",
        "the span must cover the mention at the caret, not the earlier one"
    );
}

#[test]
fn editing_the_first_of_two_mentions_selects_that_one() {
    let mut buffer = Typed::of("diff @aaa against @bb");
    buffer.left("against @bb".len() + 1);
    assert_eq!(buffer.query().as_deref(), Some("aaa"));
}

// ---------------------------------------------------------------------------
// Resolution in place
// ---------------------------------------------------------------------------

#[test]
fn resolving_replaces_only_the_mention_and_keeps_the_sentence() {
    let mut buffer = Typed::of("compare @ab with the other one");
    buffer.left("with the other one".len() + 1);
    let mention = buffer.active().expect("active");
    let (text, cursor) = replace_active(&buffer.text, &mention, "sess-9f3");
    assert_eq!(text, "compare @session:sess-9f3 with the other one");
    assert_eq!(&text[..cursor], "compare @session:sess-9f3 ");
}

/// The caret must always come to rest past a separator, or the next keystroke
/// lengthens the id into one no session has.
#[test]
fn resolving_at_the_end_of_the_line_appends_the_separator() {
    let buffer = Typed::of("compare @ab");
    let mention = buffer.active().expect("active");
    let (text, cursor) = replace_active(&buffer.text, &mention, "sess-9f3");
    assert_eq!(text, "compare @session:sess-9f3 ");
    assert_eq!(cursor, text.len());
}

/// Resolution has to be a fixed point: land the caret after a resolved token
/// and the picker must not reopen, or every mention would re-trigger forever.
#[test]
fn a_resolved_token_does_not_look_like_a_mention_being_typed() {
    let buffer = Typed::of("read @session:sess-9f3");
    assert_eq!(buffer.query(), None);
}

#[test]
fn resolving_the_second_mention_leaves_the_first_alone() {
    let buffer = Typed::of("diff @session:aaa against @bb");
    let mention = buffer.active().expect("active");
    let (text, _) = replace_active(&buffer.text, &mention, "ccc");
    assert_eq!(text, "diff @session:aaa against @session:ccc ");
}

// ---------------------------------------------------------------------------
// Send-time scan
// ---------------------------------------------------------------------------

fn ids(text: &str) -> Vec<String> {
    scan_tokens(text)
        .into_iter()
        .filter_map(|token| match token {
            MentionToken::Session { id, .. } => Some(id),
            MentionToken::Malformed { .. } => None,
        })
        .collect()
}

#[test]
fn both_mentions_in_a_prompt_are_found_in_order() {
    assert_eq!(
        ids("diff @session:aaa against @session:bbb please"),
        vec!["aaa".to_string(), "bbb".to_string()]
    );
}

#[test]
fn the_span_covers_exactly_the_token() {
    let text = "look at @session:ab-3 now";
    let tokens = scan_tokens(text);
    let MentionToken::Session { span, .. } = &tokens[0] else {
        panic!("expected a session token: {tokens:?}");
    };
    assert_eq!(&text[span.clone()], "@session:ab-3");
}

/// The failure mode of the whole feature, at the send-time end: a token that
/// names no session must be reportable, never silently skipped.
#[test]
fn a_namespace_with_no_id_is_reported_as_malformed() {
    assert!(matches!(
        scan_tokens("read @session: please").as_slice(),
        [MentionToken::Malformed { .. }]
    ));
}

#[test]
fn an_email_is_not_a_token_at_send_time_either() {
    assert!(scan_tokens("mail me@session:x.com").is_empty());
}

#[test]
fn a_quoted_token_is_not_resolved() {
    assert!(
        scan_tokens(r#"the literal text is "@session:aaa" verbatim"#).is_empty(),
        "quoted text is being shown, not referenced"
    );
}

#[test]
fn a_bare_mention_that_was_never_resolved_is_not_a_token() {
    assert!(
        scan_tokens("I meant @abc but never picked it").is_empty(),
        "only the namespaced form is a reference"
    );
}

#[test]
fn a_file_attachment_is_left_alone() {
    assert!(scan_tokens("@C:\\Users\\steve\\notes.md summarise this").is_empty());
}

// ---------------------------------------------------------------------------
// Boundaries
// ---------------------------------------------------------------------------

#[test]
fn a_cursor_off_the_end_or_mid_character_is_refused_rather_than_panicking() {
    assert_eq!(active_at_cursor("@ab", 99), None);
    // Multi-byte: the em dash occupies three bytes; byte 1 is inside it.
    assert_eq!(active_at_cursor("—@ab", 1), None);
}

#[test]
fn a_mention_after_multibyte_text_still_opens_at_the_right_offset() {
    let text = "café — @ab";
    let mention = active_at_cursor(text, text.len()).expect("active");
    assert_eq!(&text[mention.span.clone()], "@ab");
    assert_eq!(mention.query, "ab");
}

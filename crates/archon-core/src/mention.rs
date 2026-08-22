//! The `@`-mention grammar for cross-session references (#200 Phase 4).
//!
//! `/session-ref <id>` was the first surface onto
//! [`crate::session_reference::prepare_session_reference`]; this is the second,
//! and the one the issue actually asked for. It lives here rather than in
//! `archon-tui` because two crates need the same grammar and they must not
//! disagree about it: the TUI has to decide, on every keystroke, whether the
//! caret sits inside a mention worth popping a picker for, and the session loop
//! has to find the mentions in a finished prompt and resolve them. A second
//! copy of "what counts as a mention" would eventually answer those two
//! questions differently, and the visible symptom would be a token the user
//! picked from a list that silently resolves to nothing.
//!
//! # What starts a mention
//!
//! An `@` opens a mention only when both hold:
//!
//! 1. It is at the start of the buffer or directly preceded by whitespace.
//! 2. It is not inside an open `"` or `` ` `` span on its line.
//!
//! Rule 1 is what keeps `steve@example.com`, `origin/main@{u}` and
//! `user@host:/path` from opening a picker mid-word — the overwhelming majority
//! of `@`s in a real prompt are one of those, and a scanner that fires on them
//! interrupts ordinary typing. Rule 2 is for the other common case: text being
//! quoted *as* text — a shell line, a snippet, an error message — where the
//! user is transcribing an `@`, not composing one.
//!
//! `'` is deliberately NOT a quote character here. English contractions are far
//! more common in a chat prompt than single-quoted strings, and treating the
//! apostrophe in "don't" as an unclosed quote would suppress every mention
//! after it on the line. Quote state also resets at each newline, so one stray
//! `"` cannot poison the rest of a multi-line draft.
//!
//! # What a mention is made of
//!
//! After the `@`, a mention body runs over `[A-Za-z0-9_-]` and stops at the
//! first character outside that set. That set is exactly what a session id can
//! contain, and it is what makes the resolved form inert: a resolved mention is
//! `@session:<id>`, whose `:` is not a body character, so the caret sitting
//! after an already-resolved token reports no active mention and the picker
//! does not reopen. It is also why `@C:\Users\me\notes.md` — the shape the
//! `/files` picker injects — never opens a session picker: the `:` ends the
//! token immediately.

use std::ops::Range;

/// The namespace a resolved session mention carries.
///
/// Namespaced rather than bare because `@<path>` is already this TUI's
/// file-attachment convention. `@session:` cannot collide with it, and the
/// colon doubles as the terminator that makes the resolved token inert.
pub const SESSION_NAMESPACE: &str = "session:";

/// The character that opens a mention.
pub const SIGIL: char = '@';

/// A mention the caret is currently inside — the picker's trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMention {
    /// Byte range from the `@` up to the caret, inclusive of the sigil.
    pub span: Range<usize>,
    /// What has been typed after the `@`, which is what the picker filters on.
    pub query: String,
}

/// A `@session:` token found in a finished prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionToken {
    /// A well-formed reference to `id`.
    Session { span: Range<usize>, id: String },
    /// `@session:` with no id after it.
    ///
    /// Carried rather than skipped. A caller that dropped this on the floor
    /// would send a turn whose text says a session was referenced while
    /// nothing was attached, which is the exact silent-empty failure the
    /// whole of #200 Phase 4 is written against.
    Malformed { span: Range<usize> },
}

/// Whether `c` may appear in a mention body.
#[must_use]
pub fn is_body_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Byte offsets of every `@` in `text` that opens a mention.
///
/// One pass, used by both the caret scan and the whole-prompt scan, so the two
/// cannot drift apart on what "a mention starts here" means.
fn sigil_offsets(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut prev: Option<char> = None;
    for (offset, ch) in text.char_indices() {
        match ch {
            '\n' => quote = None,
            '"' | '`' => {
                quote = match quote {
                    Some(open) if open == ch => None,
                    Some(open) => Some(open),
                    None => Some(ch),
                };
            }
            SIGIL => {
                if quote.is_none() && prev.is_none_or(char::is_whitespace) {
                    out.push(offset);
                }
            }
            _ => {}
        }
        prev = Some(ch);
    }
    out
}

/// The mention the caret is inside, if any.
///
/// `cursor` is a byte offset into `text`. Called after every edit, so
/// backspacing shortens the query, backspacing over the `@` ends the mention,
/// and typing a character the body does not allow ends it too — there is no
/// separate "mention mode" that could get out of step with the buffer.
///
/// Only the text before the caret is examined. Quote state depends solely on
/// preceding characters, so truncating there is not an approximation, and it
/// means a mention stays active while the user types on with text already to
/// the right of the caret.
#[must_use]
pub fn active_at_cursor(text: &str, cursor: usize) -> Option<ActiveMention> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let head = &text[..cursor];
    let at = sigil_offsets(head).into_iter().next_back()?;
    let body = &head[at + SIGIL.len_utf8()..];
    if !body.chars().all(is_body_char) {
        return None;
    }
    Some(ActiveMention {
        span: at..cursor,
        query: body.to_string(),
    })
}

/// Every `@session:` token in a finished prompt, in order.
///
/// Bare `@`s and mentions in other namespaces are not tokens and are left
/// alone; this is the send-time reader, and it must not claim ownership of
/// text it was never asked about.
#[must_use]
pub fn scan_tokens(text: &str) -> Vec<MentionToken> {
    let mut out = Vec::new();
    for at in sigil_offsets(text) {
        let after_sigil = at + SIGIL.len_utf8();
        let Some(rest) = text[after_sigil..].strip_prefix(SESSION_NAMESPACE) else {
            continue;
        };
        let id: String = rest.chars().take_while(|c| is_body_char(*c)).collect();
        let end = after_sigil + SESSION_NAMESPACE.len() + id.len();
        let span = at..end;
        out.push(if id.is_empty() {
            MentionToken::Malformed { span }
        } else {
            MentionToken::Session { span, id }
        });
    }
    out
}

/// The text a chosen session is written into the buffer as.
#[must_use]
pub fn resolved_token(session_id: &str) -> String {
    format!("{SIGIL}{SESSION_NAMESPACE}{session_id}")
}

/// Replace the mention being typed with the resolved token, in place.
///
/// Returns the new buffer and where the caret should land. Deliberately unlike
/// the `/fork-at` picker, which resolves by overwriting the whole input with a
/// slash command: a mention is one noun inside a sentence the user is still
/// writing, so anything already typed around it has to survive, and the caret
/// has to come back to where the noun ended.
///
/// Landing the caret past a separator is part of the contract. Without it the
/// caret sits flush against the end of the token, where the next character
/// typed silently extends the session id into one that does not exist. When
/// the text after the mention already begins with whitespace the caret simply
/// steps over it, so resolving mid-sentence does not leave a double space.
#[must_use]
pub fn replace_active(text: &str, active: &ActiveMention, session_id: &str) -> (String, usize) {
    let rest = &text[active.span.end..];
    let separator = rest.chars().next().filter(|c| c.is_whitespace());
    let mut out = String::with_capacity(text.len() + SESSION_NAMESPACE.len() + session_id.len());
    out.push_str(&text[..active.span.start]);
    out.push_str(&resolved_token(session_id));
    if separator.is_none() {
        out.push(' ');
    }
    let cursor = out.len() + separator.map_or(0, char::len_utf8);
    out.push_str(rest);
    (out, cursor)
}

#[cfg(test)]
#[path = "mention_tests.rs"]
mod tests;

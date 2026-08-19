//! Offset-addressable output for a persistent terminal (#189 Phase 6).
//!
//! The shared PTY session is a *stream*: chunks arrive and are gone once taken.
//! `TerminalRead(id, since)` needs the opposite — output that stays addressable
//! after the fact, so an agent can start something, go away, and come back for
//! what it missed. This is that difference, and it is why the tool layer is not
//! a thin wrapper over `archon-pty`.
//!
//! Offsets are counted in raw PTY bytes, before any of the cleaning below.
//! That is deliberate: an offset has to mean the same thing on the next read,
//! and the escape sequences that get stripped are exactly the part whose length
//! nobody should have to predict.

use std::collections::VecDeque;

/// How much output one terminal keeps.
///
/// Small enough that eight of them cannot cost much memory, large enough to
/// hold a build log's tail. Passing it evicts the oldest bytes, and a read that
/// lands in the evicted region says so rather than silently starting late.
const CAPACITY: usize = 256 * 1024;

/// A window of terminal output, cleaned for a reader that is not a terminal.
pub(crate) struct BufferRead {
    pub text: String,
    /// Where to resume. Pass this back as the next `since`.
    pub next_offset: u64,
    /// Bytes that had already been evicted when this read asked for them.
    pub dropped: u64,
    /// Bytes produced but not returned, because this read hit its ceiling.
    pub remaining: u64,
}

/// A bounded ring of raw PTY bytes with absolute offsets.
pub(crate) struct OutputBuffer {
    bytes: VecDeque<u8>,
    /// Absolute offset of `bytes.front()`. Only ever increases.
    start: u64,
}

impl OutputBuffer {
    pub(crate) fn new() -> Self {
        Self {
            bytes: VecDeque::new(),
            start: 0,
        }
    }

    /// Total bytes ever produced — one past the last readable offset.
    pub(crate) fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk.iter().copied());
        let overflow = self.bytes.len().saturating_sub(CAPACITY);
        if overflow > 0 {
            self.bytes.drain(..overflow);
            self.start += overflow as u64;
        }
    }

    /// Read from `since`, returning at most `max_bytes` of raw output.
    ///
    /// A `since` past the end is clamped rather than rejected: it means the
    /// caller is up to date, which is an ordinary answer and not an error.
    pub(crate) fn read_from(&self, since: u64, max_bytes: usize) -> BufferRead {
        let since = since.min(self.end());
        let from = since.max(self.start);
        let offset = (from - self.start) as usize;
        let take = (self.bytes.len() - offset).min(max_bytes);
        let raw: Vec<u8> = self.bytes.iter().skip(offset).take(take).copied().collect();
        let next_offset = from + take as u64;
        BufferRead {
            text: sanitize(&raw),
            next_offset,
            dropped: self.start.saturating_sub(since),
            remaining: self.end() - next_offset,
        }
    }
}

/// Strip the parts of a PTY stream that only mean something to a screen.
///
/// A terminal's output is mostly cursor movement, colour and mode switching.
/// Handed to a model verbatim it is noise that costs tokens and hides the text
/// underneath, so the escape sequences come out here. Content is never dropped
/// — only the sequences that address a display this side does not have.
fn sanitize(raw: &[u8]) -> String {
    let source = String::from_utf8_lossy(raw);
    let mut chars = source.chars().peekable();
    let mut out = String::with_capacity(source.len());
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => skip_escape(&mut chars),
            // `\r\n` is a line ending; a lone `\r` is a carriage return the
            // shell uses to redraw the current line, and the redraw is what
            // matters, not the draft it overwrote.
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(ch),
            // Everything else below space is protocol, not text — including the
            // BEL a shell rings on a bad completion.
            ch if (ch as u32) < 0x20 || ch == '\u{7f}' => {}
            ch => out.push(ch),
        }
    }
    out
}

/// Consume one escape sequence, having already taken the `ESC`.
fn skip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        // CSI: parameters, then one final byte in `@`..`~`.
        Some('[') => {
            for ch in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    break;
                }
            }
        }
        // OSC — window titles and the like — runs to BEL or to ST (`ESC \`).
        Some(']') => {
            while let Some(ch) = chars.next() {
                if ch == '\u{7}' {
                    break;
                }
                if ch == '\u{1b}' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    break;
                }
            }
        }
        // Character-set selection and similar take exactly one more byte.
        Some('(' | ')' | '#' | '%') => {
            chars.next();
        }
        // Everything else — `ESC =`, `ESC 7` — is the whole sequence.
        _ => {}
    }
}

#[cfg(test)]
#[path = "terminal_buffer_tests.rs"]
mod tests;

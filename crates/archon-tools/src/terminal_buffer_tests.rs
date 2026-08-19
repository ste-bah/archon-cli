//! Tests for offset-addressable terminal output (#189 Phase 6).

use super::*;

#[test]
fn a_fresh_buffer_reads_empty_from_zero() {
    let buffer = OutputBuffer::new();
    let read = buffer.read_from(0, 4096);

    assert_eq!(read.text, "");
    assert_eq!(read.next_offset, 0);
    assert_eq!(read.dropped, 0);
    assert_eq!(read.remaining, 0);
}

/// The point of the whole type: come back later with the offset from last time
/// and get exactly what arrived in between.
#[test]
fn a_second_read_resumes_where_the_first_stopped() {
    let mut buffer = OutputBuffer::new();
    buffer.push(b"first ");
    let first = buffer.read_from(0, 4096);
    assert_eq!(first.text, "first ");

    buffer.push(b"second");
    let second = buffer.read_from(first.next_offset, 4096);

    assert_eq!(second.text, "second");
    assert_eq!(second.next_offset, 12);
    assert_eq!(second.remaining, 0);
}

#[test]
fn reading_again_with_no_new_output_returns_nothing() {
    let mut buffer = OutputBuffer::new();
    buffer.push(b"done");
    let first = buffer.read_from(0, 4096);

    let again = buffer.read_from(first.next_offset, 4096);
    assert_eq!(again.text, "");
    assert_eq!(again.next_offset, first.next_offset);
}

/// An offset past the end means "already up to date", which is an answer, not
/// a caller error — a terminal that was reaped and recreated can produce one.
#[test]
fn an_offset_past_the_end_is_clamped_rather_than_rejected() {
    let mut buffer = OutputBuffer::new();
    buffer.push(b"four");

    let read = buffer.read_from(9_000, 4096);
    assert_eq!(read.text, "");
    assert_eq!(read.next_offset, 4);
    assert_eq!(read.dropped, 0);
}

#[test]
fn a_read_is_capped_and_reports_what_it_left_behind() {
    let mut buffer = OutputBuffer::new();
    buffer.push(b"abcdefghij");

    let read = buffer.read_from(0, 4);
    assert_eq!(read.text, "abcd");
    assert_eq!(read.next_offset, 4);
    assert_eq!(read.remaining, 6);
}

/// Losing output is acceptable; losing it silently is not — the agent would
/// read a build log's tail as though it were the whole thing.
#[test]
fn output_past_capacity_is_evicted_and_the_loss_is_reported() {
    let mut buffer = OutputBuffer::new();
    buffer.push(&vec![b'x'; CAPACITY]);
    buffer.push(b"tail");

    assert_eq!(buffer.end(), CAPACITY as u64 + 4);
    let read = buffer.read_from(0, 16);

    assert_eq!(read.dropped, 4, "the first four bytes are gone");
    assert_eq!(read.next_offset, 20);
}

#[test]
fn offsets_keep_counting_across_eviction() {
    let mut buffer = OutputBuffer::new();
    buffer.push(&vec![b'x'; CAPACITY + 100]);

    assert_eq!(buffer.end(), CAPACITY as u64 + 100);
    let read = buffer.read_from(CAPACITY as u64 + 90, 4096);
    assert_eq!(read.text.len(), 10);
    assert_eq!(read.dropped, 0);
}

#[test]
fn colour_and_cursor_sequences_are_stripped_but_their_text_survives() {
    let read = OutputBuffer::from_bytes(b"\x1b[32mgreen\x1b[0m and \x1b[4;1Hmoved");

    assert_eq!(read, "green and moved");
}

#[test]
fn a_window_title_sequence_is_removed_whole() {
    let bell = OutputBuffer::from_bytes(b"\x1b]0;C:\\Windows\\cmd.exe\x07prompt>");
    assert_eq!(bell, "prompt>");

    let terminated = OutputBuffer::from_bytes(b"\x1b]0;title\x1b\\after");
    assert_eq!(terminated, "after");
}

#[test]
fn line_endings_are_normalised_and_control_bytes_dropped() {
    let read = OutputBuffer::from_bytes(b"one\r\ntwo\r\x07three\tfour");

    assert_eq!(read, "one\ntwo\nthree\tfour");
}

/// The real thing: a `cmd.exe` banner as ConPTY actually emits it.
#[test]
fn a_real_conpty_preamble_reduces_to_its_text() {
    let raw = b"\x1b[?25h\x1b[?9001h\x1b[m\x1b]0;C:\\Windows\\System32\\cmd.exe\x07\
                Microsoft Windows\x1b[?25l\r\nC:\\Users\\u>";

    let read = OutputBuffer::from_bytes(raw);

    assert_eq!(read, "Microsoft Windows\nC:\\Users\\u>");
}

#[test]
fn an_unterminated_escape_does_not_swallow_the_rest_forever() {
    // A chunk boundary can cut a sequence in half. Losing the tail of that one
    // read is fine; hanging on it would not be.
    let read = OutputBuffer::from_bytes(b"before\x1b[");
    assert_eq!(read, "before");
}

impl OutputBuffer {
    /// Push and read in one step, for the sanitiser tests.
    fn from_bytes(raw: &[u8]) -> String {
        let mut buffer = Self::new();
        buffer.push(raw);
        buffer.read_from(0, CAPACITY).text
    }
}

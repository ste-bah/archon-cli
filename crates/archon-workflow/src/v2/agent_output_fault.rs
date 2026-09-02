//! What a parse failure must carry for the repair loop to correct it.
//!
//! The repair loop is one bounded re-ask. A re-ask that names "line 41 column
//! 962" beside the first 200 characters of the reply sends the model looking
//! for a fault it cannot see, and the observed result is the same reply with
//! the same fault, then exhaustion. Every function here exists to put the
//! bytes at fault -- or the reason there is no interior fault -- into the
//! error text that the re-ask quotes. Nothing here alters the reply.

use std::fmt;

/// Marker placed immediately before the byte the parser stopped on.
pub(super) const FAULT_MARKER: &str = "<HERE>";

/// Characters of context kept before and after the fault.
const BEFORE: usize = 120;
const AFTER: usize = 60;

/// The reply is source code (a script, a module) rather than an envelope
/// carrying it. Quoting one of its tokens as a JSON syntax error says nothing
/// the model can act on; naming the shape does.
pub(super) const SOURCE_CODE_HINT: &str = "the reply is source code, not the result envelope. \
Return the envelope, and put any file contents the task asked for inside it as a JSON string value \
(escape every \" as \\\" and every line break as \\n) -- never as bare code";

/// The reply ends before a container or string closes. There is no interior
/// fault to point at; the cause is almost always file contents pasted into a
/// string value without escaping, or a reply cut short.
pub(super) const UNTERMINATED_HINT: &str = "the reply ends while a JSON value is still open \
(unterminated). If a string value carries file contents, every \" inside it must be \\\" and every \
line break \\n; the reply must end by closing every open string, array and object";

/// A literal line break or tab inside a string value: multi-line file contents
/// pasted into the envelope as-is. serde reports this at column 0 of the next
/// line, which is the break itself.
pub(super) const CONTROL_CHARACTER_HINT: &str = "a raw line break or control character sits inside \
a string value; if the value carries file contents, every line break must be written as \\n and \
every tab as \\t so the string stays on one line";

/// A parse failure together with where in the raw reply the parsed text began,
/// so the error's line and column can be resolved against the bytes the
/// parser actually saw.
#[derive(Debug)]
pub(super) struct EnvelopeParseError {
    source: serde_json::Error,
    /// Byte offset into the raw reply of the text handed to serde.
    base_offset: usize,
}

impl EnvelopeParseError {
    pub(super) fn new(source: serde_json::Error, base_offset: usize) -> Self {
        Self {
            source,
            base_offset,
        }
    }

    /// The error text the repair loop quotes: serde's own message, followed by
    /// the bytes at fault with the marker, or the reason no interior fault
    /// exists.
    pub(super) fn describe(&self, output: &str) -> String {
        let parsed = output.get(self.base_offset..).unwrap_or("");
        if self.source.is_eof() {
            let tail = tail_window(parsed);
            return format!("{}; {UNTERMINATED_HINT}; reply ends: {tail}", self.source);
        }
        let hint = if self.source.to_string().contains("control character") {
            format!("; {CONTROL_CHARACTER_HINT}")
        } else {
            String::new()
        };
        match fault_window(parsed, self.source.line(), self.source.column()) {
            Some(window) => format!("{}{hint}; at the fault: {window}", self.source),
            None => format!("{}{hint}", self.source),
        }
    }
}

impl fmt::Display for EnvelopeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

/// Bounded, single-line excerpt around a 1-based line and column, with
/// [`FAULT_MARKER`] inserted before the byte at that position. Column 0 is
/// how serde reports the line break that ends the previous line, so the marker
/// goes at the start of the line. Line breaks inside the window are shown as
/// `\n` so the excerpt stays one line.
pub(super) fn fault_window(text: &str, line: usize, column: usize) -> Option<String> {
    if line == 0 {
        return None;
    }
    let line_start = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum::<usize>();
    // serde counts columns in bytes; clamp to the line and to a char boundary.
    let line_len = text.get(line_start..)?.split('\n').next()?.len();
    let mut fault = line_start + column.saturating_sub(1).min(line_len);
    while !text.is_char_boundary(fault) {
        fault -= 1;
    }
    let start = text[..fault]
        .char_indices()
        .rev()
        .nth(BEFORE.saturating_sub(1))
        .map(|(offset, _)| offset)
        .unwrap_or(0);
    // The byte at fault plus AFTER characters beyond it.
    let end = text[fault..]
        .char_indices()
        .nth(AFTER + 1)
        .map(|(offset, _)| fault + offset)
        .unwrap_or(text.len());
    let mut window = String::new();
    if start > 0 {
        window.push_str("...");
    }
    window.push_str(&single_line(&text[start..fault]));
    window.push_str(FAULT_MARKER);
    window.push_str(&single_line(&text[fault..end]));
    if end < text.len() {
        window.push_str("...");
    }
    Some(window)
}

fn tail_window(text: &str) -> String {
    let start = text
        .char_indices()
        .rev()
        .nth(BEFORE + AFTER)
        .map(|(offset, _)| offset)
        .unwrap_or(0);
    let mut tail = String::new();
    if start > 0 {
        tail.push_str("...");
    }
    tail.push_str(&single_line(&text[start..]));
    tail.push_str(FAULT_MARKER);
    tail
}

fn single_line(text: &str) -> String {
    text.replace("\r\n", "\\n").replace(['\n', '\r'], "\\n")
}

/// Whether a reply that failed to parse is a program rather than prose or a
/// damaged envelope. Keyed on how the reply OPENS, after any code fence: a
/// module or script declaration cannot begin an envelope, and prose does not
/// begin with one either.
pub(super) fn looks_like_source_code(output: &str) -> bool {
    let mut body = output.trim_start();
    if let Some(rest) = body.strip_prefix("```") {
        body = rest
            .split_once('\n')
            .map(|(_, after_fence)| after_fence)
            .unwrap_or("")
            .trim_start();
    }
    const OPENERS: [&str; 10] = [
        "export ",
        "import ",
        "const ",
        "let ",
        "var ",
        "function ",
        "async ",
        "class ",
        "#!",
        "use ",
    ];
    OPENERS.iter().any(|opener| body.starts_with(opener))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_lands_on_the_reported_column() {
        let window = fault_window(r#"{"a": True}"#, 1, 7).unwrap();
        assert_eq!(window, r#"{"a": <HERE>True}"#);
    }

    #[test]
    fn multi_line_text_resolves_the_reported_line() {
        let text = "{\n  \"a\": 1,\n  \"b\": True\n}";
        let window = fault_window(text, 3, 8).unwrap();
        assert_eq!(window, r#"{\n  "a": 1,\n  "b": <HERE>True\n}"#);
    }

    #[test]
    fn window_is_bounded_on_both_sides() {
        let text = format!("{}X{}", "a".repeat(500), "b".repeat(500));
        let window = fault_window(&text, 1, 501).unwrap();
        assert!(window.starts_with("..."));
        assert!(window.ends_with("..."));
        assert!(window.contains(&format!("{}<HERE>X{}", "a".repeat(BEFORE), "b".repeat(AFTER))));
        assert!(window.len() < BEFORE + AFTER + 20);
    }

    #[test]
    fn column_past_the_end_marks_the_end() {
        assert_eq!(fault_window("ab", 1, 9).unwrap(), "ab<HERE>");
    }

    /// serde places a control character inside a string at column 0 of the
    /// following line: the break itself. The marker must land there, not
    /// vanish.
    #[test]
    fn column_zero_marks_the_line_break() {
        let window = fault_window("{\"a\":\"x\ny\"}", 2, 0).unwrap();
        assert_eq!(window, r#"{"a":"x\n<HERE>y"}"#);
    }

    #[test]
    fn source_code_is_recognised_with_or_without_a_fence() {
        assert!(looks_like_source_code("```js\nexport const meta = {}"));
        assert!(looks_like_source_code("\n\nimport fs from 'fs';"));
        assert!(looks_like_source_code("#!/bin/sh\necho hi"));
        assert!(!looks_like_source_code("markdown only"));
        assert!(!looks_like_source_code("```json\n{\"status\": \"accepted\"}\n```"));
        assert!(!looks_like_source_code("{\"status\": \"accepted\""));
    }
}

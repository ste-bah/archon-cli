//! Sentences and the lists an assertion runs through (#443); see the
//! parent module for the rule.

use super::{Kind, is_lead, item_marker, tokens, walk, word};

/// Every prose sentence of `text` in order, each with whether it opens
/// inside an assertion carried from the sentence or line before it. Fenced
/// code blocks are not prose and are skipped.
pub(crate) fn sentences(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut list: Option<OpenList> = None;
    for line in crate::command::topology_lint::fences::prose_lines(text) {
        // A blank line ends the list once it has items; before its first
        // item it only separates the opener from the list.
        if line.trim().is_empty() {
            if list.as_ref().is_some_and(|open| open.first.is_some()) {
                list = None;
            }
            continue;
        }
        let line_carried = list.as_mut().is_some_and(|open| open.carries(line));
        if !line_carried {
            list = None;
        }
        let mut pass = Pass::None;
        for (index, sentence) in split_sentences(line).into_iter().enumerate() {
            let carried = match (index, pass) {
                (0, _) => line_carried,
                (_, Pass::Marker) => true,
                (_, Pass::Item) => item_marker(sentence.trim_start()).is_some(),
                (_, Pass::None) => false,
            };
            out.push((sentence, carried));
            pass = continuation(sentence, carried);
        }
        if let Some(opened) = OpenList::opened_by(line) {
            list = Some(opened);
        }
    }
    out
}

fn split_sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if matches!(bytes[index], b'.' | b';' | b'!' | b'?') && bytes[index + 1] == b' ' {
            out.push(&line[start..=index]);
            start = index + 1;
        }
        index += 1;
    }
    out.push(&line[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// How a sentence hands an open enumeration to the next one.
#[derive(Debug, Clone, Copy)]
enum Pass {
    None,
    /// The sentence ended on a bare item number: the next one is that item.
    Marker,
    /// The sentence ended with `;`: a next one starting with a marker is
    /// the next item.
    Item,
}

fn continuation(segment: &str, carried: bool) -> Pass {
    let walked = walk(segment, carried);
    if !walked.enumerated || !walked.live {
        Pass::None
    } else if walked.last_marker {
        Pass::Marker
    } else if segment.trim_end().ends_with(';') {
        Pass::Item
    } else {
        Pass::None
    }
}

/// The list a line ending with a lead and `:` opens ("…, and assert:").
struct OpenList {
    min_indent: usize,
    first: Option<(usize, Kind)>,
}

impl OpenList {
    fn opened_by(line: &str) -> Option<Self> {
        let emphasis = ['*', '_'];
        let trimmed = line.trim_end().trim_end_matches(emphasis);
        let body = trimmed.strip_suffix(':')?.trim_end_matches(emphasis);
        let tokens = tokens(body);
        let last = tokens.len().checked_sub(1)?;
        let lead = if word(&tokens, Some(last)) == Some("that") {
            last.checked_sub(1)?
        } else {
            last
        };
        if !is_lead(&tokens, lead, body) {
            return None;
        }
        let content = line.trim_start();
        let indent = line.len() - content.len();
        let item = usize::from(item_marker(content).is_some());
        Some(Self {
            min_indent: indent + item,
            first: None,
        })
    }

    /// Is `line` one of the list items this opener carries into?
    fn carries(&mut self, line: &str) -> bool {
        let content = line.trim_start();
        let indent = line.len() - content.len();
        let Some((_, kind)) = item_marker(content) else {
            return false;
        };
        if indent < self.min_indent {
            return false;
        }
        match self.first {
            None => {
                self.first = Some((indent, kind));
                true
            }
            Some((first, first_kind)) => indent > first || (indent == first && kind == first_kind),
        }
    }
}

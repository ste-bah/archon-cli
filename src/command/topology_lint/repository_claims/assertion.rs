//! What a test the body specifies will check at run time is not a claim
//! about the repository (#443).
//!
//! A body that specifies a test describes what the test asserts when it runs:
//! "run the producer, and assert: (a) `out/report.json` exists; (b) …". The
//! existence there is the object of the assertion — a postcondition checked
//! in whatever root the test runs in (a temporary directory, a fresh store) —
//! and not a statement that the path is in the repository at the base
//! commit. Refuting it tells the author to delete a correct assertion.
//!
//! The rule is grammatical and narrow, so a statement of fact about the
//! repository keeps its check:
//!
//! - A lead is a verb of checking outside backticks: `assert`, `asserts` or
//!   `asserting` (not after an article or determiner, which makes it a noun,
//!   and not `assert!`), or `check`, `verify`, `expect`, `confirm`, `ensure`
//!   and their forms followed by `that` or preceded by `test`/`tests`.
//! - A path is the subject of the asserted clause when the text between the
//!   lead and the path is only the clause's opening: `:`, articles and
//!   determiners, enumeration markers (`(a)`, `-`, `1.`), coordinated
//!   backticked objects joined by `,`, `and` or `or`, and at most
//!   `MAX_CLAUSE_WORDS` other words. A comma after a plain word, a clause
//!   word (`then`, `which`, `when`, …) or other punctuation ends the clause.
//!   An enumeration marker after a lead in the same sentence starts the
//!   next item's clause.
//! - Enumerated items continue the clause. A sentence that ends with `;`
//!   while a lead is open carries it to a next sentence that starts with an
//!   enumeration marker; a line that ends with a lead and `:` carries it to
//!   the list items under it (nested deeper when the line is itself a list
//!   item), each item's first sentence opening on its marker.
//! - Only the predicate after the path is the asserted clause. A word before
//!   the path ("modify `x`", "the existing `x`", "new file `x`") is the
//!   body's own statement about the repository and is still checked.

/// Every prose sentence of `text` in order, each with whether it opens
/// inside an assertion carried from the sentence or line before it. Fenced
/// code blocks are not prose and are skipped.
pub(super) fn sentences(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut list: Option<OpenList> = None;
    for line in super::super::fences::prose_lines(text) {
        if line.trim().is_empty() {
            continue;
        }
        let line_carried = list.as_ref().is_some_and(|open| open.carries(line));
        if !line_carried {
            list = None;
        }
        let mut open = false;
        for (index, sentence) in split_sentences(line).into_iter().enumerate() {
            let carried = if index == 0 {
                line_carried
            } else {
                open && starts_item(sentence)
            };
            out.push((sentence, carried));
            open = open_after(sentence, carried);
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

/// Plain words allowed between a lead (or an enumeration marker) and the
/// path it asserts of: "assert the aggregate `x` exists".
const MAX_CLAUSE_WORDS: usize = 4;

const ASSERT_VERBS: &[&str] = &["assert", "asserts", "asserting"];
const CHECK_VERBS: &[&str] = &[
    "check",
    "checks",
    "checking",
    "verify",
    "verifies",
    "verifying",
    "expect",
    "expects",
    "expecting",
    "confirm",
    "confirms",
    "confirming",
    "ensure",
    "ensures",
    "ensuring",
];
/// A word before `assert` that makes it a noun ("an assert in `x`").
const DETERMINERS: &[&str] = &[
    "a", "an", "the", "each", "every", "one", "this", "no", "any", "its", "their",
];
/// Words of a clause's opening that are not counted.
const FREE_WORDS: &[&str] = &[
    "that", "the", "a", "an", "both", "each", "every", "its", "their", "and", "or", "also",
];
/// Words that start another clause.
const CLAUSE_WORDS: &[&str] = &[
    "then", "which", "where", "when", "while", "after", "before", "if", "unless", "because", "so",
    "but", "since", "once",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Code,
    Word(String),
    Marker,
    Comma,
    Colon,
    Other,
}

/// The tokens of `segment`, each with its starting byte offset. A backticked
/// span is one `Code` token, paired left to right as the claim parser pairs
/// them, so a verb inside code (`assert!(…)`) is never a lead.
fn tokens(segment: &str) -> Vec<(usize, Token)> {
    let mut out = Vec::new();
    let leading = segment.len() - segment.trim_start().len();
    let mut start = 0;
    if let Some(len) = item_marker_len(&segment[leading..]) {
        out.push((leading, Token::Marker));
        start = leading + len;
    }
    tokenize_from(segment, start, &mut out);
    out
}

fn tokenize_from(segment: &str, start: usize, out: &mut Vec<(usize, Token)>) {
    let mut index = start;
    while index < segment.len() {
        let rest = &segment[index..];
        let character = rest.chars().next().unwrap_or(' ');
        if character.is_whitespace() {
            index += character.len_utf8();
        } else if character == '`' {
            let close = rest[1..].find('`').map_or(segment.len(), |c| index + c + 2);
            out.push((index, Token::Code));
            index = close;
        } else if let Some(len) = parenthesized_marker_len(rest) {
            out.push((index, Token::Marker));
            index += len;
        } else if is_word_char(character) {
            let end = rest
                .char_indices()
                .find(|(_, c)| !is_word_char(*c))
                .map_or(segment.len(), |(offset, _)| index + offset);
            out.push((index, Token::Word(segment[index..end].to_ascii_lowercase())));
            index = end;
        } else {
            let token = match character {
                ',' => Token::Comma,
                ':' => Token::Colon,
                _ => Token::Other,
            };
            out.push((index, token));
            index += character.len_utf8();
        }
    }
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '\'' | '’')
}

/// `(a)`, `(iv)`, `(2)`: a parenthesized item label followed by a space or
/// the end.
fn parenthesized_marker_len(rest: &str) -> Option<usize> {
    let inner = rest.strip_prefix('(')?;
    let close = inner.find(')')?;
    let label = &inner[..close];
    let after = &inner[close + 1..];
    (is_item_label(label) && after.chars().next().is_none_or(char::is_whitespace))
        .then_some(close + 2)
}

fn is_item_label(label: &str) -> bool {
    let single_letter = label.len() == 1 && label.chars().all(|c| c.is_ascii_alphabetic());
    let roman = !label.is_empty()
        && label.len() <= 4
        && label.chars().all(|c| matches!(c, 'i' | 'v' | 'x'));
    let number = !label.is_empty() && label.len() <= 2 && label.chars().all(|c| c.is_ascii_digit());
    single_letter || roman || number
}

/// The byte length of a list or enumeration marker at the start of `text`
/// (`- `, `* `, `+ `, `1. `, `1) `, `a) `, `(a) `), including its space.
fn item_marker_len(text: &str) -> Option<usize> {
    for bullet in ["- ", "* ", "+ "] {
        if text.starts_with(bullet) {
            return Some(bullet.len());
        }
    }
    if let Some(len) = parenthesized_marker_len(text) {
        return Some(len);
    }
    let label_end = text.find(['.', ')'])?;
    let label = &text[..label_end];
    let after = &text[label_end + 1..];
    let numbered =
        !label.is_empty() && label.len() <= 3 && label.chars().all(|c| c.is_ascii_digit());
    let lettered = text.as_bytes()[label_end] == b')' && is_item_label(label);
    ((numbered || lettered) && after.starts_with(' ')).then_some(label_end + 1)
}

fn is_lead(tokens: &[(usize, Token)], index: usize, segment: &str) -> bool {
    let Token::Word(word) = &tokens[index].1 else {
        return false;
    };
    let word_at = |offset: isize| -> Option<&str> {
        let at = index.checked_add_signed(offset)?;
        match tokens.get(at) {
            Some((_, Token::Word(w))) => Some(w.as_str()),
            _ => None,
        }
    };
    if ASSERT_VERBS.contains(&word.as_str()) {
        let start = tokens[index].0;
        let macro_call = segment[start + word.len()..].starts_with('!');
        return !macro_call && !word_at(-1).is_some_and(|w| DETERMINERS.contains(&w));
    }
    CHECK_VERBS.contains(&word.as_str())
        && (word_at(1) == Some("that") || word_at(-1).is_some_and(|w| w == "test" || w == "tests"))
}

/// The state of an assertion clause while reading a sentence left to right.
#[derive(Debug, Clone, Copy)]
struct Clause {
    words: usize,
    after_code: bool,
}

impl Clause {
    const OPEN: Clause = Clause {
        words: 0,
        after_code: false,
    };
}

/// Read `segment`'s tokens, starting inside an assertion when `carried`, and
/// report for each token whether an assertion clause was open just before it,
/// plus whether one is open at the end.
fn walk(segment: &str, carried: bool) -> (Vec<(usize, bool)>, bool) {
    let tokens = tokens(segment);
    let mut clause = carried.then_some(Clause::OPEN);
    let mut opened = carried;
    let mut seen = Vec::with_capacity(tokens.len());
    for index in 0..tokens.len() {
        let (offset, token) = &tokens[index];
        seen.push((*offset, clause.is_some()));
        if is_lead(&tokens, index, segment) {
            clause = Some(Clause::OPEN);
            opened = true;
            continue;
        }
        // An enumeration marker after a lead opens the next item's clause.
        if *token == Token::Marker && opened {
            clause = Some(Clause::OPEN);
            continue;
        }
        let Some(state) = clause.as_mut() else {
            continue;
        };
        match token {
            Token::Code => state.after_code = true,
            Token::Colon => state.after_code = false,
            Token::Comma if state.after_code => state.after_code = false,
            Token::Word(word) if FREE_WORDS.contains(&word.as_str()) => state.after_code = false,
            Token::Word(word) if !CLAUSE_WORDS.contains(&word.as_str()) => {
                state.after_code = false;
                state.words += 1;
                if state.words > MAX_CLAUSE_WORDS {
                    clause = None;
                }
            }
            _ => clause = None,
        }
    }
    (seen, opened)
}

/// Is the backticked token opening at byte `at` of `segment` the subject of
/// an asserted clause? `carried` says the segment opens inside one.
pub(super) fn asserted_at(segment: &str, carried: bool, at: usize) -> bool {
    walk(segment, carried)
        .0
        .iter()
        .find(|(offset, _)| *offset == at)
        .is_some_and(|(_, open)| *open)
}

/// Does `segment` (read with `carried`) leave an assertion open for a next
/// sentence that starts with an enumeration marker? Only a `;` continues it.
fn open_after(segment: &str, carried: bool) -> bool {
    segment.trim_end().ends_with(';') && walk(segment, carried).1
}

/// Does `segment` start with an enumeration marker that continues an open
/// assertion from the previous sentence?
fn starts_item(segment: &str) -> bool {
    item_marker_len(segment.trim_start()).is_some()
}

/// A line whose last clause is a lead and `:` ("…, and assert:") opens the
/// list under it.
struct OpenList {
    indent: usize,
    nested_only: bool,
}

impl OpenList {
    fn opened_by(line: &str) -> Option<Self> {
        let trimmed = line.trim_end();
        let body = trimmed.strip_suffix(':')?;
        let tokens = tokens(body);
        let last = tokens.len().checked_sub(1)?;
        let lead = if matches!(&tokens[last].1, Token::Word(w) if w == "that") {
            last.checked_sub(1)?
        } else {
            last
        };
        if !is_lead(&tokens, lead, body) {
            return None;
        }
        let content = line.trim_start();
        Some(Self {
            indent: line.len() - content.len(),
            nested_only: item_marker_len(content).is_some(),
        })
    }

    /// Is `line` one of the list items this opener carries into?
    fn carries(&self, line: &str) -> bool {
        let content = line.trim_start();
        let indent = line.len() - content.len();
        item_marker_len(content).is_some() && (!self.nested_only || indent > self.indent)
    }
}

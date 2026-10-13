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
//!   `asserting` (not after a determiner, which makes it a noun, not after
//!   `we`/`I`, which makes it the author's own statement, and not
//!   `assert!`), or `check`, `verify`, `expect`, `confirm`, `ensure` and
//!   their forms followed by `that` or preceded by `test`/`tests`.
//! - The asserted clause runs from the lead to the first path's predicate.
//!   Between the lead and the path only the clause's opening may stand:
//!   `:`, articles and determiners, an enumeration marker, coordinated
//!   backticked subjects joined by `,`, `and` or `or`, and at most
//!   `MAX_CLAUSE_WORDS` other words. A comma after a plain word, a clause
//!   word (`then`, `which`, …) or other punctuation ends it. A path right
//!   after a preposition ("the parser in `x`") is a location the clause
//!   presupposes, not its subject.
//! - After that predicate a new clause needs the lead again: `and that`, or
//!   the next item of an enumeration the lead opened ("assert: (a) …, (b) …";
//!   a `:` or other punctuation in between breaks the enumeration).
//! - An enumeration the lead opened continues across `;` into a next
//!   sentence that starts with a marker, and past a bare item number
//!   ("assert: 1. `x` exists; 2. `y` exists"). A line that ends with a lead
//!   and `:` carries it into the list right under it: items indented at
//!   least as far as the line (further, when the line is itself a list
//!   item), of one marker kind, up to a blank line or any other line.
//! - Only a statement of state at run time is exempt. A predicate that names
//!   the repository ("(12 lines)", "(directory)", "in the repository", "at
//!   the base commit", "in the checkout") or plans a change ("will be
//!   modified", "is new") is the body's own statement, as is a word before
//!   the path ("modify `x`", "the existing `x`").

mod sentences;
pub(super) use sentences::sentences;

/// Is the claim on the path whose backtick opens at byte `at` of `sentence`
/// what a test asserts at run time? `after` is the text after the path.
pub(super) fn exempt(sentence: &str, carried: bool, at: usize, after: &str) -> bool {
    asserted_at(sentence, carried, at) && !names_repository(after) && !plans_change(after)
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
/// A word before a lead that makes it a noun ("an assert in `x`") or the
/// author's own statement ("we assert `x` exists").
const NOT_BEFORE_LEAD: &[&str] = &[
    "a", "an", "the", "each", "every", "one", "this", "no", "any", "its", "their", "we", "i",
];
/// Words of a clause's opening that are not counted.
const FREE_WORDS: &[&str] = &[
    "that", "the", "a", "an", "both", "each", "every", "its", "their", "and", "or", "also",
];
/// Nouns that may follow a path inside its subject ("`x` file exists").
const SUBJECT_NOUNS: &[&str] = &["file", "module", "directory", "dir", "crate", "path"];
/// Words that start another clause.
const CLAUSE_WORDS: &[&str] = &[
    "then", "which", "where", "when", "while", "after", "before", "if", "unless", "because", "so",
    "but", "since", "once",
];
/// A path right after one of these is a location, not the clause subject.
const PREPOSITIONS: &[&str] = &[
    "in", "under", "at", "from", "of", "inside", "within", "into", "on", "beside", "by", "to",
    "for",
];
/// Predicate wording that names the repository the claim is checked against.
const REPOSITORY_WORDS: &[&str] = &[
    "in the repository",
    "in the repo",
    "base commit",
    "in the checkout",
    "(directory)",
    "(dir)",
];
/// Predicate openings that plan a change rather than state a fact.
const PLAN_WORDS: &[&str] = &[
    "will be",
    "to be",
    "must be",
    "needs to be",
    "need to be",
    "is new",
    "are new",
    "is a new",
    "new",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Code,
    Word(String),
    Marker,
    Comma,
    Colon,
    Dash,
    End,
    Other,
}

/// The tokens of `segment`, each with its starting byte offset. A backticked
/// span is one `Code` token, paired left to right as the claim parser pairs
/// them, so a verb inside code (`assert!(…)`) is never a lead.
fn tokens(segment: &str) -> Vec<(usize, Token)> {
    let mut out = Vec::new();
    let leading = segment.len() - segment.trim_start().len();
    let mut start = 0;
    if let Some((len, _)) = item_marker(&segment[leading..]) {
        out.push((leading, Token::Marker));
        start = leading + len;
    }
    let last = segment.trim_end().len().saturating_sub(1);
    let mut index = start;
    while index < segment.len() {
        let rest = &segment[index..];
        let character = rest.chars().next().unwrap_or(' ');
        let after_space = index == start
            || segment[..index]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if character.is_whitespace() {
            index += character.len_utf8();
        } else if character == '`' {
            let close = rest[1..].find('`').map_or(segment.len(), |c| index + c + 2);
            out.push((index, Token::Code));
            index = close;
        } else if let Some(len) = parenthesized_marker_len(rest)
            .or_else(|| after_space.then(|| number_marker_len(rest)).flatten())
        {
            out.push((index, Token::Marker));
            index += len;
        } else if is_word_char(character) {
            // A hyphen between letters joins a word ("run-real").
            let joins = |offset: usize, c: char| {
                c == '-' && rest[offset + 1..].chars().next().is_some_and(is_word_char)
            };
            let end = rest
                .char_indices()
                .find(|(offset, c)| !is_word_char(*c) && !joins(*offset, *c))
                .map_or(segment.len(), |(offset, _)| index + offset);
            out.push((index, Token::Word(segment[index..end].to_ascii_lowercase())));
            index = end;
        } else {
            let token = match character {
                ',' => Token::Comma,
                ':' => Token::Colon,
                '—' | '–' => Token::Dash,
                '.' | ';' | '!' | '?' if index == last => Token::End,
                _ => Token::Other,
            };
            out.push((index, token));
            index += character.len_utf8();
        }
    }
    out
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

/// `1.` or `1)` followed by a space or the end.
fn number_marker_len(rest: &str) -> Option<usize> {
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let after = rest[digits..].strip_prefix(['.', ')'])?;
    ((1..=3).contains(&digits) && after.chars().next().is_none_or(char::is_whitespace))
        .then_some(digits + 1)
}

fn is_item_label(label: &str) -> bool {
    let single_letter = label.len() == 1 && label.chars().all(|c| c.is_ascii_alphabetic());
    let roman = !label.is_empty()
        && label.len() <= 4
        && label.chars().all(|c| matches!(c, 'i' | 'v' | 'x'));
    let number = !label.is_empty() && label.len() <= 2 && label.chars().all(|c| c.is_ascii_digit());
    single_letter || roman || number
}

/// The kind of a list or enumeration marker; a list keeps one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Bullet(char),
    Number(char),
    Label,
}

/// The byte length and kind of a list or enumeration marker at the start of
/// `text` (`- `, `* `, `+ `, `1. `, `1) `, `a) `, `(a) `), its space included.
fn item_marker(text: &str) -> Option<(usize, Kind)> {
    for bullet in ['-', '*', '+'] {
        if text
            .strip_prefix(bullet)
            .is_some_and(|r| r.starts_with(' '))
        {
            return Some((2, Kind::Bullet(bullet)));
        }
    }
    if let Some(len) = parenthesized_marker_len(text) {
        return Some((len, Kind::Label));
    }
    if let Some(len) = number_marker_len(text) {
        return Some((len, Kind::Number(text.as_bytes()[len - 1] as char)));
    }
    let close = text.find(')')?;
    let after = &text[close + 1..];
    (is_item_label(&text[..close]) && after.chars().next().is_none_or(char::is_whitespace))
        .then_some((close + 1, Kind::Label))
}

fn word(tokens: &[(usize, Token)], index: Option<usize>) -> Option<&str> {
    match tokens.get(index?) {
        Some((_, Token::Word(w))) => Some(w.as_str()),
        _ => None,
    }
}

fn is_lead(tokens: &[(usize, Token)], index: usize, segment: &str) -> bool {
    let Some(this) = word(tokens, Some(index)) else {
        return false;
    };
    let before = word(tokens, index.checked_sub(1));
    if before.is_some_and(|w| NOT_BEFORE_LEAD.contains(&w)) {
        return false;
    }
    if ASSERT_VERBS.contains(&this) {
        let start = tokens[index].0;
        return !segment[start + this.len()..].starts_with('!');
    }
    CHECK_VERBS.contains(&this)
        && (word(tokens, Some(index + 1)) == Some("that")
            || before.is_some_and(|w| w == "test" || w == "tests"))
}

/// The opening of an asserted clause: plain words counted, and whether the
/// last token was a backticked subject.
#[derive(Debug, Clone, Copy)]
struct Clause {
    words: usize,
    after_code: bool,
}

const OPEN: Clause = Clause {
    words: 0,
    after_code: false,
};

/// One sentence read left to right.
struct Walk {
    /// Each token's offset, and whether a clause was open just before it.
    seen: Vec<(usize, bool)>,
    tokens: Vec<(usize, Token)>,
    /// The lead opened an enumeration (`assert: (a) …`), or the sentence
    /// was carried into one.
    enumerated: bool,
    /// A clause is open, or closed only by its predicate.
    live: bool,
    last_marker: bool,
}

fn walk(segment: &str, carried: bool) -> Walk {
    let tokens = tokens(segment);
    let mut clause = carried.then_some(OPEN);
    let mut enumerated = carried;
    // Closed by a predicate, with nothing since that breaks the sentence.
    let mut after_predicate = false;
    let mut seen = Vec::with_capacity(tokens.len());
    for index in 0..tokens.len() {
        let token = &tokens[index].1;
        seen.push((tokens[index].0, clause.is_some()));
        if is_lead(&tokens, index, segment) {
            clause = Some(OPEN);
            enumerated = false;
            after_predicate = false;
            continue;
        }
        let Some(state) = clause.as_mut() else {
            match token {
                Token::Marker if enumerated && after_predicate => clause = Some(OPEN),
                Token::Word(w)
                    if w == "that"
                        && after_predicate
                        && matches!(word(&tokens, index.checked_sub(1)), Some("and" | "or")) =>
                {
                    clause = Some(OPEN)
                }
                // Inside an item's predicate only a new clause breaks the
                // enumeration ("…, and note: (b)"); parentheses do not.
                Token::Colon | Token::Dash => after_predicate = false,
                Token::Word(w) if CLAUSE_WORDS.contains(&w.as_str()) => after_predicate = false,
                _ => {}
            }
            continue;
        };
        match token {
            Token::Marker => {
                *state = OPEN;
                enumerated = true;
            }
            Token::Code => state.after_code = true,
            Token::Colon => state.after_code = false,
            Token::End => {}
            Token::Comma if state.after_code => state.after_code = false,
            Token::Word(w) if SUBJECT_NOUNS.contains(&w.as_str()) && state.after_code => {}
            Token::Word(w) if FREE_WORDS.contains(&w.as_str()) => state.after_code = false,
            Token::Word(w) if CLAUSE_WORDS.contains(&w.as_str()) => clause = None,
            Token::Word(_) if state.after_code => {
                clause = None;
                after_predicate = true;
            }
            Token::Word(_) => {
                state.words += 1;
                if state.words > MAX_CLAUSE_WORDS {
                    clause = None;
                }
            }
            _ => clause = None,
        }
    }
    Walk {
        seen,
        enumerated,
        live: clause.is_some() || after_predicate,
        last_marker: tokens.last().is_some_and(|(_, t)| *t == Token::Marker),
        tokens,
    }
}

fn asserted_at(segment: &str, carried: bool, at: usize) -> bool {
    let walked = walk(segment, carried);
    let Some(index) = walked.seen.iter().position(|(offset, _)| *offset == at) else {
        return false;
    };
    let located = index
        .checked_sub(1)
        .and_then(|before| word(&walked.tokens, Some(before)))
        .is_some_and(|w| PREPOSITIONS.contains(&w));
    walked.seen[index].1 && !located
}

/// The predicate text after a path: up to the next backtick.
fn predicate(after: &str) -> String {
    after
        .split('`')
        .next()
        .unwrap_or(after)
        .to_ascii_lowercase()
}

fn names_repository(after: &str) -> bool {
    let text = predicate(after);
    REPOSITORY_WORDS.iter().any(|w| text.contains(w))
        || text.match_indices('(').any(|(open, _)| {
            let inner = text[open + 1..].trim_start();
            let digits = inner
                .chars()
                .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '_'))
                .count();
            digits > 0 && inner[digits..].trim_start().starts_with("line")
        })
}

fn plans_change(after: &str) -> bool {
    let text = predicate(after);
    let rest = text.trim_start_matches(|c: char| {
        c.is_whitespace() || matches!(c, '—' | '–' | '-' | ':' | ',' | '(')
    });
    PLAN_WORDS.iter().any(|phrase| {
        rest.strip_prefix(phrase)
            .is_some_and(|tail| tail.chars().next().is_none_or(|c| !c.is_alphanumeric()))
    })
}

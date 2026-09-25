//! Recognising the line that starts a function declaration.
//!
//! - keyword headers: `fn name(` / `fn name<..>(` / `fn r#name(` behind
//!   attributes (`#[inline]`) and Rust qualifiers (`pub`, `pub(crate)`,
//!   `async`, `const`, `unsafe`, `extern "C"`, ...).
//! - `func name(` and Go receiver methods `func (s *T) Name(`.
//! - the name-before-`(` form other brace languages use (`int name(`,
//!   `function name(`, `public Foo name(`). It never applies to `.rs` files,
//!   where every function is a keyword header and the form only ever matched
//!   control flow.

use super::valid_name;

/// Leading words after which a name-before-`(` line is control flow, not a
/// declaration (`if let Some(x) = f(y) {`, `when (x) {`, `guard let ...`).
const CONTROL_WORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "foreach",
    "while",
    "do",
    "loop",
    "match",
    "switch",
    "case",
    "catch",
    "try",
    "return",
    "when",
    "guard",
    "throw",
    "using",
    "lock",
    "synchronized",
];

/// Qualifiers that may precede `fn` in a Rust declaration.
const FN_QUALIFIERS: &[&str] = &["async", "const", "unsafe", "safe", "extern", "default"];

/// The name a Rust `fn` header declares.
pub(super) fn keyword_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let (prefix, rest) = match trimmed.strip_prefix("fn ") {
        Some(rest) => ("", rest),
        None => trimmed.split_once(" fn ")?,
    };
    if !only_fn_qualifiers(prefix) {
        return None;
    }
    let rest = rest.trim_start();
    let raw = if rest.starts_with("r#") { 2 } else { 0 };
    let end = raw + identifier_len(&rest[raw..]);
    let (name, tail) = rest.split_at(end);
    let tail = tail.trim_start();
    let opens_params = tail.starts_with('(') || tail.starts_with('<');
    (end > raw && opens_params).then_some(name)
}

fn identifier_len(text: &str) -> usize {
    text.find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(text.len())
}

fn only_fn_qualifiers(prefix: &str) -> bool {
    let Some(mut rest) = without_attributes(prefix.trim()) else {
        return false;
    };
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.trim_start().strip_prefix('(') {
            Some(scope) => match scope.split_once(')') {
                Some((_, tail)) => tail,
                None => return false,
            },
            None => after,
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return false;
        }
    }
    rest.split_whitespace().all(|word| {
        FN_QUALIFIERS.contains(&word)
            || (word.len() >= 2 && word.starts_with('"') && word.ends_with('"'))
    })
}

/// `prefix` after its leading `#[...]` attributes; `None` if one is unclosed.
fn without_attributes(mut prefix: &str) -> Option<&str> {
    while let Some(attribute) = prefix.strip_prefix("#[") {
        let mut depth = 1;
        let end = attribute.find(|ch: char| {
            match ch {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            }
            depth == 0
        })?;
        prefix = attribute[end + 1..].trim_start();
    }
    Some(prefix)
}

/// The name before `(` on a non-keyword header line, unless the line is
/// control flow, continues a parameter list, or the word is cut inside a
/// generic argument list (`Result<(), E>` gives `Result<`).
pub(super) fn name_before_paren(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with(')') {
        return None;
    }
    if let Some(declared) = func_name(trimmed) {
        return declared;
    }
    let before = trimmed.split_once('(')?.0.trim();
    let first = before.trim_start_matches('}').split_whitespace().next()?;
    if CONTROL_WORDS.contains(&first) {
        return None;
    }
    let name = before.split_whitespace().last()?;
    let balanced = name.matches('<').count() == name.matches('>').count();
    (balanced && valid_name(name)).then_some(name)
}

/// For a line opening with the `func` keyword: `Some(Some(name))` for
/// `func name(`, `func name[T any](`, `func (s *T) Name(`; `Some(None)` for
/// an anonymous `func(`; `None` when the line does not open with `func`.
fn func_name(trimmed: &str) -> Option<Option<&str>> {
    let rest = trimmed.strip_prefix("func")?;
    if !rest.starts_with(|ch: char| ch.is_whitespace() || ch == '(') {
        return None;
    }
    let mut rest = rest.trim_start();
    if let Some(receiver) = rest.strip_prefix('(') {
        rest = match receiver.split_once(')') {
            Some((_, tail)) => tail.trim_start(),
            None => return Some(None),
        };
    }
    let name = &rest[..identifier_len(rest)];
    Some((!name.is_empty()).then_some(name))
}

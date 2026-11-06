//! The deliberately narrow expansion-only command template domain.

/// A list is a template iff every simple command, after leading assignments,
/// has only empty-able words (or no words). Literal arguments can become the
/// program after an unset expansion, so checking just the first word is wrong.
/// This is the declared template domain, not a general shell interpreter.
pub(super) fn expansion_only(value: &str) -> bool {
    let (mut start, mut at, mut quote) = (0, 0, None);
    let mut leading = true;
    while at < value.len() {
        let c = value[at..].chars().next().unwrap();
        if matches!(c, '\'' | '"') {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
            at += 1;
        } else if let Some((end, _)) = expansion_span(&value[at..]) {
            at += end;
        } else if quote.is_none() && (c.is_whitespace() || matches!(c, ';' | '&' | '|')) {
            let word = &value[start..at];
            if !word.is_empty() {
                if !(leading && assignment_word(word)) {
                    leading = false;
                    if !empty_word(word) {
                        return false;
                    }
                }
            }
            if matches!(c, ';' | '&' | '|' | '\n') {
                leading = true;
            }
            at += c.len_utf8();
            start = at;
        } else {
            at += c.len_utf8();
        }
    }
    quote.is_none()
        && (start == at
            || (leading && assignment_word(&value[start..]))
            || empty_word(&value[start..]))
}

fn parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn assignment_word(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| parameter_name(name))
}

/// End offset and body of a balanced braced/command/backtick expansion.
/// Keep nested substitutions opaque when splitting words and command lists.
fn expansion_span(value: &str) -> Option<(usize, &str)> {
    let (open, close) = if value.starts_with("${") {
        (2, '}')
    } else if value.starts_with("$(") {
        (2, ')')
    } else if value.starts_with('`') {
        (1, '`')
    } else {
        return None;
    };
    let (mut at, mut quote) = (open, None);
    while at < value.len() {
        let c = value[at..].chars().next()?;
        if c == close && quote.is_none() {
            return Some((at + 1, &value[open..at]));
        }
        if matches!(c, '\'' | '"') {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
        } else if let Some((end, _)) = expansion_span(&value[at..]) {
            at += end;
            continue;
        }
        at += c.len_utf8();
    }
    None
}

fn empty_word(mut word: &str) -> bool {
    while let Some(c) = word.chars().next() {
        if matches!(c, '\'' | '"') {
            word = &word[1..];
            continue;
        }
        if let Some((end, body)) = expansion_span(word) {
            let empty = if word.starts_with("${") {
                let name_end = body
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(body.len());
                let name = &body[..name_end];
                let operator = &body[name_end..];
                if operator.is_empty() {
                    parameter_name(name)
                        || (!name.is_empty() && name.chars().all(|c| c.is_ascii_digit()))
                } else if parameter_name(name) {
                    if operator.starts_with(":+") || operator.starts_with('+') {
                        true
                    } else if let Some(operand) = operator
                        .strip_prefix(":-")
                        .or_else(|| operator.strip_prefix('-'))
                    {
                        empty_word(operand)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                expansion_only(body)
            };
            if !empty {
                return false;
            }
            word = &word[end..];
        } else if let Some(after) = word.strip_prefix('$') {
            let length = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            if !parameter_name(&after[..length]) {
                return false;
            }
            word = &after[length..];
        } else {
            return false;
        }
    }
    true
}

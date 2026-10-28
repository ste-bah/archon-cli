use super::*;

pub(super) fn source(path: &Path, text: &str) -> Source {
    let words = shell_words(text);
    let all_names_count = words.iter().enumerate().any(|(at, (_, word, _))| {
        matches!(
            word.as_str(),
            "export" | "declare" | "typeset" | "readonly" | "local" | "env"
        ) || word == "set" && has_set_option(&words[at + 1..])
    });
    let mut assigned = BTreeSet::new();
    if !all_names_count {
        for statement in words.split(|(raw, _, _)| raw == ";") {
            let mut at = 0;
            while statement.get(at).is_some_and(|(raw, _, _)| {
                matches!(raw.as_str(), "then" | "do" | "if" | "while" | "until" | "!")
            }) {
                at += 1;
            }
            let mut keys = Vec::new();
            while let Some((raw, _, _)) = statement.get(at) {
                let Some(key) = assignment_key(raw) else {
                    break;
                };
                keys.push(key.to_owned());
                at += 1;
            }
            // Only a standalone assignment assigns the script's variable.
            // An assignment before a command belongs to that command's environment.
            if at == statement.len() {
                assigned.extend(keys);
            } else if statement[at].0 == "for" {
                if let Some((key, _, _)) = statement.get(at + 1) {
                    assigned.insert(key.clone());
                }
            }
        }
    }
    let name = regex::Regex::new(r"\bARCHON_[A-Z0-9_]+").unwrap();
    let tokens = text
        .lines()
        .map(|line| {
            let line = name.replace_all(line, |capture: &regex::Captures<'_>| {
                if assigned.contains(&capture[0]) {
                    String::new()
                } else {
                    capture[0].to_owned()
                }
            });
            Tok::Str(line.into_owned())
        })
        .collect();
    Source {
        path: path.to_owned(),
        tokens,
        lines: (1..=text.lines().count()).collect(),
    }
}

fn has_set_option(words: &[(String, String, usize)]) -> bool {
    words
        .iter()
        .take_while(|(raw, _, _)| raw != ";")
        .any(|(_, word, _)| word.starts_with(['-', '+']))
}

fn assignment_key(word: &str) -> Option<&str> {
    let (key, _) = word.split_once('=')?;
    shell_identifier(key).then_some(key)
}

fn shell_identifier(word: &str) -> bool {
    let mut bytes = word.bytes();
    bytes
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_')
        && bytes.all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
}

// Words identify conservative export triggers, not particular command shapes.
// Quoted heredoc bodies and comments cannot supply triggers or local assignments.
// Their text is still scanned for names by source().
fn shell_words(text: &str) -> Vec<(String, String, usize)> {
    let mut words = Vec::new();
    let mut raw = String::new();
    let mut word = String::new();
    let mut chars = text.chars().peekable();
    let (mut line, mut start, mut quote) = (1, 1, None);
    let mut heredocs = std::collections::VecDeque::new();
    while let Some(ch) = chars.next() {
        // A continuation joins words before shell tokenization, except in single quotes.
        if ch == '\\' && quote != Some('\'') && chars.peek() == Some(&'\n') {
            chars.next();
            line += 1;
            continue;
        }
        if raw.is_empty() {
            start = line;
        }
        if quote.is_none() && ch == '<' && chars.peek() == Some(&'<') {
            push_word(&mut words, &mut raw, &mut word, start);
            chars.next();
            if chars.peek() == Some(&'<') {
                chars.next(); // A here-string has no following body.
                continue;
            }
            let tabs = chars.peek() == Some(&'-');
            if tabs {
                chars.next();
            }
            while chars.peek().is_some_and(|ch| matches!(ch, ' ' | '\t')) {
                chars.next();
            }
            let (mut delimiter, mut quoted, mut delimiter_quote) = (String::new(), false, None);
            while let Some(&ch) = chars.peek() {
                if delimiter_quote.is_none() && (ch.is_whitespace() || ";|&()< >".contains(ch)) {
                    break;
                }
                chars.next();
                match (ch, delimiter_quote) {
                    ('\\', None | Some('"')) => {
                        quoted = true;
                        if let Some(ch) = chars.next() {
                            delimiter.push(ch);
                        }
                    }
                    ('\'' | '"', None) => {
                        quoted = true;
                        delimiter_quote = Some(ch);
                    }
                    (_, Some(end)) if ch == end => delimiter_quote = None,
                    _ => delimiter.push(ch),
                }
            }
            if !delimiter.is_empty() {
                heredocs.push_back((delimiter, tabs, quoted));
            }
        } else if quote.is_none() && (ch.is_whitespace() || ";|&()".contains(ch)) {
            push_word(&mut words, &mut raw, &mut word, start);
            if ch == '\n' || ";|&()".contains(ch) {
                words.push((";".into(), String::new(), line));
            }
        } else if ch == '#' && quote.is_none() && raw.is_empty() {
            while chars.peek().is_some_and(|ch| *ch != '\n') {
                chars.next();
            }
        } else {
            raw.push(ch);
            match (ch, quote) {
                ('\\', None | Some('"')) => {
                    if let Some(escaped) = chars.next() {
                        raw.push(escaped);
                        word.push(escaped);
                        if escaped == '\n' {
                            line += 1;
                        }
                    }
                }
                ('\'' | '"', None) => quote = Some(ch),
                (_, Some(delimiter)) if ch == delimiter => quote = None,
                _ => word.push(ch),
            }
        }
        if ch == '\n' {
            line += 1;
            if quote.is_none() {
                while let Some((delimiter, tabs, quoted)) = heredocs.pop_front() {
                    while chars.peek().is_some() {
                        let body = chars
                            .by_ref()
                            .take_while(|ch| *ch != '\n')
                            .collect::<String>();
                        let body_line = line;
                        line += 1;
                        if (if tabs {
                            body.trim_start_matches('\t')
                        } else {
                            &body
                        }) == delimiter
                        {
                            break;
                        }
                        if !quoted {
                            // Unquoted bodies may supply triggers, never shell-local assignments.
                            words.extend(shell_words(&body).into_iter().map(
                                |(_, word, offset)| (String::new(), word, body_line + offset - 1),
                            ));
                        }
                    }
                }
            }
        }
    }
    push_word(&mut words, &mut raw, &mut word, start);
    words
}

fn push_word(
    words: &mut Vec<(String, String, usize)>,
    raw: &mut String,
    word: &mut String,
    line: usize,
) {
    if !raw.is_empty() {
        words.push((std::mem::take(raw), std::mem::take(word), line));
    }
}

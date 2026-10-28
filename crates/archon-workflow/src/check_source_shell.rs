//! The shell reading a check command needs (PLAN-11,
//! [`super`]): words and operators, simple commands, wrappers and
//! here-document bodies set aside.

/// `command` without its here-document bodies: a body is the command's
/// stdin, written inline in the frozen contract itself, never a repository
/// source, and must not be read as commands.
/// Whether `command` feeds a here-document to a program: inline logic.
pub(super) fn has_heredoc(command: &str) -> bool {
    stripped(command).1
}

pub(crate) fn without_heredocs(command: &str) -> String {
    stripped(command).0
}

fn stripped(command: &str) -> (String, bool) {
    stripped_with_programs(command, None)
}

/// Preserve each here-document's program at its command's position for path scanning.
pub(crate) fn with_heredoc_programs(command: &str) -> (String, String) {
    // Compare lexer output so quoting and continuations cannot assemble the
    // synthetic word from an actual argument.
    let words = words(command);
    let mut flag = "--stdin-program".to_owned();
    while words.contains(&flag) {
        flag.push('_');
    }
    (stripped_with_programs(command, Some(&flag)).0, flag)
}

fn stripped_with_programs(command: &str, program_flag: Option<&str>) -> (String, bool) {
    let command = without_continuations(command);
    let mut any = false;
    let lines: Vec<&str> = command.lines().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let Some(at) = line.find("<<").filter(|at| !line[*at..].starts_with("<<<")) else {
            out.push(line.to_string());
            continue;
        };
        let rest = line[at + 2..].trim_start_matches('-').trim_start();
        let delimiter: String = rest
            .trim_start_matches(['\'', '"'])
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // Only a delimiter that starts like a word and closes a later line
        // opens a body: `x << 2` inside an inline program is not one.
        let close = (!delimiter.is_empty() && !delimiter.starts_with(|c: char| c.is_ascii_digit()))
            .then(|| (index..lines.len()).find(|at| lines[*at].trim() == delimiter))
            .flatten();
        match close {
            Some(close) => {
                out.push(if let Some(flag) = program_flag {
                    let body = lines[index..close].join("\n").replace('\'', "'\\''");
                    // Preserve stdin origin; inline flags and heredoc receivers
                    // have different rules for whether their text is executable.
                    format!("{} {flag} '{body}'", &line[..at])
                } else {
                    line[..at].to_string()
                });
                index = close + 1;
                any = true;
            }
            None => out.push(line.to_string()),
        }
    }
    (out.join("\n"), any)
}

/// Shell words, quotes removed; operators as their own words.
pub(crate) fn words(command: &str) -> Vec<String> {
    words_with_certainty(command)
        .into_iter()
        .map(|(word, _, _)| word)
        .collect()
}

/// Flags preserve literal certainty and shell-operator identity independently.
/// Quoted or escaped operator spellings are ordinary words.
pub(crate) fn words_with_certainty(command: &str) -> Vec<(String, bool, bool)> {
    let command = without_heredocs(command);
    let mut out = Vec::new();
    let mut word = String::new();
    let mut literal = true;
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                let escaped = chars.next();
                literal &= escaped.is_some_and(|c| matches!(c, '$' | '`' | '"' | '\\'));
                word.extend(escaped);
            }
            Some('"') if matches!(c, '$' | '`') => {
                literal = false;
                word.push(c);
            }
            Some(_) => word.push(c),
            None if c == '\\' => match chars.next() {
                Some(c) => word.push(c),
                None => literal = false,
            },
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '\n' => {
                flush(&mut word, &mut literal, &mut out);
                out.push((";".into(), true, true));
            }
            None if c.is_whitespace() => flush(&mut word, &mut literal, &mut out),
            None if matches!(c, '|' | '&' | ';' | '(' | ')' | '>' | '<') => {
                flush(&mut word, &mut literal, &mut out);
                let mut op = c.to_string();
                if let Some(&next) = chars.peek()
                    && matches!(c, '|' | '&' | '>')
                    && next == c
                {
                    op.push(next);
                    chars.next();
                }
                out.push((op, true, true));
            }
            None if matches!(c, '$' | '`' | '*' | '?')
                || (c == '['
                    && chars
                        .clone()
                        .take_while(|c| !c.is_whitespace())
                        .any(|c| c == ']')) =>
            {
                literal = false;
                word.push(c);
            }
            None => word.push(c),
        }
    }
    literal &= quote.is_none();
    flush(&mut word, &mut literal, &mut out);
    out
}

fn flush(word: &mut String, literal: &mut bool, out: &mut Vec<(String, bool, bool)>) {
    if !word.is_empty() {
        out.push((std::mem::take(word), *literal, false));
    }
    *literal = true;
}

/// POSIX removes escaped newlines before recognizing words, except in single quotes.
fn without_continuations(command: &str) -> String {
    let mut out = String::new();
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && quote != Some('\'') {
            if chars.peek() == Some(&'\n') {
                chars.next();
            } else {
                out.push(c);
                out.extend(chars.next());
            }
            continue;
        }
        if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        }
        out.push(c);
    }
    out
}

/// Simple commands, split on control operators, redirections removed.
pub(super) fn segments(words: &[String]) -> Vec<Vec<String>> {
    let mut out = vec![Vec::new()];
    let mut redirect = false;
    for word in words {
        if std::mem::take(&mut redirect) {
            continue;
        }
        match word.as_str() {
            "&&" | "||" | ";" | "|" | "&" | "(" | ")" => out.push(Vec::new()),
            ">" | ">>" | "<" => redirect = true,
            _ if word.starts_with("2>") || word.starts_with("1>") => {}
            _ => out.last_mut().expect("never empty").push(word.clone()),
        }
    }
    out.retain(|segment| !segment.is_empty());
    out
}

/// A simple command without its environment assignments and the wrappers
/// that only run the rest (`env`, `timeout N`, `time`, `nice`, `exec`,
/// `command`).
pub(crate) fn strip_wrappers(segment: &[String]) -> Vec<String> {
    let mut rest = segment;
    loop {
        match rest.first().map(String::as_str) {
            Some(word) if is_assignment(word) => rest = &rest[1..],
            Some("env" | "time" | "nice" | "exec" | "command" | "nohup") => rest = &rest[1..],
            Some("timeout") => {
                rest = &rest[1..];
                while rest.first().is_some_and(|w| w.starts_with('-')) {
                    rest = &rest[1..];
                }
                rest = rest.get(1..).unwrap_or_default();
            }
            _ => return rest.to_vec(),
        }
    }
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

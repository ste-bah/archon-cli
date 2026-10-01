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

fn without_heredocs(command: &str) -> String {
    stripped(command).0
}

fn stripped(command: &str) -> (String, bool) {
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
                out.push(line[..at].to_string());
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
    let command = without_heredocs(command);
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => word.extend(chars.next()),
            Some(_) => word.push(c),
            None if c == '\\' => word.extend(chars.next()),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c == '\n' => {
                flush(&mut word, &mut out);
                out.push(";".into());
            }
            None if c.is_whitespace() => flush(&mut word, &mut out),
            None if matches!(c, '|' | '&' | ';' | '(' | ')' | '>' | '<') => {
                flush(&mut word, &mut out);
                let mut op = c.to_string();
                if let Some(&next) = chars.peek()
                    && matches!(c, '|' | '&' | '>')
                    && next == c
                {
                    op.push(next);
                    chars.next();
                }
                out.push(op);
            }
            None => word.push(c),
        }
    }
    flush(&mut word, &mut out);
    out
}

fn flush(word: &mut String, out: &mut Vec<String>) {
    if !word.is_empty() {
        out.push(std::mem::take(word));
    }
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
pub(super) fn strip_wrappers(segment: &[String]) -> Vec<String> {
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

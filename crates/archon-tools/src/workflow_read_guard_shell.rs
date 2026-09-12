//! Recognise executable positions, not words mentioned by echo, comments or
//! quoted script text. This is a workflow efficiency guard, not a shell sandbox.

/// A small quote-aware lexer for ordinary shell command lists. Dynamic
/// substitutions and redirects are deliberately not called read-only.
fn commands(text: &str, allow_redirects: bool) -> Option<Vec<Vec<String>>> {
    let mut commands = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for ch in text.chars() {
        if comment && ch != '\n' {
            continue;
        }
        if ch == '\n' {
            comment = false;
        }
        if escaped {
            word.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                if q == '"' && matches!(ch, '$' | '`') {
                    return None;
                }
                word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '#' if word.is_empty() => comment = true,
            '<' | '>' if allow_redirects => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            '$' | '`' | '<' | '>' | '(' | ')' | '{' | '}' => return None,
            ';' | '|' | '&' | '\n' => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
            }
            c if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(ch),
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    if !words.is_empty() {
        commands.push(words);
    }
    Some(commands)
}

fn program(words: &[String]) -> (&str, &[String]) {
    let mut i = 0;
    while words
        .get(i)
        .is_some_and(|w| w.contains('=') && !w.starts_with('-'))
    {
        i += 1;
    }
    if words
        .get(i)
        .is_some_and(|w| matches!(w.as_str(), "env" | "command"))
    {
        i += 1;
        while words.get(i).is_some_and(|w| w.contains('=') || w == "--") {
            i += 1;
        }
    }
    let Some(name) = words.get(i) else {
        return ("", &[]);
    };
    (name.rsplit('/').next().unwrap_or(name), &words[i + 1..])
}

pub(super) fn inspection(command: &str) -> bool {
    let Some(commands) = commands(command, false) else {
        return false;
    };
    let mut read = false;
    for words in &commands {
        let (name, args) = program(words);
        match name {
            "cat" | "head" | "tail" | "ls" | "grep" | "rg" | "wc" | "pwd" => read = true,
            "sed"
                if args.iter().any(|a| a == "-n")
                    && !args.iter().any(|a| a.starts_with("-i"))
                    && args
                        .iter()
                        .filter(|a| !a.starts_with('-'))
                        .next()
                        .is_some_and(|s| {
                            s.chars()
                                .all(|c| c.is_ascii_digit() || matches!(c, ',' | 'p' | ';' | ' '))
                        }) =>
            {
                read = true
            }
            "git"
                if matches!(
                    args.first().map(String::as_str),
                    Some("status" | "diff" | "show" | "log" | "ls-files")
                ) && !args.iter().any(|a| {
                    a.starts_with("--output") || a == "--ext-diff" || a == "--textconv"
                }) =>
            {
                read = true
            }
            "cd" if args.len() <= 2 => {}
            _ => return false,
        }
    }
    read
}

pub(super) fn release_build(command: &str) -> bool {
    let Some(commands) = commands(command, true) else {
        return false;
    };
    commands.iter().any(|words| {
        let (name, args) = program(words);
        if name != "cargo" {
            return false;
        }
        let args = if args.first().is_some_and(|a| a.starts_with('+')) {
            &args[1..]
        } else {
            args
        };
        let build = args.iter().position(|a| a == "build" || a == "b");
        let Some(build) = build else {
            return false;
        };
        let args = &args[build + 1..];
        args.iter()
            .any(|a| matches!(a.as_str(), "--release" | "-r" | "--profile=release"))
            || args
                .windows(2)
                .any(|a| a[0] == "--profile" && a[1] == "release")
    })
}

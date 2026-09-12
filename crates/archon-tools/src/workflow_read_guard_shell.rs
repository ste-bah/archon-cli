//! Recognise executable positions, not words mentioned by echo, comments or
//! quoted script text. This is a workflow efficiency guard, not a shell sandbox.

/// Lex executable segments separately: a variable in a later echo must not
/// hide an earlier inspection/build. Shell expansion is not evaluated here.
fn commands(text: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
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
                word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '#' if word.is_empty() => comment = true,
            '<' | '>' => {
                let fd = if !word.is_empty() && word.chars().all(|c| c.is_ascii_digit()) {
                    std::mem::take(&mut word)
                } else {
                    if !word.is_empty() {
                        words.push(std::mem::take(&mut word));
                    }
                    String::new()
                };
                let mut op = format!("{fd}{ch}");
                if chars.peek() == Some(&ch) {
                    op.push(chars.next().unwrap());
                }
                if chars.peek() == Some(&'&') {
                    op.push(chars.next().unwrap());
                }
                words.push(op);
            }
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
    if !word.is_empty() {
        words.push(word);
    }
    if !words.is_empty() {
        commands.push(words);
    }
    commands
}

/// Keep stderr diagnostics/redirection from disguising an inspection. A stdout
/// file creation (cat > deliverable), in contrast, is a write and is not counted.
fn inspection_words(words: &[String]) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if word.contains('>') || word.contains('<') {
            let destination = words.get(i + 1)?;
            if !(word.starts_with("2>") || (word == ">" && destination == "/dev/null")) {
                return None;
            }
            i += 2;
        } else {
            result.push(word.clone());
            i += 1;
        }
    }
    Some(result)
}

fn git_subcommand(mut args: &[String]) -> Option<&str> {
    while let Some(arg) = args.first() {
        if matches!(arg.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
            args = args.get(2..)?;
        } else if arg.starts_with('-') {
            args = &args[1..];
        } else {
            return Some(arg);
        }
    }
    None
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
    let commands = commands(command);
    let mut read = false;
    for words in &commands {
        let Some(words) = inspection_words(words) else {
            return false;
        };
        let (name, args) = program(&words);
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
                    git_subcommand(args),
                    Some("status" | "diff" | "show" | "log" | "ls-files")
                ) && !args.iter().any(|a| {
                    a.starts_with("--output") || a == "--ext-diff" || a == "--textconv"
                }) =>
            {
                read = true
            }
            "cd" if args.len() <= 2 => {}
            "true" | "false" | ":" => {}
            "echo" | "printf"
                if !args
                    .iter()
                    .any(|arg| arg.contains("$(") || arg.contains('`')) => {}
            _ => return false,
        }
    }
    read
}

pub(super) fn release_build(command: &str) -> bool {
    let commands = commands(command);
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

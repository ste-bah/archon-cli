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

fn git_subcommand(args: &[String]) -> Option<&str> {
    git_command(args).map(|(sub, _)| sub)
}

/// The git subcommand and its own arguments, past any global `-C x`/`-c k=v` options.
fn git_command(mut args: &[String]) -> Option<(&str, &[String])> {
    while let Some(arg) = args.first() {
        if matches!(arg.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
            args = args.get(2..)?;
        } else if arg.starts_with('-') {
            args = &args[1..];
        } else {
            return Some((arg, &args[1..]));
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
            "" if args.is_empty() => {}
            "export" | "unset" | "local" => {}
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

/// The history/worktree-mutating git verb in any executable segment ("stash pop",
/// "reset --hard", "checkout"). Read-only git — status, diff, log, show, stash
/// list/show, branch and remote without mutating flags, config --get/--list — is None.
pub(super) fn git_mutation(command: &str) -> Option<String> {
    commands(command).iter().find_map(|words| {
        let (name, args) = program(words);
        if name != "git" { return None; }
        let (sub, rest) = git_command(args)?;
        let first = rest.iter().find(|a| !a.starts_with('-')).map(String::as_str);
        let flag = |f: &dyn Fn(&str) -> bool| rest.iter().find(|a| f(a.as_str())).map(|a| format!("{sub} {a}"));
        match sub {
            "stash" if !matches!(first, Some("list" | "show")) => Some(first.map_or(sub.into(), |f| format!("{sub} {f}"))),
            "reset" => Some(flag(&|a| matches!(a, "--hard" | "--soft" | "--mixed" | "--merge" | "--keep")).unwrap_or(sub.into())),
            "branch" => flag(&|a| matches!(a, "-d" | "-D" | "-m" | "-M" | "-c" | "-C" | "-f" | "--delete" | "--move" | "--copy" | "--force" | "--unset-upstream") || a.starts_with("--set-upstream-to")),
            "config" if !rest.iter().any(|a| a.starts_with("--get") || matches!(a.as_str(), "-l" | "--list")) => Some(sub.into()),
            "remote" if !matches!(first, None | Some("show" | "get-url")) => Some(format!("{sub} {}", first.unwrap())),
            "reflog" if matches!(first, Some("expire" | "delete")) => Some(format!("{sub} {}", first.unwrap())),
            "checkout" | "switch" | "restore" | "rebase" | "merge" | "cherry-pick" | "revert" | "clean" | "commit"
            | "am" | "apply" | "push" | "pull" | "fetch" | "worktree" | "tag" | "submodule" | "mv" | "rm" | "add"
            | "notes" | "filter-branch" | "replace" | "update-ref" | "symbolic-ref" | "gc" | "prune" => Some(sub.into()),
            _ => None,
        }
    })
}

/// Conservative progress escape: opaque scripts may write, and must stay runnable.
/// The fallback broadens inspection recognition without blocking a corrective command.
pub(super) fn fallback_inspection(command: &str) -> bool {
    if inspection(command) { return true; }
    let commands = commands(command);
    let mut inspection_seen = false;
    for words in &commands {
        if words.iter().any(|w| w.contains("$(") || w.contains('`')) { return false; }
        let Some(words) = inspection_words(words) else { return false; };
        let (name, args) = program(&words);
        match name {
            "" | "export" | "unset" | "local" | "cd" | "true" | "false" | ":" | "echo" | "printf" => {}
            "find" if !args.iter().any(|a| matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete" | "-fprint" | "-fprint0" | "-fprintf" | "-fls")) => inspection_seen = true,
            "cat" | "head" | "tail" | "ls" | "grep" | "rg" | "wc" | "pwd" | "stat" | "file" | "du" | "df" | "which" | "whereis" | "tree" | "readlink" | "realpath" => inspection_seen = true,
            "git" if matches!(git_subcommand(args), Some("status" | "diff" | "show" | "log" | "ls-files" | "ls-tree" | "rev-parse")) && !args.iter().any(|a| a.starts_with("--output") || a == "--ext-diff" || a == "--textconv") => inspection_seen = true,
            // Builds, tests, editors, interpreters and unknown programs may mutate files.
            _ => return false,
        }
    }
    inspection_seen
}

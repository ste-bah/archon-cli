//! Recognise executable positions, not words mentioned by echo, comments or
//! quoted script text. This is a workflow efficiency guard, not a shell sandbox.

/// Lex executable segments separately: a variable in a later echo must not
/// hide an earlier inspection/build. Shell expansion is not evaluated here.
pub(super) fn commands(text: &str) -> Vec<Vec<String>> {
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

/// Scratch destinations: capturing output there is still reading it
/// (`cmd > /tmp/x; cat /tmp/x`). Literal tokens only; `$TMPDIR` is not expanded.
fn temp_destination(path: &str) -> bool {
    ["/tmp/", "/private/tmp/", "/var/folders/", "/private/var/folders/", "/dev/", "$TMPDIR", "${TMPDIR"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Keep stderr diagnostics and scratch/`/dev/null` redirection from disguising an
/// inspection. A stdout file creation elsewhere (cat > deliverable), in contrast,
/// is a write and is not counted.
fn inspection_words(words: &[String]) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if word.contains('>') || word.contains('<') {
            let destination = words.get(i + 1)?;
            // `< file` only feeds stdin; `<(cmd)` runs an opaque command and stays refused.
            let stdin = matches!(word.as_str(), "<" | "0<") && !destination.starts_with('(');
            let scratch = (word.starts_with('>') || word.starts_with("1>")) && temp_destination(destination);
            if !(stdin || scratch || word.starts_with("2>")) {
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

pub(super) fn program(words: &[String]) -> (&str, &[String]) {
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

/// sed's `w file`/`W file` command: a `w` that follows an address or `s///`
/// delimiter, not a letter inside a pattern or replacement (`s/new /old/`).
fn sed_writes(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        matches!(c, 'w' | 'W')
            && matches!(chars.get(i + 1), Some(' ' | '/'))
            && !chars.get(i.wrapping_sub(1)).is_some_and(|p| p.is_ascii_alphabetic())
    })
}

/// Read-only git for the read budget: not a mutating verb (the predicate the
/// refusal uses, so the two never disagree), a known read subcommand, and no
/// diff/show/log flag that writes a file or runs an external program.
fn git_read(sub: &str, rest: &[String]) -> bool {
    git_mutating_verb(sub, rest).is_none()
        && matches!(sub, "status" | "diff" | "show" | "log" | "ls-files" | "ls-tree" | "rev-parse" | "rev-list" | "describe"
            | "blame" | "cat-file" | "name-rev" | "shortlog" | "for-each-ref" | "grep" | "stash" | "branch" | "remote" | "config" | "worktree")
        && !rest.iter().any(|a| a.starts_with("--output") || a == "--ext-diff" || a == "--textconv")
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
            "cat" | "head" | "tail" | "ls" | "grep" | "rg" | "wc" | "pwd" | "cut" | "uniq" | "tr" | "stat" | "diff" | "cmp"
            | "file" | "du" | "df" | "which" | "type" | "date" | "basename" | "dirname" | "realpath" | "readlink" | "ps"
            | "pgrep" | "printenv" | "nl" | "column" | "jq" | "xxd" | "od" | "strings" | "tree" => read = true,
            "sort" if !args.iter().any(|a| a == "-o" || a.starts_with("--output")) => read = true,
            // `print >` / `-i inplace` / `system()` inside the program text write or run something.
            "awk"
                if !args.iter().any(|a| {
                    a.contains('>') || a.starts_with("-i") || a == "--inplace" || a.contains("system(")
                }) =>
            {
                read = true
            }
            "find"
                if !args.iter().any(|a| {
                    matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete" | "-fprint" | "-fprint0" | "-fprintf" | "-fls")
                }) =>
            {
                read = true
            }
            // Any `-n` script that never writes is a read, including `$(...)`-computed
            // ranges and `/pat/,/pat/p`. sed without `-n` is also a read but stays
            // unclassified to keep prior behaviour; `-f` script files are opaque.
            "sed"
                if args.iter().any(|a| {
                    matches!(a.as_str(), "--quiet" | "--silent")
                        || (a.starts_with('-') && !a.starts_with("--") && a[1..].chars().all(|c| c.is_ascii_alphabetic()) && a.contains('n'))
                }) && !args.iter().any(|a| a.starts_with("-i") || a == "--in-place" || a == "-f" || a.starts_with("--file"))
                    && !args.iter().filter(|a| !a.starts_with('-')).any(|a| sed_writes(a)) =>
            {
                read = true
            }
            "git" if git_command(args).is_some_and(|(sub, rest)| git_read(sub, rest)) => read = true,
            // Bare `env` prints the environment; `program()` consumed it as a prefix.
            "" if args.is_empty() => read |= words.last().is_some_and(|w| w == "env"),
            "export" | "unset" | "local" => {}
            "cd" if args.len() <= 2 => {}
            "true" | "false" | ":" => {}
            "echo" | "printf"
                if !args
                    .iter()
                    .any(|arg| arg.contains("$(") || arg.contains('`')) => {}
            // Interpreters, builds, `sh -c`, `xargs`, file mutators and unknown programs are opaque.
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
        git_mutating_verb(sub, rest)
    })
}

/// The mutating verb of one git statement; shared by `git_mutation` and `inspection`.
fn git_mutating_verb(sub: &str, rest: &[String]) -> Option<String> {
    let first = rest.iter().find(|a| !a.starts_with('-')).map(String::as_str);
    let flag = |f: &dyn Fn(&str) -> bool| rest.iter().find(|a| f(a.as_str())).map(|a| format!("{sub} {a}"));
    match sub {
        "stash" if !matches!(first, Some("list" | "show")) => Some(first.map_or(sub.into(), |f| format!("{sub} {f}"))),
        // `worktree list` is read-only; add/remove/prune/move/lock/unlock/repair and bare `worktree` are not.
        "worktree" if first != Some("list") => Some(first.map_or(sub.into(), |f| format!("{sub} {f}"))),
        "reset" => Some(flag(&|a| matches!(a, "--hard" | "--soft" | "--mixed" | "--merge" | "--keep")).unwrap_or(sub.into())),
        "branch" => flag(&|a| matches!(a, "-d" | "-D" | "-m" | "-M" | "-c" | "-C" | "-f" | "--delete" | "--move" | "--copy" | "--force" | "--unset-upstream") || a.starts_with("--set-upstream-to")),
        "config" if !rest.iter().any(|a| a.starts_with("--get") || matches!(a.as_str(), "-l" | "--list")) => Some(sub.into()),
        "remote" if !matches!(first, None | Some("show" | "get-url")) => Some(format!("{sub} {}", first.unwrap())),
        "reflog" if matches!(first, Some("expire" | "delete")) => Some(format!("{sub} {}", first.unwrap())),
        "checkout" | "switch" | "restore" | "rebase" | "merge" | "cherry-pick" | "revert" | "clean" | "commit"
        | "am" | "apply" | "push" | "pull" | "fetch" | "tag" | "submodule" | "mv" | "rm" | "add"
        | "notes" | "filter-branch" | "replace" | "update-ref" | "symbolic-ref" | "gc" | "prune" => Some(sub.into()),
        _ => None,
    }
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

/// A build or test runner in any executable segment. Consulted only after every
/// declared focused test has passed and the submit grace is spent: at that point
/// another build or test run is verification the verifier owns, not progress.
pub(super) fn build_or_test(command: &str) -> bool {
    commands(command).iter().any(|words| {
        let (name, args) = program(words);
        match name {
            "cargo" | "rustc" | "npm" | "pnpm" | "yarn" | "npx" | "bun" | "pytest" | "go" | "make"
            | "cmake" | "ctest" | "mvn" | "gradle" | "gradlew" | "dotnet" | "tsc" | "jest" | "vitest"
            | "mocha" | "swift" | "xcodebuild" | "bazel" | "tox" | "nox" => true,
            "python" | "python3" => args
                .windows(2)
                .any(|a| a[0] == "-m" && matches!(a[1].as_str(), "pytest" | "unittest" | "build")),
            _ => false,
        }
    })
}

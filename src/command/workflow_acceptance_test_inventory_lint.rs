//! Pre-judge lint for an allow-list of straight-line command text.

pub(crate) fn cargo_lint_forms(text: &str) -> Vec<String> {
    // Shell word boundaries use only space and tab; CRLF and other whitespace
    // are outside this grammar, even inside comments.
    if text
        .chars()
        .any(|ch| ch.is_whitespace() && !matches!(ch, ' ' | '\t' | '\n'))
    {
        return Vec::new();
    }
    // sh never continues a comment line. Refuse these before joining any
    // non-comment continuations so they cannot swallow a following command.
    if text
        .split('\n')
        .any(|line| line.trim_start_matches([' ', '\t']).starts_with('#') && line.ends_with('\\'))
    {
        return Vec::new();
    }
    let joined = text.replace("\\\n", "");
    let lines: Vec<_> = joined
        .split('\n')
        .map(|line| line.trim_matches([' ', '\t']))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut commands = Vec::new();
    for line in &lines {
        // Only literal, unquoted words are modeled. All shell operators,
        // substitutions, redirections, globbing and escapes are outside this grammar.
        if !line.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    ' ' | '\t' | '_' | '-' | '.' | '/' | '=' | ':' | ',' | '+' | '@' | '%'
                )
        }) {
            return Vec::new();
        }
        let words: Vec<_> = line
            .split([' ', '\t'])
            .filter(|word| !word.is_empty())
            .collect();
        if words
            .iter()
            .any(|word| word.contains("PATH") || protected_assignment(word))
        {
            return Vec::new();
        }
        if words
            .iter()
            .any(|word| matches!(*word, "declare" | "typeset" | "local"))
            && words
                .iter()
                .skip(1)
                .any(|word| word.starts_with('-') && word[1..].contains('n'))
        {
            return Vec::new();
        }
        if words.first() == Some(&"set") {
            // No positional arguments, option names or disabling flags.
            if words.len() != 2
                || !words[1].starts_with('-')
                || !words[1].contains('e')
                || !words[1][1..].chars().all(|ch| "eufxvECBTh".contains(ch))
            {
                return Vec::new();
            }
            commands.push((true, String::new()));
            continue;
        }
        if words.contains(&"set") {
            return Vec::new();
        }
        let mut rest = words.as_slice();
        while let Some(word) = rest.first() {
            if assignment(word) || matches!(*word, "env" | "time" | "nohup") {
                rest = &rest[1..];
            } else {
                break;
            }
        }
        if rest.is_empty() || rest[0].starts_with('-') {
            return Vec::new();
        }
        // Reserved words and state-changing builtins matter in command
        // position. Literal operands such as `echo done` do not alter flow.
        if matches!(
            rest[0],
            "if" | "then"
                | "fi"
                | "elif"
                | "else"
                | "for"
                | "in"
                | "do"
                | "done"
                | "while"
                | "until"
                | "case"
                | "esac"
                | "select"
                | "function"
                | "trap"
                | "eval"
                | "source"
                | "."
                | "hash"
                | "alias"
                | "shopt"
                | "exit"
                | "return"
                | "exec"
                | "command"
                | "builtin"
                | "enable"
                | "PATH"
        ) {
            return Vec::new();
        }
        commands.push((false, rest.join(" ")));
    }
    let mut errexit = false;
    let mut forms = Vec::new();
    for (index, (sets_errexit, command)) in commands.iter().enumerate() {
        if *sets_errexit {
            errexit = true;
            continue;
        }
        if (errexit || index + 1 == commands.len())
            && command.split([' ', '\t']).find(|word| !word.is_empty()) == Some("cargo")
        {
            forms.extend(strict_multi_names(command));
        }
    }
    forms
}

fn protected_assignment(word: &str) -> bool {
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    name.starts_with("RUSTUP_") || matches!(name, "CARGO_HOME" | "RUSTC" | "RUSTC_WRAPPER")
}

fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    })
}

fn strict_multi_names(text: &str) -> Vec<String> {
    super::simple_commands(text)
        .into_iter()
        .filter_map(|command| {
            let program = command.program.as_deref()?;
            if program.rsplit('/').next()? != "cargo" || command.args.first()? != "test" {
                return None;
            }
            // Expansions can remove or split operands, so text alone cannot
            // establish the number of positional arguments they produce.
            if command
                .args
                .iter()
                .any(|arg| super::super::verdict_shell::expands(arg))
            {
                return None;
            }
            let mut names = 0;
            let mut skip = false;
            for arg in command.args.iter().skip(1).take_while(|arg| *arg != "--") {
                if std::mem::take(&mut skip) {
                    continue;
                }
                if super::CARGO_TEST_VALUE_OPTIONS.contains(&arg.as_str())
                    || [
                        "--config",
                        "--message-format",
                        "--lockfile-path",
                        "--artifact-dir",
                        "-Z",
                    ]
                    .contains(&arg.as_str())
                {
                    skip = true;
                } else if !arg.starts_with('-') {
                    names += 1;
                } else if !arg.contains('=')
                    && ![
                        "--lib",
                        "--bins",
                        "--tests",
                        "--examples",
                        "--benches",
                        "--all-targets",
                        "--doc",
                        "--workspace",
                        "--all",
                        "--all-features",
                        "--no-default-features",
                        "--ignore-rust-version",
                        "--no-run",
                        "--no-fail-fast",
                        "--release",
                        "-r",
                        "--frozen",
                        "--locked",
                        "--offline",
                        "--future-incompat-report",
                        "--keep-going",
                        "--timings",
                        "--quiet",
                        "-q",
                        "--verbose",
                        "-v",
                        "-vv",
                    ]
                    .contains(&arg.as_str())
                {
                    // Unknown option arity cannot establish positional names.
                    return None;
                }
            }
            (names > 1).then(|| super::rendered(program, &command.args))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::cargo_lint_forms;

    #[test]
    fn argv_lint_test_inventory_keywords_as_literal_operands() {
        // The Windows CI failure used these exact LF bytes; no host shell,
        // PATH lookup or platform-specific line ending is involved.
        assert_eq!(
            cargo_lint_forms("set -eu\ncargo test first second\necho done"),
            vec!["'cargo' 'test' 'first' 'second'"]
        );
        for word in ["done", "if", "fi", "for", "while", "eval", "exit"] {
            assert_eq!(
                cargo_lint_forms(&format!("set -eu\ncargo test first second\necho {word}")),
                vec!["'cargo' 'test' 'first' 'second'"],
                "{word} is an operand"
            );
            for prefix in ["", "env ", "time ", "nohup ", "FOO=1 "] {
                let script = format!("set -eu\ncargo test first second\n{prefix}{word} command");
                assert!(cargo_lint_forms(&script).is_empty(), "{script}");
            }
        }
    }

    #[test]
    fn refuses_comment_continuations_and_non_shell_whitespace() {
        for command in [
            "cargo test a b\n# note \\\necho done",
            "# note \\\ncargo test a b",
            "set\u{a0}-e\ncargo test a b\necho done",
            "\u{a0}set -e\ncargo test a b\necho done",
            "set -e\u{a0}\ncargo test a b\necho done",
            "cargo test a b\r\n",
            "cargo test a b\n# note \\\r\necho done",
            "cargo test a b\n# comment\r",
        ] {
            assert!(cargo_lint_forms(command).is_empty(), "{command:?}");
        }
    }

    #[test]
    fn refuses_toolchain_overrides_and_namerefs() {
        for command in [
            "declare -n P=PATH\ncargo test a b",
            "cargo test a b somePATHword",
            "declare -n P=TOOL\ncargo test a b",
            "typeset -n P=TOOL\ncargo test a b",
            "local -rn P=TOOL\ncargo test a b",
            "time declare -n P=TOOL\ncargo test a b",
        ] {
            assert!(cargo_lint_forms(command).is_empty(), "{command}");
        }
        for name in [
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_HOME",
            "CARGO_HOME",
            "RUSTC",
            "RUSTC_WRAPPER",
        ] {
            for command in [
                format!("{name}=x cargo test a b"),
                format!("export {name}=x\ncargo test a b"),
                format!("export {name}\ncargo test a b"),
                format!("env {name}=x cargo test a b"),
            ] {
                assert!(cargo_lint_forms(&command).is_empty(), "{command}");
            }
        }
    }

    #[test]
    fn refuses_every_construct_outside_the_line_grammar() {
        for command in [
            "echo `cargo test first second`",
            "cat <<EOF\nset -e\nEOF\ncargo test first second\necho done",
            "[ x ] && set -e\ncargo test first second\necho done",
            "PATH=./bin:$PATH cargo test first second",
            "PATH=./bin cargo test first second",
            "env PATH=./bin cargo test first second",
            "export PATH\ncargo test first second",
            "export PATH=./bin\ncargo test first second",
            "echo $(cargo test first second)",
            "cargo test first second ; echo done",
            "cargo test first second\necho done",
            "cargo test first second\nset -e",
            "set +e\ncargo test first second",
            "set -e extra\ncargo test first second",
            "cargo test first second | cat",
            "cargo test first second || true",
            "cargo test first second && echo done",
            "cargo test first second &",
            "! cargo test first second",
            "(cargo test first second)",
            "{ cargo test first second; }",
            "cargo test first second > output",
            "cargo test first second # inline comment",
            "env -i cargo test first second",
        ] {
            assert!(cargo_lint_forms(command).is_empty(), "{command}");
        }
        for word in [
            "if", "then", "fi", "for", "while", "case", "function", "trap", "eval", "source", ".",
            "hash", "alias",
        ] {
            for command in [
                format!("{word} command\ncargo test first second"),
                format!("set -e\ncargo test first second\n{word} command"),
            ] {
                assert!(cargo_lint_forms(&command).is_empty(), "{command}");
            }
        }
    }
}

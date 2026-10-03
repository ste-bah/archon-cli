//! What the statements before a script's final command can do to its status.

use std::collections::BTreeSet;

use super::shell_lexer::{Command, Statement};
use super::statement_fixed_label;

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Prelude {
    /// An earlier statement can end the script with a failing status: `exit`
    /// (other than `exit 0`), `return`, `logout`, `exec <program>`,
    /// `kill $$`, or a `trap` that exits.
    pub ends_script: bool,
    /// errexit was on while an earlier statement that can fail ran, so that
    /// statement's failure ends the script.
    pub errexit_failure: bool,
    /// `set -o pipefail` is in effect when the final statement runs.
    pub pipefail: bool,
}

/// Only command words that run in the script's own shell count: never words
/// in a subshell or substitution, and words in a function body only when the
/// script calls that function outside any function or subshell.
pub(super) fn prelude(earlier: &[Statement]) -> Prelude {
    let called: BTreeSet<&str> = earlier
        .iter()
        .flat_map(|statement| &statement.commands)
        .filter(|command| !command.subshell && command.function.is_none())
        .map(|command| command.name.as_str())
        .collect();
    let live = |command: &&Command| {
        !command.subshell
            && command
                .function
                .as_deref()
                .is_none_or(|function| called.contains(function))
    };
    let mut prelude = Prelude::default();
    let mut errexit = false;
    for statement in earlier {
        let errexit_before = errexit;
        for command in statement.commands.iter().filter(live) {
            prelude.ends_script |= ends_script(command);
            if command.name == "set" {
                apply_set(&command.args, &mut errexit, &mut prelude.pipefail);
            }
        }
        let only_set = !statement.commands.is_empty()
            && statement
                .commands
                .iter()
                .all(|command| command.name == "set");
        if !only_set
            && (errexit_before || errexit)
            && statement_fixed_label(statement, false, 0).is_none()
        {
            prelude.errexit_failure = true;
        }
    }
    prelude
}

fn ends_script(command: &Command) -> bool {
    let args: Vec<&str> = command
        .args
        .split_whitespace()
        .map(|arg| arg.trim_matches(['\'', '"']))
        .collect();
    let failing_code = || args.first().is_none_or(|code| *code != "0");
    match command.name.as_str() {
        "exit" | "logout" => failing_code(),
        "return" => command.function.is_none() && failing_code(),
        "exec" => args.first().is_some_and(|arg| {
            !arg.trim_start_matches(|ch: char| ch.is_ascii_digit())
                .starts_with(['<', '>'])
        }),
        "kill" => args
            .iter()
            .filter(|arg| !arg.starts_with('-'))
            .any(|arg| matches!(*arg, "$$" | "0")),
        "trap" => command
            .args
            .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .any(|word| word == "exit"),
        _ => false,
    }
}

/// Apply `set` options in order: `-e`/`+e`, `-o errexit`, `-o pipefail`.
fn apply_set(args: &str, errexit: &mut bool, pipefail: &mut bool) {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    let mut index = 0;
    while let Some(token) = tokens.get(index) {
        let on = match token.chars().next() {
            Some('-') => true,
            Some('+') => false,
            _ => break,
        };
        if matches!(*token, "-" | "--") {
            break;
        }
        let flags = &token[1..];
        if flags.contains('e') {
            *errexit = on;
        }
        if flags.contains('o') {
            match tokens.get(index + 1).copied() {
                Some("errexit") => *errexit = on,
                Some("pipefail") => *pipefail = on,
                _ => {}
            }
            index += 1;
        }
        index += 1;
    }
}

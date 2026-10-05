//! A subcommand a check's search path cannot resolve (Issues 331, 333).
//!
//! Some tools dispatch their first argument: `tool sub` runs a subcommand
//! built into `tool`, or else a program `tool-sub` found on the search path
//! (cargo, git and kubectl plugins work so). A check that runs `tool sub`
//! where its search path has neither fails whatever it asserts: the tool
//! rejects the subcommand before anything is checked. When the host itself
//! has `tool-sub` installed, that run gave no verdict -- an environment
//! failure, like a missing program (rule 1 of
//! `workflow_acceptance_executability_verdict`) -- so it is unproven, then
//! its author's finding under the same strike rule, never a proof.
//!
//! The rule is the same for every tool. `sub` is a plugin the check's
//! environment lacks when all of these hold: no program `tool-sub` is on
//! the check's search path, one is on the host's own search path, and
//! `sub` is not among the commands `tool --list` lists at the check's site
//! with no search path (`verdict_subcommand_list`). A subcommand no `tool-sub`
//! anywhere provides may be one the deliverable adds (an alias in the
//! tree's configuration, a command of the product's own command line): its
//! rejection before the implementation is a verdict. A tool whose built-in
//! commands are not known that way is never judged either.
//!
//! `sub` is the tool's first argument that is not an option -- or, since an
//! option may take a value (`tool --color never sub`), the word after such
//! a value ([`candidates`]). A failed run is an environment failure only
//! when the run's output has a line that rejects one of those words,
//! quoted, as an unknown command, that word is a plugin its environment
//! lacks, and nothing shows an assertion ran. So a check that falls back
//! when the subcommand is missing keeps its verdict.
//!
//! Before a run the host also names each program and plugin subcommand
//! missing from the configured toolchain path ([`unresolved_on_path`]). It
//! lists only tools some check runs with a word the host has a `tool-word`
//! program for, each tool once, all at once, never on an async thread.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::super::verdict_shell::{Simple, simple_commands};
use super::{ASSERTED, Context, which};

#[path = "workflow_acceptance_executability_verdict_subcommand_list.rs"]
mod list;
pub(super) use list::LIST_STALL;
use list::{Listing, listing, prefetch};

/// A line that rejects a command: `no such command`, `unknown subcommand`,
/// `'x' is not a git command`.
static REJECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:no such|unknown|unrecognized|invalid)\s+(?:sub)?command\b|\bis not an?\s+\S+\s+(?:sub)?command\b",
    )
    .expect("static pattern")
});

#[cfg(test)]
thread_local! {
    /// A test's stand-in for the host's search path, on its own thread.
    pub(crate) static HOST_PATH: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// The host's own search path.
pub(super) fn host_path() -> Option<String> {
    #[cfg(test)]
    if let Some(path) = HOST_PATH.with(|path| path.borrow().clone()) {
        return Some(path);
    }
    std::env::var("PATH").ok()
}

/// Why the failed run printing `output` gave no verdict because a
/// subcommand one of `commands` runs is a plugin `at`'s environment lacks
/// (see the module docs); `None` otherwise.
pub(super) fn missing(commands: &[Simple], output: &str, at: &Context) -> Option<String> {
    if ASSERTED.is_match(output) {
        return None;
    }
    let rejections: Vec<&str> = output.lines().filter(|l| REJECTION.is_match(l)).collect();
    if rejections.is_empty() {
        return None;
    }
    commands.iter().find_map(|command| {
        let (tool, program) = tool(command, at)?;
        candidates(command).into_iter().find_map(|name| {
            (rejections.iter()).find(|line| quoted(line, name))?;
            let host = plugin(&tool, name, at)?;
            let Listing::Commands(builtins) = listing(&program, at) else {
                return None;
            };
            (!builtins.contains(name)).then(|| {
                format!(
                    "it runs `{tool} {name}`, but `{name}` is neither a command built into `{tool}` nor a program `{tool}-{name}` on its search path (PATH={}), though the host has one at {}: a subcommand its environment lacks",
                    at.path.as_deref().unwrap_or(""),
                    host.display()
                )
            })
        })
    })
}

/// Whether `output` rejects a quoted word as an unknown command and shows
/// no assertion ran: the text-free shape of [`missing`].
pub(super) fn rejected(output: &str) -> bool {
    !ASSERTED.is_match(output)
        && (output.lines())
            .filter(|line| REJECTION.is_match(line))
            .any(|line| line.contains(['`', '\'', '"']))
}

/// For each check text of `texts`, the commands it runs that `at`'s search
/// path does not resolve, each described for an operator: a program it
/// starts by name that is not there, one it starts by an absolute path that
/// does not exist, and a plugin subcommand that path lacks (see the module
/// docs). Blocking: it may run the tools it lists.
pub(crate) fn unresolved_on_path(texts: &[&str], at: &Context) -> Vec<Vec<String>> {
    let commands: Vec<Vec<Simple>> = texts.iter().map(|text| simple_commands(text)).collect();
    let listed: BTreeSet<PathBuf> = (commands.iter().flatten())
        .filter_map(|command| {
            let (tool, program) = tool(command, at)?;
            let names = candidates(command);
            (names.iter().any(|name| plugin(&tool, name, at).is_some())).then_some(program)
        })
        .collect();
    prefetch(&listed, at);
    commands
        .iter()
        .map(|commands| {
            let mut found = Vec::new();
            for command in commands {
                found.extend(unresolved(command, at));
            }
            found.dedup();
            found
        })
        .collect()
}

/// What `command` runs that `at`'s search path does not resolve.
fn unresolved(command: &Simple, at: &Context) -> Option<String> {
    let program = command.program.as_deref()?;
    if which::is_path(program) {
        if which::is_absolute(program) && !Path::new(program).exists() {
            return Some(format!("`{program}` (no such file)"));
        }
    } else if at.find(program).is_none() {
        return Some(format!("`{program}` (not on the path)"));
    }
    let (tool, located) = tool(command, at)?;
    let names = candidates(command);
    let on_path = |name: &str| at.on_path(&format!("{tool}-{name}"));
    let index = (names.iter()).position(|name| plugin(&tool, name, at).is_some())?;
    if names[..index].iter().any(|name| on_path(name)) {
        return None;
    }
    let first = names[index];
    let builtins = match listing(&located, at) {
        Listing::Commands(builtins) => builtins,
        Listing::Unknown(why) => {
            return Some(format!(
                "`{tool} {first}` (no `{tool}-{first}` on the path, though the host has one; whether `{first}` is built into `{tool}` is not known: {why})"
            ));
        }
    };
    for name in names {
        if builtins.contains(name) || on_path(name) {
            return None;
        }
        if let Some(host) = plugin(&tool, name, at) {
            return Some(format!(
                "`{tool} {name}` (not built into `{tool}`, and no `{tool}-{name}` on the path; the host has it at {})",
                host.display()
            ));
        }
    }
    None
}

/// The tool `command` starts, by its own name, and where it is: a name on
/// `at`'s search path, or an absolute path that exists.
fn tool(command: &Simple, at: &Context) -> Option<(String, PathBuf)> {
    let program = command.program.as_deref()?;
    if !which::is_path(program) {
        return Some((program.to_string(), at.find(program)?));
    }
    let path = Path::new(program);
    let name = path.file_name()?.to_str()?;
    let name = which::bare_name(name);
    (which::is_absolute(program) && path.is_file()).then(|| (name, path.to_path_buf()))
}

/// The words of `command` that may be its subcommand, in order: its first
/// argument that is not an option and, while the word before is an option
/// that may take a value (`--color never`, not `--color=never`), the word
/// after that value. Only words shaped like a command name are kept.
pub(super) fn candidates(command: &Simple) -> Vec<&str> {
    let mut found = Vec::new();
    let mut after_option = false;
    for word in &command.args {
        if word == "--" {
            break;
        }
        if word.len() > 1 && word.starts_with(['-', '+']) {
            after_option = !word.contains('=');
            continue;
        }
        if named(word) {
            found.push(word.as_str());
        }
        if !std::mem::take(&mut after_option) {
            break;
        }
    }
    found
}

fn named(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_alphabetic())
        && (word.chars()).all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn quoted(line: &str, name: &str) -> bool {
    ['`', '\'', '"']
        .iter()
        .any(|quote| line.contains(&format!("{quote}{name}{quote}")))
}

/// Where the host has the program `tool-name` that `at`'s search path
/// lacks; `None` when that path has it, or the host has none.
fn plugin(tool: &str, name: &str, at: &Context) -> Option<PathBuf> {
    let program = format!("{tool}-{name}");
    if at.on_path(&program) {
        return None;
    }
    which::find(at.host_path.as_deref()?, &program)
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_probe_tests.rs"]
mod probe_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_333_tests.rs"]
mod tests_333;

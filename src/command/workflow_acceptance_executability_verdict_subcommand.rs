//! A subcommand a check's search path cannot resolve (Issues 331, 333).
//!
//! Some tools dispatch their first argument: `tool sub` runs a subcommand
//! built into `tool`, or else a program `tool-sub` found on the search path
//! (cargo, git and kubectl plugins work so). A check that runs `tool sub`
//! where its search path has neither fails whatever it asserts: the tool
//! rejects the subcommand before anything is checked. Such a run gave no
//! verdict, like one whose program is missing (rule 1 of
//! `workflow_acceptance_executability_verdict`): it is unproven, then its
//! author's finding under the same strike rule, never a can-fail proof --
//! a check that cannot pass after any implementation must go back to its
//! author, and a false proof never does.
//!
//! The rule is the same for every tool. A failed run gave no verdict when
//! its output has a line that rejects, quoted, as an unknown command, a
//! word that may be the subcommand ([`candidates`]), nothing shows an
//! assertion ran, no program `tool-word` is on the check's search path,
//! and one of these holds:
//!
//! - the host's own search path has `tool-word`: a plugin the check's
//!   environment lacks. Only `tool --list` at the check's site
//!   (`verdict_subcommand_list`) listing `word` -- built in after all --
//!   turns it back into a verdict;
//! - the listing names the tool's commands and `word` is not one, and no
//!   `tool-word` is anywhere: it passes only if the deliverable adds it
//!   (an alias in the tree's configuration). Its author makes the check
//!   show that first -- for example `tool --list | grep -qw word && tool
//!   word ...` -- so its failure before the implementation is its own
//!   assertion, which is a verdict;
//! - the tool gave no listing at all (it stalled, or could not start): the
//!   host could not tell.
//!
//! A tool that answers its listing without listing any command (it has no
//! `--list`), of whose `tool-word` the host has none, is not judged: the
//! word may be a command its own deliverable adds to it (Issue 328).
//!
//! Before a run the host also names each program and subcommand missing
//! from the configured toolchain path ([`unresolved_on_path`]). It lists
//! only tools some check runs with a word no `tool-word` on that path
//! provides, of which some `tool-*` program is on that path or the host's,
//! each tool once, all at once, never on an async thread.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::super::verdict_shell::{Simple, simple_commands};
use super::{ASSERTED, Context, which};

#[path = "workflow_acceptance_executability_verdict_subcommand_list.rs"]
mod list;
#[path = "workflow_acceptance_executability_verdict_subcommand_tree.rs"]
mod tree;
pub(super) use list::LIST_STALL;
use list::{Listing, listing, prefetch};
pub(crate) use tree::SiteTree;

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
/// subcommand one of `commands` runs does not resolve at `at`'s site (see
/// the module docs); `None` otherwise.
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
            if at.on_path(&format!("{tool}-{name}")) {
                return None;
            }
            let path = at.path.as_deref().unwrap_or("");
            let lacks = format!(
                "it runs `{tool} {name}`, but no program `{tool}-{name}` is on its search path (PATH={path})"
            );
            let host = plugin(&tool, name, at);
            match (listing(&program, at), host) {
                (Listing::Commands(builtins), _) if builtins.contains(name) => None,
                (Listing::Commands(_), Some(host)) => Some(format!(
                    "{lacks} and `{name}` is not a command built into `{tool}`, though the host has one at {}: a subcommand its environment lacks",
                    host.display()
                )),
                (Listing::Unlisted(why), Some(host)) => Some(format!(
                    "{lacks}, though the host has one at {}: a subcommand its environment lacks (whether `{name}` is built into `{tool}` is not known: {why})",
                    host.display()
                )),
                (Listing::Commands(_), None) => Some(format!(
                    "{lacks}, nor on the host's, and `{name}` is not a command built into `{tool}`: it passes only if the deliverable adds it; if it does, make the check show first that the tree provides it (for example `{tool} --list | grep -qw {name} && ...`), so that it fails by its own assertion before the implementation"
                )),
                (Listing::Unlisted(_), None) => None,
                (Listing::NoAnswer(why), _) => Some(format!(
                    "{lacks}, and the host could not tell whether `{name}` is built into `{tool}`: {why}"
                )),
            }
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
/// does not exist, and a subcommand that path does not resolve (see the
/// module docs). Blocking: it may run the tools it lists.
pub(crate) fn unresolved_on_path(texts: &[&str], at: &Context) -> Vec<Vec<String>> {
    let commands: Vec<Vec<Simple>> = texts.iter().map(|text| simple_commands(text)).collect();
    let wanted: BTreeSet<PathBuf> = (commands.iter().flatten())
        .filter_map(|command| Some(judged(command, at)?.1))
        .collect();
    let listed = prefetch(&wanted, at);
    commands
        .iter()
        .map(|commands| {
            let mut found = Vec::new();
            for command in commands {
                found.extend(unresolved(command, at, &listed));
            }
            found.dedup();
            found
        })
        .collect()
}

/// The tool `command` starts and where, and its candidate subcommands,
/// when the start of a run lists it: no candidate is a `tool-word` program
/// on `at`'s path, and some `tool-*` program is on that path or the host's.
fn judged<'a>(command: &'a Simple, at: &Context) -> Option<(String, PathBuf, Vec<&'a str>)> {
    let (tool, program) = tool(command, at)?;
    let names = candidates(command);
    let resolved = |name: &&str| at.on_path(&format!("{tool}-{name}"));
    if names.is_empty() || names.iter().any(resolved) {
        return None;
    }
    let prefix = format!("{tool}-");
    let paths = [at.path.as_deref(), at.host_path.as_deref()];
    let dispatches = (paths.into_iter().flatten())
        .flat_map(|path| which::search_dirs(path, cfg!(windows)))
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix));
    dispatches.then_some((tool, program, names))
}

/// What `command` runs that `at`'s search path does not resolve, by the
/// listings `listed`.
fn unresolved(
    command: &Simple,
    at: &Context,
    listed: &std::collections::BTreeMap<PathBuf, Listing>,
) -> Option<String> {
    let program = command.program.as_deref()?;
    if which::is_path(program) {
        if which::is_absolute(program) && !Path::new(program).exists() {
            return Some(format!("`{program}` (no such file)"));
        }
    } else if at.find(program).is_none() {
        return Some(format!("`{program}` (not on the path)"));
    }
    let (tool, located, names) = judged(command, at)?;
    let shown = (names.iter())
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>();
    let which = |name: &str| match names.len() {
        1 => format!("`{tool} {name}`"),
        _ => format!(
            "`{tool} {name}` (whichever of {} is its subcommand)",
            shown.join(", ")
        ),
    };
    let no_answer = Listing::NoAnswer(format!("`{}` was not listed", located.display()));
    let listing = listed.get(&located).unwrap_or(&no_answer);
    // In order: the first word built in, or that some program provides,
    // decides; one only an option's value may be is passed over.
    for name in &names {
        let host = plugin(&tool, name, at);
        match (listing, host) {
            (Listing::Commands(builtins), _) if builtins.contains(*name) => return None,
            (Listing::Commands(_), Some(host)) => {
                return Some(format!(
                    "`{tool} {name}` (not built into `{tool}`, and no `{tool}-{name}` on the path; the host has it at {})",
                    host.display()
                ));
            }
            (Listing::Unlisted(why), Some(host)) => {
                return Some(format!(
                    "`{tool} {name}` (no `{tool}-{name}` on the path, though the host has it at {}; whether `{name}` is built into `{tool}` is not known: {why})",
                    host.display()
                ));
            }
            _ => {}
        }
    }
    let last = names[names.len() - 1];
    match listing {
        Listing::Commands(_) => Some(format!(
            "{} (not built into `{tool}`, and no `{tool}-{last}` on the path or the host's: it passes only if the deliverable adds it)",
            which(last)
        )),
        Listing::Unlisted(_) => None,
        Listing::NoAnswer(why) => Some(format!(
            "{} (the host could not tell whether it is built into `{tool}`: {why})",
            which(names[0])
        )),
    }
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

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_333b_tests.rs"]
mod tests_333b;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_333c_tests.rs"]
mod tests_333c;

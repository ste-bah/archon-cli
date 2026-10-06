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
//! word that may be the subcommand ([`candidates`]), a line that is the
//! tool's (its own line names no other program the check runs instead of
//! the tool), nothing shows an assertion ran, no program `tool-word` is on
//! the check's search path, and one of these holds:
//!
//! - the host's own search path has `tool-word`: a plugin the check's
//!   environment lacks, whatever `tool --list` says;
//! - `tool --list` (`verdict_subcommand_list`) names the tool's commands,
//!   with `word` or without it. The listing runs in a fresh directory, not
//!   the check's tree, so it never sees that tree's aliases or the
//!   toolchain it pins: listed, the site lacks what the host lists. Not
//!   listed, and no `tool-word` anywhere, it passes only if the
//!   deliverable adds it (an alias in the tree's configuration); its author
//!   makes the check show that first -- `tool --list | grep -qw word &&
//!   tool word ...` -- so its failure before the implementation is its own
//!   assertion, which is a verdict;
//! - the tool gave no answer (it stalled, never stopped, could not start
//!   or run, was killed, or failed without saying it has no `--list`): the
//!   host could not tell.
//!
//! So the listing only ever withholds a verdict, and every case where the
//! site may lack what a check runs tells its author not to depend on it
//! ([`missing`] has the whole table). A tool that says it has no listing
//! (`--list` succeeds listing nothing, or is rejected as an option), of
//! whose `tool-word` the host has none, is not judged: the word may be a
//! command its own deliverable adds to it (Issue 328).
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
/// subcommand one of `commands` runs does not resolve at `at`'s site (see
/// the module docs); `None` otherwise.
///
/// It is judged only when `output` has a line rejecting, quoted, a word
/// `tool word` may run as its subcommand, and nothing shows an assertion
/// ran. A rejection is `tool`'s unless its own line -- not the help lines
/// after it, where a tool suggests a similar command such as `test` --
/// names another program the check runs and not `tool`: `mycli: unknown
/// command 'build'` in `cargo build && mycli build` is mycli's, never
/// cargo's. A line that names no program is `tool`'s: a false proof is
/// worse than a lost verdict. Then, by the `tool --list` listing
/// (`verdict_subcommand_list`), whether the host's own search path has a
/// program `tool-word`, and whether the check's does:
///
/// | listing | host has `tool-word` | check's path has it | result |
/// |---|---|---|---|
/// | any | any | yes | verdict: the subcommand resolves, so the rejection is not for its lack; the check's own failure decides |
/// | lists `word` | any | no | no verdict: the site rejected what the host lists, so the site lacks it (a toolchain its tree pins, which the listing never reads); the check depends on what the site does not provide |
/// | lists commands without `word` | yes | no | no verdict: an environment tool the site lacks; the check must not depend on it |
/// | lists commands without `word` | no | no | no verdict: no program anywhere provides it, so it passes only if the deliverable adds it (an alias in the tree); its author guards it (`tool --list \| grep -qw word && tool word`), so that its failure before the implementation is its own assertion |
/// | lists none (it has no `--list`) | yes | no | no verdict: the environment lacks a program the host has; the check must not depend on it |
/// | lists none (it has no `--list`) | no | no | verdict (Issue 328): nothing the site could install provides `word`, so only the deliverable can add it to its own tool |
/// | no answer (it stalled, never stopped, could not run, failed) | any | no | no verdict: the host could not tell; the check must not depend on what it cannot show the site provides |
///
/// A verdict is a can-fail proof: the check is then held to fail before
/// the implementation and pass after it. Every verdict cell above is one
/// where some implementation can make it pass -- the subcommand resolves,
/// or only the deliverable can provide the word. (A rejection another
/// program's own line claims is judged as that program's, by this table.) Every cell where the site may lack what the check needs, which
/// no implementation can supply, gives no verdict: the check goes back to
/// its author, and never becomes a task that can never pass.
pub(super) fn missing(commands: &[Simple], output: &str, at: &Context) -> Option<String> {
    if ASSERTED.is_match(output) {
        return None;
    }
    let lines: Vec<&str> = output.lines().collect();
    let rejections: Vec<usize> = (0..lines.len())
        .filter(|&at| REJECTION.is_match(lines[at]))
        .collect();
    if rejections.is_empty() {
        return None;
    }
    let programs: Vec<String> = (commands.iter())
        .filter_map(|command| command.program.as_deref())
        .map(|program| at.bare_name(program.rsplit(['/', '\\']).next().unwrap_or(program)))
        .collect();
    commands.iter().find_map(|command| {
        let (tool, program) = tool(command, at)?;
        candidates(command).into_iter().find_map(|name| {
            // A rejection of `name` is `tool`'s unless its own line names
            // another program of the check and not `tool` (the quoted word
            // itself aside: `test` rejected is not the program `test`).
            let theirs = |line: &str| {
                let line = ['`', '\'', '"'].iter().fold(line.to_string(), |line, quote| {
                    line.replace(&format!("{quote}{name}{quote}"), "")
                });
                !names(&line, &tool) && (programs.iter()).any(|other| *other != tool && names(&line, other))
            };
            (rejections.iter()).find(|&&line| quoted(lines[line], name) && !theirs(lines[line]))?;
            if at.on_path(&format!("{tool}-{name}")) {
                return None;
            }
            let path = at.path.as_deref().unwrap_or("");
            let lacks = format!(
                "it runs `{tool} {name}`, but no program `{tool}-{name}` is on its search path (PATH={path})"
            );
            let depends = depends(&tool, name);
            let host = plugin(&tool, name, at);
            match (listing(&program, at), host) {
                (Listing::Commands(builtins), _) if builtins.contains(name) => Some(format!(
                    "{lacks}; the host lists `{name}` as built into `{tool}`, yet `{tool}` rejected it at the check's site, so that site lacks it (for example a toolchain its tree pins, which the host does not read). {depends}"
                )),
                (Listing::Commands(_), Some(host)) => Some(format!(
                    "{lacks} and `{name}` is not a command built into `{tool}`, though the host has one at {}: a subcommand its environment lacks. {depends}",
                    host.display()
                )),
                (Listing::Unlisted(why), Some(host)) => Some(format!(
                    "{lacks}, though the host has one at {}: a subcommand its environment lacks (whether `{name}` is built into `{tool}` is not known: {why}). {depends}",
                    host.display()
                )),
                (Listing::Commands(_), None) => Some(format!(
                    "{lacks}, nor on the host's, and `{name}` is not a command built into `{tool}`: it passes only if the deliverable adds it. {}",
                    guard(&tool, name)
                )),
                (Listing::Unlisted(_), None) => None,
                (Listing::NoAnswer(why), _) => Some(format!(
                    "{lacks}, and the host could not tell whether `{name}` is built into `{tool}`: {why}. {depends}"
                )),
            }
        })
    })
}

/// Whether `text` names `program` as a word of its own.
fn names(text: &str, program: &str) -> bool {
    let part = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    (text.match_indices(program)).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + program.len()..].chars().next();
        !before.is_some_and(part) && !after.is_some_and(part)
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
                    "`{tool} {name}` (not built into `{tool}`, and no `{tool}-{name}` on the path; the host has it at {}. {})",
                    host.display(),
                    depends(&tool, name)
                ));
            }
            (Listing::Unlisted(why), Some(host)) => {
                return Some(format!(
                    "`{tool} {name}` (no `{tool}-{name}` on the path, though the host has it at {}; whether `{name}` is built into `{tool}` is not known: {why}. {})",
                    host.display(),
                    depends(&tool, name)
                ));
            }
            _ => {}
        }
    }
    let last = names[names.len() - 1];
    match listing {
        Listing::Commands(_) => Some(format!(
            "{} (not built into `{tool}`, and no `{tool}-{last}` on the path or the host's: it passes only if the deliverable adds it. {})",
            which(last),
            guard(&tool, last)
        )),
        Listing::Unlisted(_) => None,
        Listing::NoAnswer(why) => Some(format!(
            "{} (the host could not tell whether it is built into `{tool}`: {why}. {})",
            which(names[0]),
            depends(&tool, names[0])
        )),
    }
}

/// What a check's author does when it runs `tool name`, a command the
/// site does not provide, or may not: never depend on it. No guard helps:
/// a check that shows first the command is there fails before the
/// implementation by its own assertion, and fails after it too.
fn depends(tool: &str, name: &str) -> String {
    format!(
        "The check depends on `{tool} {name}`, a program the site does not have, or may not: it must not depend on it; check the criterion with what the site provides"
    )
}

/// What a check's author does when the deliverable adds `tool name` (an
/// alias in the tree's configuration, which the host never reads); only
/// where no program anywhere provides it.
fn guard(tool: &str, name: &str) -> String {
    format!(
        "If the deliverable adds it (for example an alias in the tree's configuration), make the check show first that the tree provides it: `{tool} --list | grep -qw {name} && {tool} {name} ...`, so that it fails by its own assertion before the implementation"
    )
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
    let name = at.bare_name(name);
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

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_333d_tests.rs"]
mod tests_333d;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_333e_tests.rs"]
mod tests_333e;

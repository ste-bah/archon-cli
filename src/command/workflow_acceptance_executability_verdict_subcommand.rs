//! A subcommand a check's search path cannot resolve (Issue 331).
//!
//! Some tools dispatch their first argument: `tool sub` runs a subcommand
//! built into `tool`, or else a program `tool-sub` found on the search path
//! (cargo, git and kubectl plugins work so). A check that runs `tool sub`
//! where neither exists fails whatever it asserts: the tool rejects the
//! subcommand before anything is checked. That run gave no verdict -- an
//! environment failure, like a missing program (rule 1 of
//! `workflow_acceptance_executability_verdict`) -- so it is unproven, then
//! its author's finding under the same strike rule, never a proof.
//!
//! The rule is the same for every tool. `tool` dispatches when it has a
//! program `tool-*` on the search path and lists its built-in commands:
//! the commands `tool --list` lists when run with no search path and an
//! empty home, where no external one can be found. Its subcommand `sub`
//! (its first argument that is not an option) is missing from a search
//! path when `sub` is not built in and `tool-sub` is not a program there.
//! A tool whose built-in commands are not known that way is never judged:
//! a subcommand it rejects may be a deliverable not built yet (the product's
//! own command line), and that is a verdict.
//!
//! A failed run is an environment failure only when all of these hold:
//! `sub` is missing from the check's search path, the run's output has a
//! line that rejects `sub`, quoted, as an unknown command, and nothing
//! shows an assertion ran. So a check that falls back when the subcommand
//! is missing, or whose first argument is an option's value, keeps its
//! verdict. Before a run the host also names each program and subcommand
//! missing from the configured toolchain path ([`unresolved_on_path`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;

use super::super::verdict_shell::{Simple, simple_commands};
use super::{ASSERTED, Context};

/// The longest a tool may take to list its commands.
const LIST_TIMEOUT: Duration = Duration::from_secs(10);

/// A line that rejects a command: `no such command`, `unknown subcommand`,
/// `'x' is not a git command`.
static REJECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:no such|unknown|unrecognized|invalid)\s+(?:sub)?command\b|\bis not an?\s+\S+\s+(?:sub)?command\b",
    )
    .expect("static pattern")
});

/// Each listing tool's built-in commands, by its path.
type Listings = BTreeMap<PathBuf, Option<BTreeSet<String>>>;
static LISTED: LazyLock<Mutex<Listings>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Why the failed run printing `output` gave no verdict because a
/// subcommand one of `commands` runs is missing from `at`'s search path
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
        let name = subcommand(command)?;
        (rejections.iter()).find(|line| quoted(line, name))?;
        lacks(&tool, &program, name, at).then(|| {
            format!(
                "it runs `{tool} {name}`, but `{name}` is neither a command built into `{tool}` nor a program `{tool}-{name}` on its search path (PATH={}): a subcommand its environment lacks",
                at.path.as_deref().unwrap_or("")
            )
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

/// The commands the check text `text` runs that the search path `path`
/// does not resolve, each described for an operator: a program it starts
/// by name that is not there, one it starts by an absolute path that does
/// not exist, and a subcommand that does not resolve there, of a tool that
/// dispatches (see the module docs).
pub(crate) fn unresolved_on_path(text: &str, path: &str) -> Vec<String> {
    let at = Context {
        path: Some(path.to_string()),
        deliverables: Vec::new(),
    };
    let mut found = Vec::new();
    for command in simple_commands(text) {
        let Some(program) = command.program.as_deref() else {
            continue;
        };
        if program.contains('/') {
            if program.starts_with('/') && !Path::new(program).exists() {
                found.push(format!("`{program}` (no such file)"));
            }
        } else if at.find(program).is_none() {
            found.push(format!("`{program}` (not on the path)"));
            continue;
        }
        let Some(((tool, located), name)) = tool(&command, &at).zip(subcommand(&command)) else {
            continue;
        };
        if lacks(&tool, &located, name, &at) {
            found.push(format!(
                "`{tool} {name}` (not built into `{tool}`, and no `{tool}-{name}` on the path)"
            ));
        }
    }
    found.dedup();
    found
}

/// The tool `command` starts, by its own name, and where it is: a name on
/// `at`'s search path, or an absolute path that exists.
fn tool(command: &Simple, at: &Context) -> Option<(String, PathBuf)> {
    let program = command.program.as_deref()?;
    if !program.contains('/') {
        return Some((program.to_string(), at.find(program)?));
    }
    let path = Path::new(program);
    let name = path.file_name()?.to_str()?;
    (program.starts_with('/') && path.is_file()).then(|| (name.to_string(), path.to_path_buf()))
}

/// `command`'s first argument that is not an option, when it is shaped like
/// a command name.
fn subcommand(command: &Simple) -> Option<&str> {
    let word = (command.args.iter()).find(|word| !word.starts_with(['-', '+']))?;
    named(word).then_some(word.as_str())
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

/// Whether `tool` (at `program`) dispatches, and its subcommand `name` is
/// missing from `at`'s search path: not built in, and no `tool-name` there.
fn lacks(tool: &str, program: &Path, name: &str, at: &Context) -> bool {
    !at.on_path(&format!("{tool}-{name}"))
        && dispatches(tool, at)
        && builtins(program).is_some_and(|commands| !commands.contains(name))
}

/// Whether some program `tool-*` is on `at`'s search path.
fn dispatches(tool: &str, at: &Context) -> bool {
    let prefix = format!("{tool}-");
    let Some(path) = at.path.as_deref() else {
        return false;
    };
    std::env::split_paths(path)
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .any(|entry| {
            entry.file_name().to_string_lossy().starts_with(&prefix) && entry.path().is_file()
        })
}

/// The commands `program` builds in, as `program --list` lists them with
/// no search path and an empty home; `None` when it lists none. A tool that
/// answered is asked once per process; one that could not be started or
/// timed out is asked again next time.
fn builtins(program: &Path) -> Option<BTreeSet<String>> {
    let lock = || LISTED.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(known) = lock().get(program) {
        return known.clone();
    }
    let commands = list(program)?;
    lock().insert(program.to_path_buf(), commands.clone());
    commands
}

/// `program`'s listed commands; `None` when it did not answer.
fn list(program: &Path) -> Option<Option<BTreeSet<String>>> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let home = std::env::temp_dir().join(format!(
        "archon-command-list-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&home).ok()?;
    let listed = list_in(program, &home);
    let _ = std::fs::remove_dir_all(&home);
    listed
}

fn list_in(program: &Path, home: &Path) -> Option<Option<BTreeSet<String>>> {
    let out = home.join(".list");
    let spawn = || {
        Command::new(program)
            .arg("--list")
            .current_dir(home)
            .env_clear()
            .env("PATH", "")
            .env("HOME", home)
            .env("TMPDIR", home)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out)?)
            .stderr(Stdio::null())
            .spawn()
    };
    // A program written a moment ago can be briefly unable to start while
    // another thread's child still holds it open.
    let mut child = (0..3).find_map(|attempt| {
        std::thread::sleep(Duration::from_millis(50 * attempt));
        spawn().ok()
    })?;
    let deadline = Instant::now() + LIST_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = std::fs::read_to_string(&out).ok()?;
    let commands: BTreeSet<String> = (text.lines())
        .filter(|line| line.starts_with(char::is_whitespace))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| named(word))
        .map(str::to_string)
        .collect();
    Some((status.success() && !commands.is_empty()).then_some(commands))
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_verdict_subcommand_tests.rs"]
mod tests;

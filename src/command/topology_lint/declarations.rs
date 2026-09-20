//! Does a task declare the capabilities its own commands need?
//!
//! `required_tools` and `required_env_keys` are not documentation. The host
//! derives what it wires from them — the tool allowlist a write branch is
//! given, the provider environment it resolves and proves. A task that declares
//! `[]` and then runs `cargo test` has told the host it needs nothing, and the
//! host believes it.
//!
//! Presence is already enforced: `REQUIRED_TASK_KEYS` refuses a task file that
//! omits either key, on the stated grounds that absence "silently changes what
//! a run does". `[]` changes what a run does in the same way, and nothing
//! checked it — so whether a generated corpus got this right varied run to run.
//! Measured across the task sets on disk when this landed: one 15-task set
//! clean, the next 2 of 15 under-declaring, an older 22-task set wrong in all
//! 22.
//!
//! **Advisory, and deliberately not a parse-time refusal.** Making it a hard
//! error would render those existing task sets unrunnable, which is a
//! migration, not a lint. Reporting puts the finding in front of the generator
//! — which already runs `workflow lint` over what it wrote and acts on the
//! result — while the spec can still be corrected.
//!
//! Declarations come from [`parse_task_file`], the same parser a run uses, so
//! this cannot disagree with the runtime about what a task declared. The
//! commands come from the raw text, because that parser discards
//! `## Focused Tests` by design.
//!
//! Everything is derived from the task file's own content: no toolchain, PRD or
//! task identifier appears in this module, and none should. The question it
//! asks — "does this file contradict itself" — is the same for every corpus.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use archon_workflow::task_universe::parsing::parse_task_file;
use archon_workflow::task_universe::task_files_under;

/// First tokens that mean "a shell will execute this".
///
/// The same list the focused-test classifier uses to tell a command from a
/// described CLI fragment: a backticked `data list --json` in a bullet is prose
/// about a command, `cargo test -p x` is one.
/// Re-exported so this lint and the traceability reader cannot disagree about
/// what a runnable command looks like. They were byte-identical copies once,
/// and widening one alone left the task universe recording no focused tests
/// while this lint accepted them.
use archon_knowledge::traceability::tasks::KNOWN_RUNNERS;

/// Tasks that declare no runnable focused test, for a caller that blocks.
///
/// # Why this blocks rather than warns
///
/// A task with no runnable command cannot prove anything it claims. It will be
/// implemented, accepted on a summary nobody can check, and every requirement
/// it owns becomes unfalsifiable — the run reports success and the evidence for
/// it does not exist. That is worse than a task that fails, because a failure
/// is visible.
///
/// Derived from the task's own `## Focused Tests` bullets and the shared runner
/// list, so it needs no knowledge of the project's toolchain and holds for any
/// PRD in any language.
pub(super) fn task_has_runnable_test(raw: &str) -> bool {
    focused_test_commands(raw).iter().any(|command| {
        // `! grep -q marker file` asserts absence; the negation is shell
        // syntax, not the program being run.
        command
            .trim_start_matches('!')
            .trim_start()
            .split_whitespace()
            .next()
            .is_some_and(|first| KNOWN_RUNNERS.contains(&first))
            && archon_workflow::verifier_strength::verifier_strength_defect(
                Some(command),
                None,
                None,
            )
            .is_none()
    })
}

pub(super) fn missing_runnable_test_finding(task: &str) -> String {
    format!(
        "{task}: declares no runnable focused test — add a bullet under `## Focused Tests` containing a backticked command that can exit non-zero, or add a content check against this task's own output"
    )
}

pub(super) fn tasks_without_a_runnable_test(tasks_root: Option<&Path>) -> Vec<String> {
    let Some(root) = tasks_root else {
        return Vec::new();
    };
    let Ok(paths) = task_files_under(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for path in &paths {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(task) = parse_task_file(path, &raw) else {
            continue; // unparseable files are blocked by the contracts gate
        };
        if !task_has_runnable_test(&raw) {
            out.push(task.canonical_task_id.clone());
        }
    }
    out
}

pub(super) fn section(tasks_root: Option<&Path>) -> String {
    let mut out = String::from("\n## declared capabilities\n");
    let Some(root) = tasks_root else {
        out.push_str(
            "  only computed for --tasks: a spec or a recorded graph carries no task \
             frontmatter to check against its own commands.\n",
        );
        return out;
    };

    let paths = match task_files_under(root) {
        Ok(paths) => paths,
        Err(error) => {
            out.push_str(&format!("  could not read task files: {error}\n"));
            return out;
        }
    };
    if paths.is_empty() {
        out.push_str(&format!("  no task files under {}.\n", root.display()));
        return out;
    }

    let mut no_commands: Vec<String> = Vec::new();
    let mut undeclared_tools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut undeclared_env: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unread: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for path in &paths {
        let Ok(raw) = fs::read_to_string(path) else {
            unread.push(display_name(path));
            continue;
        };
        // A file the run would refuse is reported, not skipped. Skipping was
        // wrong in the way this whole section exists to prevent: with every
        // file unparseable the summary below said "every runner is declared",
        // which is a clean bill of health for a corpus nothing had read.
        let Ok(task) = parse_task_file(path, &raw) else {
            unread.push(display_name(path));
            continue;
        };
        checked += 1;
        let name = task.canonical_task_id.clone();

        let commands = focused_test_commands(&raw);
        // A task with no runnable verifier is the worse defect and used to be
        // invisible here: with nothing parsed there is no undeclared runner to
        // report, so the summary called the corpus clean. Observed live — six
        // of fifteen specs moved their commands into fenced ```bash blocks
        // while a repair drove a "prose entries" count to zero, and every
        // requirement those tasks claim became unprovable in the same stroke.
        if !task_has_runnable_test(&raw) {
            no_commands.push(name.clone());
        }
        for command in commands {
            let Some(first) = command.split_whitespace().next() else {
                continue;
            };
            if KNOWN_RUNNERS.contains(&first)
                && !task.required_tools.iter().any(|tool| tool == first)
            {
                undeclared_tools
                    .entry(name.clone())
                    .or_default()
                    .insert(first.to_string());
            }
            for key in referenced_env_keys(&command) {
                if !task.required_env_keys.contains(&key) {
                    undeclared_env.entry(name.clone()).or_default().insert(key);
                }
            }
        }
    }

    if !unread.is_empty() {
        out.push_str(&format!(
            "  {} task file(s) could not be read as tasks and were NOT checked: {}\n",
            unread.len(),
            unread.join(", ")
        ));
    }
    if !no_commands.is_empty() {
        out.push_str(&format!(
            "  {} task(s) declare NO runnable focused test: {}\n",
            no_commands.len(),
            no_commands.join(", ")
        ));
        out.push_str(
            "      a focused test counts only as a bullet containing a backticked \
             command; commands inside a fenced code block are not read, so these \
             tasks can never prove the requirements they claim\n",
        );
    }
    if no_commands.is_empty() && undeclared_tools.is_empty() && undeclared_env.is_empty() {
        out.push_str(&if checked == 0 {
            "  no task file could be read; nothing was checked, and this is not a pass.\n"
                .to_string()
        } else {
            format!(
                "  {checked} task(s) checked; every runner and environment key a focused \
                 test uses is declared.\n"
            )
        });
        return out;
    }

    for (name, tools) in &undeclared_tools {
        out.push_str(&format!(
            "  {name} runs {} without declaring it in `required_tools`\n",
            joined(tools)
        ));
        out.push_str(
            "      the host builds this task's tool allowlist from that field, so an \
             undeclared runner is one its branch may never be given\n",
        );
    }
    for (name, keys) in &undeclared_env {
        out.push_str(&format!(
            "  {name} references {} without declaring it in `required_env_keys`\n",
            joined(keys)
        ));
        out.push_str(
            "      the host resolves and proves exactly the keys this field names; an \
             undeclared key is not injected, and its absence is not reported as a gap\n",
        );
    }
    // Said plainly, because the obvious reading — "add these to the
    // declaration" — is only half of it, and the wrong half when the command is
    // what is mistaken.
    out.push_str(
        "  a mismatch is the task contradicting itself; either the declaration or the \
         command is wrong, and only the task's author knows which.\n",
    );
    out
}

/// Backticked spans under `## Focused Tests`.
///
/// Heading matched with the same tolerance the traceability reader uses —
/// either heading may be a whole-word prefix of the other — because authors
/// write `## Focused Tests and Evidence` and a stricter match would silently
/// find no commands and report a clean task.
fn focused_test_commands(raw: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut inside = false;
    let mut section_depth = 0;
    // Commands inside a fenced block count too — the traceability reader
    // reads them, and a lint that disagreed would report "no runnable
    // tests" for a spec the engine parses fine. Fence lines are recognised
    // by the shared rule; the toggle stays gated on the section, as the
    // engine's own reader gates it, so a fence elsewhere in the body (an
    // author's outer wrapper, a block in another section) cannot switch
    // this reader on or off.
    let mut in_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if super::fences::is_fence_line(trimmed) {
            if inside {
                in_fence = !in_fence;
            }
            continue;
        }
        if in_fence {
            if inside && !trimmed.is_empty() {
                commands.push(trimmed.to_string());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if !inside || level <= section_depth {
                inside = heading_matches(rest.trim_start_matches('#').trim(), "focused tests");
                section_depth = level;
            }
            continue;
        }
        if !inside {
            continue;
        }
        let mut chars = trimmed.char_indices().peekable();
        while let Some((start, ch)) = chars.next() {
            if ch != '`' {
                continue;
            }
            if let Some(end) = trimmed[start + 1..].find('`') {
                let span = trimmed[start + 1..start + 1 + end].trim();
                if !span.is_empty() {
                    commands.push(span.to_string());
                }
                while let Some(&(index, _)) = chars.peek() {
                    if index > start + end + 1 {
                        break;
                    }
                    chars.next();
                }
            }
        }
    }
    commands
}

fn heading_matches(found: &str, requested: &str) -> bool {
    let found = found.to_ascii_lowercase();
    if found == requested {
        return true;
    }
    let (shorter, longer) = if found.len() < requested.len() {
        (found.as_str(), requested)
    } else {
        (requested, found.as_str())
    };
    !shorter.is_empty()
        && longer.starts_with(shorter)
        && longer.as_bytes().get(shorter.len()) == Some(&b' ')
}

/// `$NAME` and `${NAME}` occurrences in a command.
///
/// Deliberately narrow. This cannot know that a task describing a remote
/// service needs the variable that addresses it — a spec whose text never names
/// the key is invisible here, and closing that gap belongs to the authoring
/// rules, not to a text scan. What it catches is a command naming a variable
/// the frontmatter does not.
/// Variables the shell itself provides. Naming one is not a project
/// capability, and reporting it as undeclared is noise that trains a reader to
/// ignore the section. Observed live on `$PWD`.
const SHELL_PROVIDED: &[&str] = &[
    "PWD", "OLDPWD", "HOME", "PATH", "USER", "SHELL", "SHLVL", "TERM", "TMPDIR", "LANG", "PS1",
    "RANDOM", "HOSTNAME", "EDITOR", "PAGER",
];

fn referenced_env_keys(command: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let bytes = command.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let mut start = index + 1;
        if bytes.get(start) == Some(&b'{') {
            start += 1;
        }
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        // A shell name cannot start with a digit: `$1` is a positional
        // argument, `$?` a status. Requiring an upper-case letter keeps `$dir`
        // and other lower-case locals out of an environment-key report.
        if end > start && !bytes[start].is_ascii_digit() {
            let name = &command[start..end];
            if name.chars().any(|c| c.is_ascii_uppercase()) && !SHELL_PROVIDED.contains(&name) {
                keys.insert(name.to_string());
            }
        }
        index = end.max(index + 1);
    }
    keys
}

/// File name for a report line, falling back to the whole path.
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn joined(values: &BTreeSet<String>) -> String {
    values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "declarations_tests.rs"]
mod declarations_tests;

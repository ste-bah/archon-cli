//! Compare declared capabilities with the commands the task actually names.
//!
//! The command grammar is shared with the task universe and traceability
//! reader. Code spans and fenced continuations are read by that grammar;
//! executable availability or task declarations distinguish commands from
//! test descriptions. There is no independent runner registry here.
//!
//! Capability comparisons are advisory. They never create a blocking finding:
//! shell syntax and environment references alone cannot establish a missing
//! capability. The separate runnable-test check applies the existing verifier
//! strength rule to the same command set.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use archon_workflow::task_universe::parsing::parse_task_file;
use archon_workflow::task_universe::task_files_under;

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
/// Derived from the task's declared commands and their ability to fail.
pub(super) fn task_has_runnable_test(
    task: &archon_workflow::task_universe::WorkflowV2TaskUniverseTask,
) -> bool {
    task.declared_focused_test_commands().iter().any(|command| {
        archon_workflow::verifier_strength::verifier_strength_defect(Some(command), None, None)
            .is_none()
    })
}

pub(super) fn missing_runnable_test_finding(task: &str) -> String {
    format!(
        "{task}: declares no runnable focused test — add a bullet under `## Focused Tests` containing a backticked command that can exit non-zero, or add a content check against this task's own output"
    )
}

#[cfg(test)]
pub(super) fn tasks_without_a_runnable_test(tasks_root: Option<&Path>) -> Vec<String> {
    let Some(root) = tasks_root else {
        return Vec::new();
    };
    let site = match crate::command::acceptance_check_policy::command_site_path(root) {
        Ok(site) => site,
        Err(error) => return vec![format!("command site policy: {error}")],
    };
    tasks_without_a_runnable_test_at_path(tasks_root, &site)
}

pub(super) fn tasks_without_a_runnable_test_at_path(
    tasks_root: Option<&Path>,
    site: &std::ffi::OsStr,
) -> Vec<String> {
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
        let Ok(mut task) = parse_task_file(path, &raw) else {
            continue; // unparseable files are blocked by the contracts gate
        };
        task.freeze_focused_commands(
            &super::scope_declarations::repository_root(root, root),
            &site,
        );
        if !task_has_runnable_test(&task) {
            out.push(task.canonical_task_id.clone());
        }
    }
    out
}

#[cfg(test)]
fn section(tasks_root: Option<&Path>) -> String {
    let Some(root) = tasks_root else {
        return section_at_path(None, std::ffi::OsStr::new(""));
    };
    match crate::command::acceptance_check_policy::command_site_path(root) {
        Ok(site) => section_at_path(tasks_root, &site),
        Err(error) => format!("command site policy: {error}"),
    }
}

pub(super) fn section_at_path(tasks_root: Option<&Path>, site: &std::ffi::OsStr) -> String {
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
    let files: Vec<_> = paths
        .iter()
        .map(|path| (path.as_path(), fs::read_to_string(path).ok()))
        .collect();
    out.push_str(&checked_files_with_path(&files, Some(site)));
    out
}

/// The section body for task files already read; `None` denotes an unreadable file.
/// Command selection uses the supplied captured site.
fn checked_files_with_path(
    files: &[(&Path, Option<String>)],
    toolchain_path: Option<&std::ffi::OsStr>,
) -> String {
    let mut out = String::new();
    let mut no_commands: Vec<String> = Vec::new();
    let mut undeclared_tools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut undeclared_env: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut unread: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (path, raw) in files {
        let Some(raw) = raw else {
            unread.push(display_name(path));
            continue;
        };
        // A file the run would refuse is reported, not skipped. Skipping was
        // wrong in the way this whole section exists to prevent: with every
        // file unparseable the summary below said "every runner is declared",
        // which is a clean bill of health for a corpus nothing had read.
        let Ok(mut task) = parse_task_file(path, raw) else {
            unread.push(display_name(path));
            continue;
        };
        {
            let path = toolchain_path.unwrap_or(std::ffi::OsStr::new(""));
            task.freeze_focused_commands(
                &archon_workflow::task_universe::focused_commands::repository_root(
                    &task.source_path,
                ),
                path,
            );
        }
        checked += 1;
        let name = task.canonical_task_id.clone();

        let commands = task.declared_focused_test_commands();
        // A task with no runnable verifier is the worse defect and used to be
        // invisible here: with nothing parsed there is no undeclared runner to
        // report, so the summary called the corpus clean. Observed live — six
        // of fifteen specs moved their commands into fenced ```bash blocks
        // while a repair drove a "prose entries" count to zero, and every
        // requirement those tasks claim became unprovable in the same stroke.
        if !task_has_runnable_test(&task) {
            no_commands.push(name.clone());
        }
        for command in commands {
            if let Some(first) =
                archon_workflow::task_universe::focused_commands::leading_program(&command)
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
            "      a focused test needs a declared command that can exit non-zero; \
             commands in inline spans and fenced blocks are read\n",
        );
    }
    if no_commands.is_empty() && undeclared_tools.is_empty() && undeclared_env.is_empty() {
        out.push_str(&if checked == 0 {
            "  no task file could be read; nothing was checked, and this is not a pass.\n"
                .to_string()
        } else {
            format!(
                "  {checked} task(s) checked; every compared leading word and environment key \
                 is declared.\n"
            )
        });
        return out;
    }

    for (name, tools) in &undeclared_tools {
        out.push_str(&format!(
            "  {name}: command-leading word(s) {} absent from `required_tools`\n",
            joined(tools)
        ));
        out.push_str(
            "      a program name alone cannot establish a required capability; this comparison \
             alone does not establish a missing capability\n",
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
        "  declaration comparisons above are advisory; review them against the declared commands.\n",
    );
    out
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

#[cfg(test)]
#[path = "declarations_agnostic_proof_tests.rs"]
mod agnostic_proof_tests;

#[cfg(test)]
#[path = "declarations_bench_tests.rs"]
mod bench_tests;

#[cfg(test)]
#[path = "declarations_rule_tests.rs"]
mod rule_tests;

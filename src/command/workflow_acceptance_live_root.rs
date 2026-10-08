//! A check that names a live root by its absolute path (Issue 366).
//!
//! A check starts from its working directory in whichever tree the host
//! runs it in. A freeze proves it in a copy (the scratch observation, or the
//! probe's own hermetic copy); an acceptance round runs it in the scratch
//! copy when `[workflow.acceptance_execution]` is configured, and otherwise
//! in the live checkout itself. A command that names a live root by its
//! absolute path would read or change the live tree from a copy too. The
//! freeze probe refuses such a check unrun, and both acceptance authors (the
//! decomposition's author step and the host re-author) refuse it in the same
//! words, so the author repairs it before any judge. All use this one rule,
//! and both authors are told it in one text ([`check_path_rule`]).
//!
//! A root is named when its text occurs in the command and the next
//! character does not continue a path component (`[A-Za-z0-9._-]`): the root
//! itself, a path under it, or the root followed by any other character (a
//! quote, a glob, a variable, a `printf` directive). A longer sibling name
//! that only starts with the root's text (`<root>-other`, `<root>.bak`) is a
//! different path and is not the root, unless its path leads back under the
//! root (`<root>-x/../<name>`). Every absolute path in the text is also read
//! lexically (`//`, `/./` and `x/..` resolved), so a spelling of a root with
//! redundant separators is still that root. Only absolute roots are matched:
//! a relative root would match nearly any text.

use std::path::{Path, PathBuf};

use crate::command::acceptance_scratch_policy::NativeBinding;

/// What every acceptance author is told before it writes a check, with the
/// repository's and the project's absolute paths: where a check can run, and
/// the relative-path rule. One text for the decomposition author
/// (`checkPathRule` in `workflow_decompose_v1_acceptance.js`, held equal to
/// this by test) and the host re-author.
pub(crate) fn check_path_rule(repository: &str, project: &str) -> String {
    format!(
        "Where a check runs: a check starts in a POSIX shell whose working directory is the project root when cwd is project_root (a floor's typed_verifier_command always starts there) or the repository root when cwd is repo_root, of whichever tree the host runs it in. Before acceptance the host proves each check in a disposable copy: a clone of the code repository at its committed HEAD (uncommitted changes are not in it) and the project files the host is configured to copy, which may be fewer than the project holds. An acceptance round runs the same check in such a copy or, when no isolated acceptance execution is configured, in the live repository and project themselves. So a check must not change, delete or reset anything outside its own temporary files (build output its own commands produce excepted). Choose the cwd whose root holds the files the command reads: repo_root for repository source and tests; never reach one root from the other with ..: the roots do not sit the same way in every copy.\nIn every check command, name every path relative to the check's working directory; never write the repository's or the project's absolute path ({repository}, {project}) or any path under them: those paths are for your reading only. The host refuses a check that names either root by its absolute path and never runs it."
    )
}

/// The repository a task set was decomposed against (its
/// `repository.lock`), else the project.
pub(crate) fn recorded_repository(project: &Path, tasks_root: &Path) -> PathBuf {
    archon_workflow::repository_record::read_repository_record(tasks_root)
        .ok()
        .flatten()
        .map(|record| PathBuf::from(record.repository_root))
        .filter(|root| root.is_dir())
        .unwrap_or_else(|| project.to_path_buf())
}

/// The repository a freeze proves a task set's checks against: the scratch
/// policy's (`binding`) when one is configured, else the recorded one. The
/// freeze probe and the author step both take it from here (L4).
pub(crate) fn freeze_repository(
    project: &Path,
    tasks_root: &Path,
    binding: Option<&NativeBinding>,
) -> PathBuf {
    binding.map_or_else(
        || recorded_repository(project, tasks_root),
        |binding| binding.policy.repository.clone(),
    )
}

/// The live roots of a task set as its freeze probe sees them: the project
/// and [`freeze_repository`] under the policy configured now. A policy that
/// cannot be captured adds nothing (the freeze then runs nothing).
pub(crate) fn task_set_roots(project: &Path, tasks_root: &Path) -> Vec<PathBuf> {
    let binding = crate::command::acceptance_scratch_policy::capture(project, tasks_root);
    let binding = binding.ok().flatten();
    vec![
        freeze_repository(project, tasks_root, binding.as_ref()),
        project.to_path_buf(),
    ]
}

/// The forms of `roots` check text could name: each absolute root as given
/// and its canonical form, without trailing separators. A relative root,
/// the filesystem root and empty text have no form: none is a live root
/// to match.
pub(crate) fn root_forms<'a>(roots: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    let mut forms = Vec::new();
    for root in roots.into_iter().filter(|root| root.has_root()) {
        forms.push(root.to_path_buf());
        if let Ok(canonical) = root.canonicalize().map(archon_shell::paths::plain) {
            forms.push(canonical);
        }
    }
    let mut forms: Vec<PathBuf> = (forms.into_iter())
        .map(|form| {
            let text = form.to_string_lossy();
            PathBuf::from(text.trim_end_matches(['/', '\\']))
        })
        .filter(|form| !form.as_os_str().is_empty())
        .collect();
    forms.sort();
    forms.dedup();
    forms
}

/// `roots` as absolute paths, or why one is not: the author-step validator
/// refuses to apply the rule with a relative root.
pub(crate) fn absolute_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    match roots.iter().find(|root| !root.has_root()) {
        Some(root) => Err(format!(
            "live root {} is not absolute; only absolute canonical roots are matched",
            root.display()
        )),
        None => Ok(roots.to_vec()),
    }
}

/// Whether `c` continues a path component (so the root's text is only the
/// start of a longer name).
fn continues_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Whether `c` ends a path token: whitespace or shell syntax.
fn ends_token(c: char) -> bool {
    c.is_whitespace() || "'\"`;|&<>()$=:,{}[]*?!#%\\".contains(c)
}

/// `token` (an absolute path) with `//`, `/./` and `x/..` resolved.
fn lexical(token: &str) -> Option<PathBuf> {
    if !token.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in token.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    Some(PathBuf::from(format!("/{}", parts.join("/"))))
}

/// The first of `forms` (from [`root_forms`]) that `text` names.
pub(crate) fn named_root<'a>(text: &str, forms: &'a [PathBuf]) -> Option<&'a PathBuf> {
    let paths: Vec<PathBuf> = text.split(ends_token).filter_map(lexical).collect();
    forms.iter().find(|form| {
        let root = form.to_string_lossy();
        let named_at = |at: usize| match text[at + root.len()..].chars().next() {
            // A sibling name, unless its path leads back under the root.
            Some(next) if continues_name(next) => {
                let token = text[at..].split(ends_token).next().unwrap_or_default();
                lexical(token).is_some_and(|path| path.starts_with(form))
            }
            _ => true,
        };
        !root.is_empty()
            && (text
                .match_indices(root.as_ref())
                .any(|(at, _)| named_at(at))
                || paths.iter().any(|path| path.starts_with(form)))
    })
}

/// The finding for check `id` that names live root `root`: the one text
/// the freeze and both authors give.
pub(crate) fn live_root_finding(id: &str, root: &Path) -> String {
    format!(
        "check '{id}': it names the live root {} by its absolute path, so no hermetic copy can keep it off the live tree and the host never runs it; name every path relative to the check's working directory",
        root.display()
    )
}

/// The text the host executes for an authored entry's `check` value: a
/// command's text, or a floor's non-blank verifier (as `executed_text`).
pub(crate) fn executed_check_text(check: &serde_json::Value) -> Option<&str> {
    match check.get("kind")?.as_str()? {
        "command" => check.get("command")?.as_str(),
        "floor" => (check
            .get("contract")?
            .get("typed_verifier_command")?
            .as_str())
        .filter(|command| !command.trim().is_empty()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_live_root_tests.rs"]
mod tests;

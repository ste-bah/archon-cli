//! A task whose contract obliges a repository file no task declares
//! (Issue-117).
//!
//! Live on wf-0ddadd81, a task listed a provider store module among surfaces
//! "observed and reused, not deliverables", yet required that it and the
//! task's own ingest lane "must stay consistent ... any change to one lane
//! must be mirrored". No task declared the file, so when a verifier found it
//! wrong no remediation could write it. A decomposition that places an
//! obligation on a file must give that file an owner.
//!
//! The check: every sentence of a task body's prose blocks (a paragraph or a
//! list item, with its wrapped lines) outside the ownership sections that
//! states a positive obligation (`must`, `shall`, `required`/`requires`,
//! `mirrored`, `stay consistent`) -- and neither a prohibition (`must not`,
//! `do not`, `never`) nor an observation (`observed`, `not a deliverable`)
//! -- and names a FILE that exists at the recorded base commit, by its
//! repository path or by a path suffix that names exactly one file there,
//! which no task owns (the same owned set the PRD owner check uses).
//!
//! It is ADVISORY, a report section, never a gate finding. Every other
//! ownership gap the set gate fails on is a pure set comparison that reads no
//! prose (`owner_coverage`: a PRD-named path either has an owner or not);
//! this one has to read an obligation out of prose, and the gate's own rule
//! is that a heuristic which blocks a decomposition is one the author cannot
//! argue with (`base_blocking_findings_with_mode`). So it warns, loudly, by
//! name -- and a frozen task set is never re-linted, so no running work is
//! touched by it.

use std::collections::BTreeSet;
use std::path::Path;

use archon_workflow::repository_record::{RepositoryTree, read_repository_record};
use archon_workflow::task_universe::{parsing::parse_task_file, task_files_under};

/// Whole words (matched on the sentence's letters, digits and spaces).
const OBLIGATIONS: [&str; 7] = [
    " must ",
    " shall ",
    " required ",
    " requires ",
    " mirrored ",
    " stay consistent ",
    " kept consistent ",
];
/// A sentence carrying any of these prohibits, observes, or asks only that
/// something keeps passing; it obliges no file to change.
const NOT_OBLIGATIONS: [&str; 16] = [
    " must not ",
    " shall not ",
    " mustn t ",
    " never ",
    " not required ",
    " do not ",
    " don t ",
    " observed ",
    " not deliverable",
    " not a deliverable",
    " pass ",
    " passes ",
    " passing ",
    " unchanged ",
    " untouched ",
    " green ",
];
/// Sections that declare ownership rather than oblige anything.
const OWNERSHIP_HEADINGS: [&str; 2] = ["forbidden", "expected to change"];

/// The report section: one WARNING per (task, file).
pub(super) fn section(tasks_root: Option<&Path>) -> String {
    let mut out = String::from("\n## unowned obligations\n");
    let Some(root) = tasks_root else {
        out.push_str("  not checked: no task directory\n");
        return out;
    };
    match warnings(root) {
        Ok(None) => out.push_str("  not checked: the task set has no repository record\n"),
        Ok(Some(found)) if found.is_empty() => {
            out.push_str("  none: every file a task body obliges has an owning task\n")
        }
        Ok(Some(found)) => {
            for warning in found {
                out.push_str(&format!("  WARNING: {warning}\n"));
            }
        }
        Err(error) => out.push_str(&format!("  not checked: {error:#}\n")),
    }
    out
}

/// Every (task, file) warning, or `None` without a repository record.
pub(super) fn warnings(root: &Path) -> anyhow::Result<Option<Vec<String>>> {
    let Some(record) = read_repository_record(root)? else {
        return Ok(None);
    };
    let tree = RepositoryTree::load(&record)?;
    let owned: BTreeSet<String> = super::owner_coverage::set_owned_paths(root)
        .iter()
        .flat_map(|entry| entry_paths(entry))
        .filter_map(|path| tree.relative_to_root(&path))
        .filter(|path| !path.is_empty())
        .collect();
    let covered = |path: &str| {
        owned
            .iter()
            .any(|owner| owner == path || path.starts_with(&format!("{owner}/")))
    };
    let mut found = Vec::new();
    for path in task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        let mut seen = BTreeSet::new();
        for block in obligation_blocks(&raw) {
            for file in named_files(&tree, &block) {
                if !covered(&file) && seen.insert(file.clone()) {
                    found.push(format!(
                        "{} obliges `{file}` (\"{}\") but no task declares it, so no remediation may write it; declare it in a task's Files Expected to Change",
                        task.canonical_task_id,
                        clip(&block)
                    ));
                }
            }
        }
    }
    Ok(Some(found))
}

/// The path spellings of one declared entry: its backticked spans, else its
/// first word.
fn entry_paths(entry: &str) -> Vec<String> {
    let spans: Vec<String> = entry
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|span| !span.is_empty())
        .map(str::to_string)
        .collect();
    if !spans.is_empty() {
        return spans;
    }
    entry
        .split_whitespace()
        .next()
        .map(|word| vec![word.to_string()])
        .unwrap_or_default()
}

/// Sentences of prose blocks outside the ownership sections that state an
/// obligation.
fn obligation_blocks(text: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut skipping = false;
    let mut flush = |current: &mut String, skipping: bool| {
        if !skipping {
            let joined = current.split_whitespace().collect::<Vec<_>>().join(" ");
            blocks.extend(
                joined
                    .split(". ")
                    .filter(|sentence| obliges(sentence))
                    .map(str::to_string),
            );
        }
        current.clear();
    };
    for line in super::fences::prose_lines(text) {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            flush(&mut current, skipping);
            let heading = heading.to_ascii_lowercase();
            skipping = OWNERSHIP_HEADINGS.iter().any(|h| heading.contains(h));
            continue;
        }
        let starts_item = trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with('|')
            || trimmed
                .split_once(". ")
                .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if trimmed.is_empty() || starts_item {
            flush(&mut current, skipping);
        }
        current.push(' ');
        current.push_str(trimmed);
    }
    flush(&mut current, skipping);
    blocks
}

fn obliges(sentence: &str) -> bool {
    let words: String = sentence
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    let words = format!(
        " {} ",
        words.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    !NOT_OBLIGATIONS.iter().any(|word| words.contains(word))
        && OBLIGATIONS.iter().any(|word| words.contains(word))
}

/// Files existing at the base that `block` names: an exact repository path,
/// or a path suffix (with a directory in it) that names exactly one file.
fn named_files(tree: &RepositoryTree, block: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for raw in block.split(|c: char| c.is_whitespace() || "`()[]{},;'\"<>|".contains(c)) {
        let token = raw.split("::").next().unwrap_or(raw);
        let token = token
            .trim_end_matches(['.', ':', ';', '!', '?'])
            .trim_start_matches("./");
        let token = match token.rsplit_once(':') {
            Some((head, tail)) if tail.bytes().all(|b| b.is_ascii_digit() || b == b'-') => head,
            _ => token,
        };
        if !token.contains('/') || token.contains(['*', '$', '<', '>']) || token.contains("..") {
            continue;
        }
        let Some(relative) = tree.relative_to_root(token) else {
            continue;
        };
        let is_file = |path: &str| tree.exists_at_base(path) && !tree.is_dir_at_base(path);
        if is_file(&relative) {
            found.insert(relative);
            continue;
        }
        let suffix = format!("/{relative}");
        let mut matches = tree
            .paths_at_base()
            .iter()
            .filter(|path| path.ends_with(&suffix) && is_file(path));
        if let (Some(only), None) = (matches.next(), matches.next()) {
            found.insert(only.clone());
        }
    }
    found
}

fn clip(text: &str) -> String {
    const LIMIT: usize = 160;
    if text.chars().count() <= LIMIT {
        return text.to_string();
    }
    format!("{}...", text.chars().take(LIMIT).collect::<String>())
}

#[cfg(test)]
#[path = "unowned_obligations_tests.rs"]
mod tests;

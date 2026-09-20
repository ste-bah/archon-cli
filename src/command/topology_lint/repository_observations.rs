//! Every deliverable path in a task body carries an observation the lint can
//! verify against the recorded repository (Issue-56).
//!
//! `repository_claims` refutes what a body asserts about a path; it says
//! nothing about a body that asserts nothing. Run wf-123aa567's authors,
//! refused every read of the repository, wrote each deliverable as "not
//! observed from this authoring run — implementer must record exists (N
//! lines) or absent", and that passed, because a refusal to claim is not a
//! false claim. This section closes that: a path the body lists as a
//! deliverable must be followed by one of three observations, and the lint
//! checks the observation against the checkout, line count included.
//!
//! # The observation grammar
//!
//! For every path under `## Files Expected to Change`, every
//! `deliverable_contracts` `artifact_path` and every
//! `shared_append_target_files` entry that resolves under the repository
//! root, the body must contain, somewhere in its prose (any list item or
//! sentence outside a fenced block), the backticked path — repository-relative
//! or `<repo>/<relative>` — followed, after at most a separator (`—`, `-`,
//! `–`, `:`, `,`) and any closing emphasis or backticks, by exactly one of:
//!
//! - `exists (N lines)` — a file; `N` is the number of lines as the `Read`
//!   tool numbers them (the last line number shown once the whole file has
//!   been read), checked `±0` against the checkout;
//! - `exists (directory)` — a directory (`dir` is accepted);
//! - `absent` — not in the repository.
//!
//! A path with none of these, or whose sentence says it was not observed
//! ("not observed", "could not read", "outside allowed" and kin), is a
//! blocking `Body` finding naming the path, so the repair loop rewrites it.
//! Refutation follows `repository_claims`: a claim is contradicted only when
//! the base commit and the checkout agree against it, and a line count is
//! read from the checkout the author read.

use std::path::Path;

use archon_workflow::repository_record::RepositoryTree;
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Observation {
    File { lines: usize },
    Directory,
    Absent,
}

/// Wording that says the author did not look. Lowercase; matched anywhere in
/// a sentence that names the path.
const UNOBSERVED_WORDING: &[&str] = &[
    "not observed",
    "could not read",
    "couldn't read",
    "could not be read",
    "unable to read",
    "was not read",
    "not readable",
    "outside allowed",
    "outside this run",
    "outside the run",
    "not verified",
    "unverified",
    "implementer must record",
    "implementer must confirm",
    "could not verify",
    "couldn't verify",
    "unable to verify",
];

/// The deliverable paths a parsed task names, as written.
fn deliverable_paths(task: &WorkflowV2TaskUniverseTask) -> Vec<String> {
    let mut paths: Vec<String> = task
        .files_expected_to_change
        .iter()
        .flat_map(|item| paths_in_item(item))
        .collect();
    paths.extend(
        task.deliverable_contracts
            .iter()
            .map(|c| c.artifact_path.trim().to_string()),
    );
    paths.extend(
        task.shared_append_target_files
            .iter()
            .map(|p| p.trim().to_string()),
    );
    paths.retain(|path| !path.is_empty());
    paths.sort();
    paths.dedup();
    paths
}

/// The paths in one `Files Expected to Change` item: its backticked spans,
/// or its first token when it has none, split on `,`/`;`, path-shaped only.
fn paths_in_item(item: &str) -> Vec<String> {
    let mut spans = backticked_spans(item);
    if spans.is_empty() {
        spans.extend(item.split_whitespace().next().map(str::to_string));
    }
    spans
        .iter()
        .flat_map(|span| span.split([',', ';']))
        .map(str::trim)
        .filter(|token| is_path_shaped(token))
        .map(str::to_string)
        .collect()
}

fn backticked_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        spans.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    spans
}

fn is_path_shaped(token: &str) -> bool {
    if token.is_empty() || token.contains(char::is_whitespace) || token.contains("://") {
        return false;
    }
    if token.contains(['*', '{', '}', '$', '<', '>', '|', '=', '"', '\'']) {
        return false;
    }
    let last = token
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(token);
    token.contains('/')
        || last
            .rsplit_once('.')
            .is_some_and(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
}

/// The prose lines of `text`: fenced blocks are not prose.
fn prose_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut fenced = false;
    text.lines().filter(move |line| {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            return false;
        }
        !fenced
    })
}

/// Each occurrence of a backticked token naming `relative`, with the text
/// that follows it on its line.
fn mentions<'a>(tree: &RepositoryTree, line: &'a str, relative: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('`') else {
            break;
        };
        let token = after_open[..close].trim();
        let after = &after_open[close + 1..];
        if !token.is_empty()
            && !token.contains(char::is_whitespace)
            && tree.relative_to_root(token).as_deref() == Some(relative)
        {
            found.push(after);
        }
        rest = after;
    }
    found
}

/// The observation stated for `relative` anywhere in the body's prose. The
/// first well-formed one wins; a body stating two is a body to rewrite, and
/// the checkout decides which of them was wrong.
pub(crate) fn observation_for(
    tree: &RepositoryTree,
    text: &str,
    relative: &str,
) -> Option<Observation> {
    prose_lines(text)
        .flat_map(|line| mentions(tree, line, relative))
        .find_map(parse_observation)
}

/// The observation at the start of `after` (what follows the path's closing
/// backtick), or `None` when the text there is anything else.
pub(crate) fn parse_observation(after: &str) -> Option<Observation> {
    let lowered = after.to_ascii_lowercase();
    let mut rest = lowered.trim_start();
    loop {
        let trimmed = rest
            .trim_start_matches(['—', '-', '–', ':', ',', '`', '*', ')'])
            .trim_start();
        if trimmed.len() == rest.len() {
            break;
        }
        rest = trimmed;
    }
    if let Some(tail) = rest.strip_prefix("absent") {
        return ends_word(tail).then_some(Observation::Absent);
    }
    let tail = rest.strip_prefix("exists")?.trim_start();
    let inner = tail.strip_prefix('(')?;
    let close = inner.find(')')?;
    let inner = inner[..close].trim();
    if matches!(inner, "directory" | "dir") {
        return Some(Observation::Directory);
    }
    let (count, unit) = inner.split_once(char::is_whitespace)?;
    let unit = unit.trim();
    if unit != "lines" && unit != "line" {
        return None;
    }
    let lines = count.replace([',', '_'], "").parse::<usize>().ok()?;
    Some(Observation::File { lines })
}

fn ends_word(tail: &str) -> bool {
    tail.chars().next().is_none_or(|c| !c.is_alphanumeric())
}

/// A sentence naming `relative` that says it was not observed, if any.
fn unobserved_wording(tree: &RepositoryTree, text: &str, relative: &str) -> Option<String> {
    prose_lines(text)
        .filter(|line| !mentions(tree, line, relative).is_empty())
        .find(|line| {
            let lowered = line.to_ascii_lowercase();
            UNOBSERVED_WORDING
                .iter()
                .any(|phrase| lowered.contains(phrase))
        })
        .map(|line| super::repository_claims::excerpt(line.trim()))
}

/// Lines as the `Read` tool numbers them: `str::lines`, on bytes so a file
/// that is not UTF-8 still counts.
pub(crate) fn count_lines(bytes: &[u8]) -> usize {
    match bytes.last() {
        None => 0,
        Some(last) => bytes.iter().filter(|b| **b == b'\n').count() + usize::from(*last != b'\n'),
    }
}

fn checkout_lines(tree: &RepositoryTree, relative: &str) -> Option<usize> {
    std::fs::read(tree.root().join(relative))
        .ok()
        .map(|bytes| count_lines(&bytes))
}

/// The blocking findings for the body `raw` of `task`, each with the
/// repository-relative path it is about.
pub(crate) fn findings_against(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    raw: &str,
    task: &WorkflowV2TaskUniverseTask,
) -> Vec<(String, String)> {
    let mut findings = Vec::new();
    for path in deliverable_paths(task) {
        let Some(relative) = tree.relative_to_root(&path).filter(|r| !r.is_empty()) else {
            continue;
        };
        let truth = tree.truth(&relative);
        // A relative path present in the project but not in the checkout is
        // a project artifact (a registry, a report), not repository source.
        if !path.starts_with('/') && !truth.in_checkout && project_root.join(&relative).exists() {
            continue;
        }
        let root = tree.root().display();
        let base = tree.base_commit();
        if let Some(wording) = unobserved_wording(tree, raw, &relative) {
            findings.push((relative.clone(), format!(
                "{task_id}: deliverable path `{relative}` is described as unobserved (\"{wording}\"); read it under the repository root {root} and replace that with `{root}/{relative}` — exists (N lines), — exists (directory) or — absent"
            )));
            continue;
        }
        let checkout_dir = tree.root().join(&relative).is_dir();
        let text = match observation_for(tree, raw, &relative) {
            None => format!(
                "{task_id}: deliverable path `{relative}` has no verifiable observation; read it under the repository root {root} and write `{root}/{relative}` — exists (N lines) with N the last line number the Read tool shows, — exists (directory) for a directory, or — absent when it is not there"
            ),
            Some(Observation::Absent) if truth.certainly_exists() => {
                match checkout_lines(tree, &relative) {
                    Some(lines) if !checkout_dir => format!(
                        "{task_id}: the body says `{relative}` is absent but it exists in repository {root} at base commit {base} and in the checkout ({lines} lines); rewrite the observation as exists ({lines} lines)"
                    ),
                    _ => format!(
                        "{task_id}: the body says `{relative}` is absent but it exists in repository {root} at base commit {base} and in the checkout as a directory; rewrite the observation as exists (directory)"
                    ),
                }
            }
            Some(Observation::File { .. } | Observation::Directory) if truth.certainly_absent() => {
                format!(
                    "{task_id}: the body says `{relative}` exists but it is absent from repository {root} at base commit {base} and from the checkout; rewrite the observation as absent"
                )
            }
            Some(Observation::File { lines }) if truth.in_checkout => {
                if checkout_dir {
                    format!(
                        "{task_id}: the body says `{relative}` exists ({lines} lines) but it is a directory in repository {root}; rewrite the observation as exists (directory)"
                    )
                } else {
                    match checkout_lines(tree, &relative) {
                        Some(actual) if actual != lines => format!(
                            "{task_id}: the body says `{relative}` exists ({lines} lines) but the checkout at {root} has {actual} lines; rewrite the observation as exists ({actual} lines)"
                        ),
                        _ => continue,
                    }
                }
            }
            Some(Observation::Directory) if truth.in_checkout && !checkout_dir => {
                let lines = checkout_lines(tree, &relative).unwrap_or(0);
                format!(
                    "{task_id}: the body says `{relative}` exists (directory) but it is a file of {lines} lines in repository {root}; rewrite the observation as exists ({lines} lines)"
                )
            }
            Some(_) => continue,
        };
        findings.push((relative, text));
    }
    findings
}

#[cfg(test)]
#[path = "repository_observations_tests.rs"]
mod tests;

//! A task body's claims about repository paths, checked against the
//! repository the decomposition was grounded in (Issue-55).
//!
//! Authors grounded in an empty tree wrote bodies asserting that files which
//! exist "do not exist", and a critic that only reads prose cannot catch a
//! false statement about the filesystem. This section is deterministic: it
//! extracts every sentence that unambiguously asserts a specific backticked
//! repository-relative path exists or does not exist, and checks the path
//! against the recorded repository at its recorded base commit and in the
//! checkout. A body that says a file does not exist when it does, or exists
//! when it does not, gets a blocking `Body` finding naming the exact path and
//! the observed truth, so the body-repair loop rewrites the sentence.
//!
//! # No false positives
//!
//! Only a claim whose subject is the path itself is read: the backticked
//! path, then at most a few filler words, then an existence phrase, then a
//! terminator. "`src/lib.rs` is missing the trait impl" says nothing about
//! the file's existence and is not a claim; "the feature does not exist in
//! `src/lib.rs`" puts the path after the phrase and is not a claim either.
//! Truth is read from both the base commit and the checkout, and a claim is
//! refuted only when both agree against it: a file added or deleted since
//! the base is neither certainly present nor certainly absent, so an author
//! who read the checkout is never contradicted by history it could not see.
//! A task set without a repository record predates the record and is not
//! checked.

use std::path::Path;

use anyhow::{Context, Result};
use archon_workflow::repository_record::{RepositoryTree, read_repository_record};
use archon_workflow::task_universe::{parsing::parse_task_file, task_files_under};

use crate::command::workflow_gate::{GateFinding, GateId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claim {
    Exists,
    Absent,
}

/// One unambiguous claim: the backticked path, what the sentence says of it,
/// and the sentence itself for the finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathClaim {
    pub(crate) path: String,
    pub(crate) claim: Claim,
    pub(crate) sentence: String,
}

/// Phrases that, directly after the path, assert it is absent.
const ABSENT_AFTER: &[&str] = &[
    "does not exist yet",
    "does not yet exist",
    "does not exist",
    "do not exist yet",
    "do not yet exist",
    "do not exist",
    "doesn't exist yet",
    "doesn't exist",
    "don't exist",
    "is not present",
    "are not present",
    "is absent",
    "are absent",
    "absent",
    "is missing",
    "are missing",
    "missing",
    "will be created",
    "to be created",
    "must be created",
    "needs to be created",
    "is new",
    "is a new file",
    "is a new module",
    "(new)",
    "new",
];
/// Phrases that, directly after the path, assert it is present.
const EXISTS_AFTER: &[&str] = &[
    "already exists",
    "exists",
    "exist",
    "is present",
    "are present",
    "present",
    "will be modified",
    "will be extended",
    "will be edited",
    "will be updated",
    "to be modified",
    "to be extended",
    "is modified",
    "is extended",
];
/// Words allowed between the path and its phrase without changing the subject.
const FILLER: &[&str] = &[
    "file",
    "module",
    "directory",
    "dir",
    "crate",
    "path",
    "currently",
    "already",
    "still",
    "now",
    "which",
    "that",
    "is",
    "are",
    "was",
    "(",
    ":",
    "—",
    "-",
    "–",
    ",",
];
/// Verbs directly before the path that assert it is present (the body will
/// change a file it has), and phrases that assert it is absent (a new file).
const EXISTS_BEFORE: &[&str] = &[
    "modify",
    "modifies",
    "modifying",
    "extend",
    "extends",
    "extending",
    "edit",
    "edits",
    "editing",
    "update",
    "updates",
    "updating",
    "existing file",
    "existing module",
    "existing",
];
const ABSENT_BEFORE: &[&str] = &[
    "new file",
    "new module",
    "new crate",
    "new directory",
    "create new",
    "creates new",
    "add new",
    "adds new",
    "a new file",
    "a new module",
];
/// What may follow a phrase for it to be the whole predicate.
const TERMINATORS: &[&str] = &[
    "",
    ".",
    ",",
    ";",
    ":",
    ")",
    "]",
    "(",
    "yet",
    "today",
    "currently",
    "in the repository",
    "in the repo",
    "at",
    "and",
    "but",
    "—",
    "-",
    "–",
    "so",
    "under",
];

/// Every unambiguous claim in `text`, in order. Fenced code blocks are not
/// prose and are skipped; inline backticks are read.
pub(crate) fn extract_claims(text: &str) -> Vec<PathClaim> {
    let mut claims = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for sentence in split_sentences(line) {
            claims.extend(claims_in_sentence(sentence));
        }
    }
    claims
}

fn split_sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if matches!(bytes[index], b'.' | b';' | b'!' | b'?') && bytes[index + 1] == b' ' {
            out.push(&line[start..=index]);
            start = index + 1;
        }
        index += 1;
    }
    out.push(&line[start..]);
    out.into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn claims_in_sentence(sentence: &str) -> Vec<PathClaim> {
    let mut claims = Vec::new();
    let mut rest = sentence;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('`') else {
            break;
        };
        let token = &after_open[..close];
        let before = &rest[..open];
        let after = &after_open[close + 1..];
        if let Some(path) = repository_relative_token(token) {
            let claim = claim_after(after).or_else(|| claim_before(before));
            if let Some(claim) = claim {
                claims.push(PathClaim {
                    path,
                    claim,
                    sentence: sentence.trim().to_string(),
                });
            }
        }
        rest = after;
    }
    claims
}

/// A backticked token that reads as a file or directory path: no whitespace,
/// a `/`, no glob or template characters, and a file extension on its last
/// component or a trailing `/`. Relative tokens are normalised; an absolute
/// token (the `<repo>/<relative path>` citation the body author is asked
/// for) is kept whole and resolved against the repository root later, where
/// one outside the root is dropped.
pub(crate) fn repository_relative_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty()
        || token.chars().any(char::is_whitespace)
        || !token.contains('/')
        || token.starts_with('~')
        || token.contains("://")
        || token.contains(['*', '{', '}', '$', '<', '>', '|', '=', '"', '\''])
        || token.starts_with("..")
        || token.contains("/..")
    {
        return None;
    }
    let absolute = token.starts_with('/');
    let normalized = archon_workflow::repository_record::normalize_relative(token);
    if normalized.is_empty() {
        return None;
    }
    let kept = if absolute {
        format!("/{normalized}")
    } else {
        normalized.clone()
    };
    if token.ends_with('/') {
        return Some(kept);
    }
    let last = normalized.rsplit('/').next().unwrap_or(&normalized);
    let (stem, extension) = last.rsplit_once('.')?;
    let extension_ok = !extension.is_empty()
        && extension.len() <= 8
        && extension.chars().all(|c| c.is_ascii_alphanumeric());
    (extension_ok && !stem.is_empty()).then_some(kept)
}

fn claim_after(after: &str) -> Option<Claim> {
    let lowered = after.trim_start().to_ascii_lowercase();
    let mut rest = lowered.as_str();
    for _ in 0..4 {
        if let Some(claim) = phrase_at_start(rest) {
            return Some(claim);
        }
        let mut advanced = false;
        for filler in FILLER {
            if let Some(stripped) = rest.strip_prefix(filler) {
                let boundary = filler.chars().last().is_some_and(|c| !c.is_alphanumeric())
                    || stripped.starts_with([' ', '(', ':', ',', '—', '-']);
                if boundary {
                    rest = stripped.trim_start();
                    advanced = true;
                    break;
                }
            }
        }
        if !advanced {
            return None;
        }
    }
    None
}

fn phrase_at_start(rest: &str) -> Option<Claim> {
    for (phrases, claim) in [(ABSENT_AFTER, Claim::Absent), (EXISTS_AFTER, Claim::Exists)] {
        for phrase in phrases {
            if let Some(tail) = rest.strip_prefix(phrase)
                && TERMINATORS
                    .iter()
                    .any(|terminator| terminates(tail, terminator))
            {
                return Some(claim);
            }
        }
    }
    None
}

/// Does `tail` (what follows a phrase) begin with `terminator` as a whole
/// word, or is it empty for the empty terminator.
fn terminates(tail: &str, terminator: &str) -> bool {
    let tail = tail.trim_start();
    if terminator.is_empty() {
        return tail.is_empty();
    }
    let Some(rest) = tail.strip_prefix(terminator) else {
        return false;
    };
    let word = terminator.chars().last().is_some_and(char::is_alphanumeric);
    !word || rest.chars().next().is_none_or(|c| !c.is_alphanumeric())
}

fn claim_before(before: &str) -> Option<Claim> {
    let lowered = before.trim_end().to_ascii_lowercase();
    let mut rest = lowered.as_str();
    for _ in 0..3 {
        for (phrases, claim) in [
            (ABSENT_BEFORE, Claim::Absent),
            (EXISTS_BEFORE, Claim::Exists),
        ] {
            for phrase in phrases {
                if let Some(head) = rest.strip_suffix(phrase)
                    && head.chars().last().is_none_or(|c| !c.is_alphanumeric())
                {
                    return Some(claim);
                }
            }
        }
        // A filler word is stripped only as a whole word: "schema" must not
        // lose its "a" and become something else.
        let trimmed = [
            "the",
            "file",
            "module",
            "directory",
            "crate",
            "a",
            "an",
            ":",
            "-",
            "—",
        ]
        .iter()
        .find_map(|filler| {
            let head = rest.strip_suffix(filler)?;
            let whole = !filler.chars().last().is_some_and(char::is_alphanumeric)
                || head.chars().last().is_none_or(|c| !c.is_alphanumeric());
            whole.then(|| head.trim_end())
        });
        match trimmed {
            Some(head) if head.len() < rest.len() => rest = head,
            _ => return None,
        }
    }
    None
}

/// The blocking findings for the task body `text` at `path`, against the
/// repository recorded under `tasks_root`. Empty when the set has no record.
/// An error means the record or the repository could not be read:
/// operational, never a pass.
pub(crate) fn inspect(
    project_root: &Path,
    tasks_root: &Path,
    task_id: &str,
    path: &Path,
    text: &str,
) -> Result<Vec<String>> {
    let Some(record) = read_repository_record(tasks_root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    Ok(body_findings(&tree, project_root, task_id, path, text))
}

/// Both sections for one body: every deliverable path's observation
/// (Issue-56), then every other claim the prose makes. A path the
/// observation section already reports is not reported twice — the finding
/// that names the line count is the one the author needs.
pub(crate) fn body_findings(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    path: &Path,
    text: &str,
) -> Vec<String> {
    // An unparsable body has no deliverable lists; the preflight reports it.
    let observed = parse_task_file(path, text)
        .map(|task| {
            super::repository_observations::findings_against(
                tree,
                project_root,
                task_id,
                text,
                &task,
            )
        })
        .unwrap_or_default();
    let reported: std::collections::BTreeSet<&str> =
        observed.iter().map(|(p, _)| p.as_str()).collect();
    let claims = claim_findings(tree, project_root, task_id, text)
        .into_iter()
        .filter(|(p, _)| !reported.contains(p.as_str()))
        .map(|(_, text)| text)
        .collect::<Vec<_>>();
    observed
        .into_iter()
        .map(|(_, text)| text)
        .chain(claims)
        .collect()
}

/// `project_root` is where a relative path that is not repository source (a
/// PRD, a task file, a project artifact) may legitimately live: an "exists"
/// claim about a path present there is about the project, not the
/// repository, and is never refuted.
pub(crate) fn findings_against(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    text: &str,
) -> Vec<String> {
    claim_findings(tree, project_root, task_id, text)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Each refuted claim with the repository-relative path it is about.
fn claim_findings(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    text: &str,
) -> Vec<(String, String)> {
    let mut findings = Vec::new();
    for claim in extract_claims(text) {
        let Some(relative) = tree.relative_to_root(&claim.path) else {
            continue;
        };
        let truth = tree.truth(&relative);
        let in_project = !claim.path.starts_with('/') && project_root.join(&relative).exists();
        let observed = match claim.claim {
            Claim::Absent if truth.certainly_exists() => format!(
                "it exists in repository {} at base commit {} and in the checkout",
                tree.root().display(),
                tree.base_commit()
            ),
            Claim::Exists if truth.certainly_absent() && !in_project => format!(
                "it is absent from repository {} at base commit {} and from the checkout",
                tree.root().display(),
                tree.base_commit()
            ),
            _ => continue,
        };
        let said = match claim.claim {
            Claim::Absent => "does not exist",
            Claim::Exists => "exists",
        };
        let finding = format!(
            "{task_id}: the body says `{relative}` {said} (\"{}\") but {observed}; read the path under the repository root and rewrite the claim to what is there",
            excerpt(&claim.sentence)
        );
        findings.push((relative, finding));
    }
    findings
}

pub(super) fn excerpt(sentence: &str) -> String {
    const MAX: usize = 160;
    if sentence.chars().count() <= MAX {
        return sentence.to_string();
    }
    let mut cut: String = sentence.chars().take(MAX).collect();
    cut.push('…');
    cut
}

/// The set gate's findings: every task body under `root`, each as a `Body`
/// finding on the task it names.
pub(crate) fn set_findings(project_root: &Path, root: &Path) -> Result<Vec<GateFinding>> {
    let Some(record) = read_repository_record(root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    let mut findings = Vec::new();
    // Unreadable or unparsable task files are the shared preflight's to report.
    for path in task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        findings.extend(
            body_findings(&tree, project_root, &task.canonical_task_id, &path, &raw)
                .into_iter()
                .map(|text| {
                    GateFinding::new(
                        GateId::WorkflowLintTaskSet,
                        text,
                        &task.canonical_task_id,
                        Some(path.clone()),
                        archon_workflow::RemediationScope::Body,
                    )
                }),
        );
    }
    Ok(findings)
}

#[cfg(test)]
#[path = "repository_claims_tests.rs"]
mod tests;

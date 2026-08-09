//! Reading the binding a task declares: which requirements it claims, which
//! paths it will touch, and which verifier commands it names.
//!
//! # Why a narrow reader rather than the workflow task parser
//!
//! `WorkflowV2TaskUniverseTask` parses these same files, and reads none of the
//! three fields this module needs: `implements` is not in `REQUIRED_TASK_KEYS`,
//! `## Focused Tests` is explicitly discarded, and `files_expected_to_change`
//! is read as prose bullets rather than paths. This reader is deliberately
//! narrow — three fields, documented grammar, fails closed on anything it does
//! not recognise — and it does not duplicate scheduling, dependency or contract
//! semantics, which stay where they are.
//!
//! # The grammar it accepts, and what it refuses
//!
//! Metadata is a fenced ```` ```yaml ```` block, not `---` front matter.
//! `implements:` is a single-line flow sequence: `implements: [REQ-DL-020,
//! REQ-DL-021]` or `implements: []`. A block sequence, a quoted scalar or a
//! missing bracket is an error naming the file — not an empty list. A task that
//! claims nothing and a task whose claim could not be read are different facts,
//! and collapsing them is how an unclaimed requirement disappears from a report.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::errors::{KnowledgeError, Result};

/// Where a verifier command was declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifierOrigin {
    /// A backticked command in the task's `## Focused Tests` section.
    FocusedTests,
    /// A deliverable contract's `typed_verifier_command`.
    TypedVerifier,
}

/// A command the task named as proof of its own work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierCommand {
    /// Whitespace-normalised, exactly as declared otherwise.
    pub command: String,
    pub origin: VerifierOrigin,
}

/// One `## Focused Tests` bullet, classified.
///
/// The distinction matters and is the single most consequential thing this
/// module reports about the real corpus: most focused-test bullets are *test
/// descriptions*, not runnable commands. A description cannot be matched
/// against a recorded run, so a requirement whose only declared verifier is
/// prose can never reach `Exercised` — and that is a decomposition gap in the
/// task, reported here rather than papered over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusedTestEntry {
    /// A backticked span whose first token is a known runner.
    Command(String),
    /// Everything else: a description of a test, not an invocation of one.
    Prose(String),
}

/// The runners a backticked span must start with to count as a command.
///
/// An explicit list, not a heuristic. A backticked `data list --json` inside a
/// focused-test bullet is a CLI fragment being described, not a command being
/// declared, and the difference is exactly whether the first token is something
/// a shell can execute. Extend this list when a real task declares a real
/// runner that is missing; do not replace it with a guess.
const KNOWN_RUNNERS: &[&str] = &[
    "archon", "bash", "cargo", "deno", "go", "gradle", "just", "make", "mvn", "node", "npm",
    "pnpm", "pytest", "python", "python3", "sh", "tox", "yarn",
];

/// What one task file declares, as far as traceability is concerned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBinding {
    pub task_id: String,
    /// Repository-relative path of the task file, for citation.
    pub source_path: String,
    /// The requirement IDs this task claims to satisfy — decision D5's explicit
    /// field. Order preserved as declared.
    pub implements: Vec<String>,
    /// Paths the task says it will change, extracted from the backticked spans
    /// of `## Files Expected to Change`. Used to scope semantic search: an
    /// anchor outside these paths is not this task's work.
    pub path_scopes: Vec<String>,
    /// Every `## Focused Tests` bullet, classified.
    pub focused_tests: Vec<FocusedTestEntry>,
    /// Commands that could actually be matched against a recorded run.
    pub verifier_commands: Vec<VerifierCommand>,
}

impl TaskBinding {
    /// Focused-test bullets that are descriptions rather than commands.
    pub fn prose_focused_tests(&self) -> Vec<&str> {
        self.focused_tests
            .iter()
            .filter_map(|entry| match entry {
                FocusedTestEntry::Prose(text) => Some(text.as_str()),
                FocusedTestEntry::Command(_) => None,
            })
            .collect()
    }
}

fn implements_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*implements:\s*(?P<body>.*)$").expect("implements regex is a literal")
    })
}

fn task_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*task_id:\s*(?P<id>\S+)\s*$").expect("task_id regex is a literal")
    })
}

fn typed_verifier_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*typed_verifier_command:\s*(?P<cmd>.+?)\s*$")
            .expect("typed_verifier regex is a literal")
    })
}

fn backtick_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"`([^`]+)`").expect("backtick regex is a literal"))
}

/// Parse one task file.
///
/// Errors when `task_id:` is absent or `implements:` is present in a shape this
/// reader does not accept. Absent `implements:` is *not* an error — a task
/// written before D5 claims nothing, and that is a true statement about it.
pub fn parse_task_binding(raw: &str, source_path: &str) -> Result<TaskBinding> {
    let mut binding = TaskBinding {
        source_path: source_path.to_string(),
        ..TaskBinding::default()
    };

    for line in yaml_block(raw).lines() {
        if binding.task_id.is_empty()
            && let Some(caps) = task_id_re().captures(line)
        {
            binding.task_id = caps["id"].to_string();
            continue;
        }
        if let Some(caps) = implements_re().captures(line) {
            binding.implements = parse_flow_sequence(caps["body"].trim(), source_path)?;
            continue;
        }
        if let Some(caps) = typed_verifier_re().captures(line) {
            binding.verifier_commands.push(VerifierCommand {
                command: normalize_command(&caps["cmd"]),
                origin: VerifierOrigin::TypedVerifier,
            });
        }
    }

    if binding.task_id.is_empty() {
        return Err(KnowledgeError::Traceability(format!(
            "{source_path}: no `task_id:` in the task's yaml block"
        )));
    }

    binding.path_scopes = collect_path_scopes(raw);
    binding.focused_tests = collect_focused_tests(raw);
    for entry in &binding.focused_tests {
        if let FocusedTestEntry::Command(command) = entry {
            let declared = VerifierCommand {
                command: command.clone(),
                origin: VerifierOrigin::FocusedTests,
            };
            if !binding.verifier_commands.contains(&declared) {
                binding.verifier_commands.push(declared);
            }
        }
    }
    Ok(binding)
}

/// The first fenced ```` ```yaml ```` block, or the whole document when there is
/// none — a task written as bare front matter still parses.
///
/// The tag filter matters here in a way it does not for the JSON callers: a
/// task file is a whole markdown document and may open with a ```` ```bash ````
/// example, so "the first fenced block" would be the wrong block.
fn yaml_block(raw: &str) -> &str {
    archon_context::fenced::fenced_block_tagged(raw, "yaml").map_or(raw, |block| block.body)
}

/// `[A, B]` → `["A", "B"]`; `[]` → `[]`. Anything else is an error.
fn parse_flow_sequence(body: &str, source_path: &str) -> Result<Vec<String>> {
    let inner = body
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| {
            KnowledgeError::Traceability(format!(
                "{source_path}: `implements:` must be a single-line flow sequence \
                 like `implements: [REQ-DL-020]`; found `{body}`"
            ))
        })?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(inner
        .split(',')
        .map(|item| item.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect())
}

/// Backticked spans under `## Files Expected to Change` that look like paths.
///
/// The real corpus writes these as prose — *"Likely anchors: `a.rs`, `b.rs`"* —
/// so the paths have to be lifted out of the sentence. A span counts as a path
/// when it contains a separator or a file extension; a backticked prose word
/// does not.
fn collect_path_scopes(raw: &str) -> Vec<String> {
    let mut scopes: Vec<String> = Vec::new();
    for bullet in section_bullets(raw, "files expected to change") {
        for caps in backtick_re().captures_iter(&bullet) {
            let span = caps[1].trim().replace('\\', "/");
            if looks_like_path(&span) && !scopes.contains(&span) {
                scopes.push(span);
            }
        }
    }
    scopes
}

fn looks_like_path(span: &str) -> bool {
    if span.is_empty() || span.contains(char::is_whitespace) {
        return false;
    }
    if span.contains('/') {
        return true;
    }
    // A bare filename: `data_lake.rs`. A backticked prose word has no
    // extension, and `status=passed` has no dot.
    match span.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=4).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

fn collect_focused_tests(raw: &str) -> Vec<FocusedTestEntry> {
    section_bullets(raw, "focused tests")
        .into_iter()
        .map(|bullet| classify_focused_test(&bullet))
        .collect()
}

fn classify_focused_test(bullet: &str) -> FocusedTestEntry {
    for caps in backtick_re().captures_iter(bullet) {
        let span = normalize_command(&caps[1]);
        let Some(first) = span.split_whitespace().next() else {
            continue;
        };
        if KNOWN_RUNNERS.contains(&first) {
            return FocusedTestEntry::Command(span);
        }
    }
    FocusedTestEntry::Prose(bullet.trim().to_string())
}

/// Bullets under a `##`-level heading, matched case-insensitively.
fn section_bullets(raw: &str, heading: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut inside = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let found = rest.trim_start_matches('#').trim();
            // A heading that is a PREFIX of the requested one counts. Authors
            // write `## Files Expected` where the contract says `## Files
            // Expected to Change`, and an exact match silently yields zero
            // bullets — so the task parses as owning NOTHING and every path it
            // declared is invisible. Observed live: one generated spec in three
            // used the short form, listing four real files that the traceability
            // report then reported as "declares no paths".
            //
            // Prefix in this direction only, so `Files Forbidden…` can never
            // satisfy a request for `Files Expected…`; the requested heading is
            // always the longer, fully-qualified one.
            inside = found.eq_ignore_ascii_case(heading)
                || (!found.is_empty()
                    && heading.len() > found.len()
                    && heading.to_ascii_lowercase().starts_with(&found.to_ascii_lowercase())
                    && heading.as_bytes().get(found.len()) == Some(&b' '));
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = item.trim();
            if !item.is_empty() {
                items.push(item.to_string());
            }
        }
    }
    items
}

/// Collapse internal whitespace so a declared command and a recorded one can be
/// compared for equality without either being reformatted.
pub fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;

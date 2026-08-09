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
///
/// A task's own `required_tools:` extends it for that task — see
/// [`classify_focused_test`]. That is what keeps this list from having to
/// anticipate every tool a generated spec might legitimately invoke.
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
    /// Tools the task declared it needs, from `required_tools:`.
    ///
    /// Load-bearing for classification, not documentation: a first token the
    /// task itself declared as a required tool is an invocation it intends to
    /// run, which is the same evidence [`KNOWN_RUNNERS`] supplies for the
    /// common runners. `serde(default)` so bindings written before this field
    /// still deserialise.
    #[serde(default)]
    pub required_tools: Vec<String>,
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

fn required_tools_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\s*required_tools:\s*(?P<body>.*)$")
            .expect("required_tools regex is a literal")
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
        if let Some(caps) = required_tools_re().captures(line) {
            // Parsed leniently, unlike `implements:`. A malformed tool list
            // costs a little classification precision; refusing the whole task
            // file over it would lose the requirement claims too, which are the
            // thing traceability actually exists to read.
            binding.required_tools =
                parse_flow_sequence(caps["body"].trim(), source_path).unwrap_or_default();
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
    binding.focused_tests = collect_focused_tests(raw, &binding.required_tools);
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
fn yaml_block(raw: &str) -> &str {
    let Some(open) = raw.find("```yaml") else {
        return raw;
    };
    let after = open + "```yaml".len();
    let rest = &raw[after..];
    match rest.find("```") {
        Some(close) => &rest[..close],
        None => rest,
    }
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

fn collect_focused_tests(raw: &str, declared_tools: &[String]) -> Vec<FocusedTestEntry> {
    let mut entries: Vec<FocusedTestEntry> = section_bullets(raw, "focused tests")
        .into_iter()
        .map(|bullet| classify_focused_test(&bullet, declared_tools))
        .collect();
    entries.extend(fenced_commands(raw, declared_tools));
    entries
}

/// Commands written inside a fenced block under `## Focused Tests`.
///
/// A fenced block is a reasonable way to write a list of commands and only this
/// reader disagreed, which turned into real damage: a run's self-check counted
/// "prose entries", an agent drove that count to zero by un-bulleting, and six
/// of fifteen specs ended with every command invisible — unable to prove a
/// single requirement they claimed. Reading the block removes the incentive
/// entirely, and it is strictly more permissive: nothing that parsed before
/// stops parsing.
///
/// Only lines whose first token is a runner count, so prose inside the block —
/// comments, headings, continuation text — is ignored rather than misread as a
/// command.
fn fenced_commands(raw: &str, declared_tools: &[String]) -> Vec<FocusedTestEntry> {
    let mut commands = Vec::new();
    let mut inside_section = false;
    let mut inside_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            // A fence only matters within the section; closing one outside it
            // must not flip the flag on.
            if inside_section {
                inside_fence = !inside_fence;
            }
            continue;
        }
        if !inside_fence && let Some(rest) = trimmed.strip_prefix('#') {
            inside_section = headings_match(rest.trim_start_matches('#').trim(), "focused tests");
            continue;
        }
        if !inside_section || !inside_fence {
            continue;
        }
        let command = normalize_command(trimmed);
        let Some(first) = command.split_whitespace().next() else {
            continue;
        };
        if is_runner(first, declared_tools) {
            commands.push(FocusedTestEntry::Command(command));
        }
    }
    commands
}

/// A first token that means "a shell will execute this", for this task.
fn is_runner(first: &str, declared_tools: &[String]) -> bool {
    KNOWN_RUNNERS.contains(&first)
        || declared_tools
            .iter()
            .any(|tool| tool.trim().eq_ignore_ascii_case(first))
}

/// Classify one bullet against the common runners *and* the tools this task
/// declared.
///
/// The declared-tools half is what stops a correct spec being read as prose.
/// Observed live: five specs ran `lizard -l rust -C 15 -L 50 -a 5 …` to enforce
/// the function-length and cyclomatic-complexity budgets their own acceptance
/// criteria set, and every one of them declared `required_tools: [cargo, bash,
/// lizard]`. `lizard` is not a common runner, so those five entries classified
/// as prose and the complexity budget went unverified — while the file-size
/// budget, which happened to run through `bash`, was enforced. The task had
/// already said what it needed; nothing was reading it.
///
/// Extending per task rather than by growing [`KNOWN_RUNNERS`] is the point.
/// The list cannot anticipate every tool a generated spec may legitimately
/// invoke, and a spec naming a tool it also declares is exactly the evidence
/// that the span is an invocation rather than a described CLI fragment. It also
/// keeps tool names out of this engine, which must stay ignorant of any
/// particular project's toolchain.
fn classify_focused_test(bullet: &str, declared_tools: &[String]) -> FocusedTestEntry {
    for caps in backtick_re().captures_iter(bullet) {
        let span = normalize_command(&caps[1]);
        let Some(first) = span.split_whitespace().next() else {
            continue;
        };
        if is_runner(first, declared_tools) {
            return FocusedTestEntry::Command(span);
        }
    }
    FocusedTestEntry::Prose(bullet.trim().to_string())
}

/// Bullets under a `##`-level heading, matched case-insensitively.
/// Whether a heading found in the document answers the requested one.
///
/// Equal, or one a whole-word prefix of the other. See the call site for the
/// two live failures this exists for.
fn headings_match(found: &str, requested: &str) -> bool {
    if found.is_empty() {
        return false;
    }
    let found = found.to_ascii_lowercase();
    let requested = requested.to_ascii_lowercase();
    if found == requested {
        return true;
    }
    let (shorter, longer) = if found.len() < requested.len() {
        (&found, &requested)
    } else {
        (&requested, &found)
    };
    // `as_bytes().get(..) == Some(&b' ')` rather than `chars().nth`: the prefix
    // is ASCII-compared already, and a space is one byte in UTF-8, so this
    // cannot split a multi-byte character.
    longer.starts_with(shorter.as_str()) && longer.as_bytes().get(shorter.len()) == Some(&b' ')
}

fn section_bullets(raw: &str, heading: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut inside = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let found = rest.trim_start_matches('#').trim();
            // Either heading may be a prefix of the other, because authors go
            // wrong in both directions and both directions silently yield zero
            // bullets.
            //
            // Shorter than requested: `## Files Expected` where the contract
            // says `## Files Expected to Change`. The task then parses as
            // owning NOTHING and every path it declared is invisible. Observed
            // live: one generated spec in three used the short form, listing
            // four real files the report then called "declares no paths".
            //
            // Longer than requested: `## Focused Tests and Evidence` where the
            // contract says `## Focused Tests`. Observed live in the same
            // corpus — "exact-heading parsing found three fatal section-name
            // defects (`Focused Tests and …` is ignored)" — and the writers
            // spent a remediation round renaming headings to satisfy this
            // parser. A reader that forces the document to change is the wrong
            // way round.
            //
            // The boundary check is what keeps this from being a substring
            // match: the shorter must end where the longer has a space, so
            // `Files Forbidden…` can never satisfy `Files Expected…` and
            // `Focused Testing` can never satisfy `Focused Tests`.
            inside = headings_match(found, heading);
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

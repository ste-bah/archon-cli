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
//! `implements:` is a YAML sequence, in flow (`[REQ-AB-020, REQ-AB-021]`) or
//! block form. A malformed value is an error naming the file — not an empty
//! list. A task that
//! claims nothing and a task whose claim could not be read are different facts,
//! and collapsing them is how an unclaimed requirement disappears from a report.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::errors::{KnowledgeError, Result};

use self::sections::{headings_match, section_bullets};

mod sections;

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
/// First tokens that mean "a shell will execute this".
///
/// Shared with the topology lint so the two readers of a `## Focused Tests`
/// section cannot drift: they were byte-identical copies, and widening one
/// alone left the task universe recording no focused tests while the lint
/// accepted them.
///
/// The POSIX entries matter because a PRD may require portable checks that
/// invoke no toolchain. Listing only toolchains classified every such bullet as
/// prose, so the universe declared no focused tests, the v3 author brief said
/// "no task declares any focused test", and the authored workflow passed none.
pub const KNOWN_RUNNERS: &[&str] = &[
    // Language toolchains and shells.
    "archon",
    "bash",
    "cargo",
    "deno",
    "go",
    "gradle",
    "just",
    "make",
    "mvn",
    "node",
    "npm",
    "pnpm",
    "pytest",
    "python",
    "python3",
    "sh",
    "tox",
    "yarn",
    // Portable POSIX checks.
    "awk",
    "cmp",
    "diff",
    "find",
    "grep",
    "head",
    "jq",
    "od",
    "printf",
    "sed",
    "sha256sum",
    "shasum",
    "stat",
    "tail",
    "test",
    "wc",
    "[",
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

fn canonical_task_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^TASK-[A-Z0-9]+-[0-9]{3}$").expect("canonical task id regex is literal")
    })
}

fn task_id_from_source_path(source_path: &str) -> Option<String> {
    let stem = std::path::Path::new(source_path).file_stem()?.to_str()?;
    if !stem.starts_with("TASK-") {
        return None;
    }
    let mut parts = stem.split('-');
    let candidate = format!("{}-{}-{}", parts.next()?, parts.next()?, parts.next()?);
    canonical_task_id_re()
        .is_match(&candidate)
        .then_some(candidate)
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

    let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml_block(raw)).map_err(|_| {
        if let Some(body) = yaml_field_body(yaml_block(raw), "implements") {
            KnowledgeError::Traceability(format!(
                "{source_path}: `implements:` must be a single-line flow sequence \
                 like `implements: [REQ-X-001]`; found `{body}`"
            ))
        } else {
            KnowledgeError::Traceability(format!(
                "{source_path}: no `task_id:` in the task's yaml block"
            ))
        }
    })?;
    let mapping = yaml.as_mapping();
    let field = |name: &str| {
        mapping.and_then(|mapping| mapping.get(serde_yaml_ng::Value::String(name.to_string())))
    };
    binding.task_id = field("task_id")
        .and_then(serde_yaml_ng::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(implements) = field("implements") {
        binding.implements = parse_string_sequence(implements).ok_or_else(|| {
            let body = yaml_field_body(yaml_block(raw), "implements").unwrap_or_default();
            KnowledgeError::Traceability(format!(
                "{source_path}: `implements:` must be a single-line flow sequence \
                 like `implements: [REQ-X-001]`; found `{body}`"
            ))
        })?;
    }
    // Malformed required_tools remains lenient: losing classification precision
    // must not hide the requirement claims this reader exists to expose.
    binding.required_tools = field("required_tools")
        .and_then(parse_string_sequence)
        .unwrap_or_default();
    collect_typed_verifiers(&yaml, &mut binding.verifier_commands);

    if binding.task_id.is_empty() {
        return Err(KnowledgeError::Traceability(format!(
            "{source_path}: no `task_id:` in the task's yaml block"
        )));
    }
    if !canonical_task_id_re().is_match(&binding.task_id) {
        return Err(KnowledgeError::Traceability(format!(
            "{source_path}: task_id '{}' is noncanonical; use exact TASK-<AREA>-<NNN> with no trailing slug",
            binding.task_id
        )));
    }
    if let Some(filename_task_id) = task_id_from_source_path(source_path)
        && filename_task_id != binding.task_id
    {
        return Err(KnowledgeError::Traceability(format!(
            "{source_path}: task_id '{}' does not match filename task id '{}'; rename the file or correct task_id so both use the same canonical TASK-<AREA>-<NNN>",
            binding.task_id, filename_task_id
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
///
/// The tag filter matters here in a way it does not for the JSON callers: a
/// task file is a whole markdown document and may open with a ```` ```bash ````
/// example, so "the first fenced block" would be the wrong block.
fn yaml_block(raw: &str) -> &str {
    archon_context::fenced::fenced_block_tagged(raw, "yaml").map_or(raw, |block| block.body)
}

fn parse_string_sequence(value: &serde_yaml_ng::Value) -> Option<Vec<String>> {
    let items = value.as_sequence()?;
    items
        .iter()
        .map(|item| item.as_str().map(str::to_string))
        .collect()
}

fn yaml_field_body<'a>(yaml: &'a str, field: &str) -> Option<&'a str> {
    yaml.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(':')?;
        (key == field).then_some(value.trim())
    })
}

fn collect_typed_verifiers(yaml: &serde_yaml_ng::Value, output: &mut Vec<VerifierCommand>) {
    match yaml {
        serde_yaml_ng::Value::Mapping(mapping) => {
            for (key, value) in mapping {
                if key.as_str() == Some("typed_verifier_command")
                    && let Some(command) = value.as_str()
                {
                    output.push(VerifierCommand {
                        command: normalize_command(command),
                        origin: VerifierOrigin::TypedVerifier,
                    });
                } else {
                    collect_typed_verifiers(value, output);
                }
            }
        }
        serde_yaml_ng::Value::Sequence(items) => {
            for item in items {
                collect_typed_verifiers(item, output);
            }
        }
        _ => {}
    }
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
    // A bare filename: `module.rs`. A backticked prose word has no
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
        let Some(first) = command.trim_start_matches('!').split_whitespace().next() else {
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
        let Some(first) = span.trim_start_matches('!').split_whitespace().next() else {
            continue;
        };
        if is_runner(first, declared_tools) {
            return FocusedTestEntry::Command(span);
        }
    }
    FocusedTestEntry::Prose(bullet.trim().to_string())
}

/// Collapse internal whitespace so a declared command and a recorded one can be
/// compared for equality without either being reformatted.
pub fn normalize_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bullets under a `##`-level heading, matched case-insensitively.
/// Whether a heading found in the document answers the requested one.
///
/// Equal, or one a whole-word prefix of the other. See the call site for the
/// two live failures this exists for.

#[cfg(test)]
mod tests;

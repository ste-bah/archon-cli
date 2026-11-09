//! Reading the binding a task declares: which requirements it claims, which
//! paths it will touch, and which verifier commands it names.
//!
//! This reader builds traceability bindings; scheduling and contract semantics
//! remain in the task universe. Both readers share the command grammar.
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

#[path = "../../../archon-workflow/src/task_universe_focused_commands.rs"]
pub mod focused_commands;
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
    /// A shell command declared by the shared command grammar.
    Command(String),
    /// Everything else: a description of a test, not an invocation of one.
    Prose(String),
}

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
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Every `## Focused Tests` bullet, classified.
    pub focused_tests: Vec<FocusedTestEntry>,
    /// Commands that could actually be matched against a recorded run.
    pub verifier_commands: Vec<VerifierCommand>,
}

impl TaskBinding {
    /// Focused-test bullets that are descriptions rather than commands.
    /// Replace executable decisions with the authoritative captured list.
    pub fn use_focused_commands(&mut self, commands: &[String]) {
        self.focused_tests.retain(|entry| match entry {
            FocusedTestEntry::Command(_) => false,
            FocusedTestEntry::Prose(text) => !commands
                .iter()
                .any(|command| text.trim() == command || text.contains(&format!("`{command}`"))),
        });
        self.focused_tests
            .extend(commands.iter().cloned().map(FocusedTestEntry::Command));
        self.verifier_commands
            .retain(|entry| entry.origin != VerifierOrigin::FocusedTests);
        self.verifier_commands
            .extend(commands.iter().map(|command| VerifierCommand {
                command: command.clone(),
                origin: VerifierOrigin::FocusedTests,
            }));
    }

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

/// Parse one task file.
///
/// Errors when `task_id:` is absent or `implements:` is present in a shape this
/// reader does not accept. Absent `implements:` is *not* an error — a task
/// written before D5 claims nothing, and that is a true statement about it.
pub fn parse_task_binding(raw: &str, source_path: &str) -> Result<TaskBinding> {
    // A caller with a captured site supplies its search path explicitly.
    // Without one, only declarations and bound repository paths participate.
    parse_task_binding_with_path(raw, source_path, std::ffi::OsStr::new(""))
}

pub fn parse_task_binding_with_path(
    raw: &str,
    source_path: &str,
    toolchain_path: &std::ffi::OsStr,
) -> Result<TaskBinding> {
    let mut binding = TaskBinding {
        source_path: source_path.to_string(),
        ..TaskBinding::default()
    };

    let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml_block(raw)).map_err(|error| {
        KnowledgeError::Traceability(format!(
            "{source_path}: YAML parse error in task yaml block: {error}"
        ))
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
    // Malformed capability metadata must not discard the requirement claims.
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
    binding.focused_tests =
        collect_focused_tests(raw, &binding.required_tools, source_path, toolchain_path);
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

/// Paths under `## Files Expected to Change`, using the shared declaration
/// grammar so both workflow gates resolve each bullet the same way.
fn collect_path_scopes(raw: &str) -> Vec<String> {
    let mut scopes: Vec<String> = Vec::new();
    for bullet in section_bullets(raw, "files expected to change") {
        for path in archon_write_plan::declared_paths::declared_file_paths(&bullet) {
            if !scopes.contains(&path) {
                scopes.push(path);
            }
        }
    }
    scopes
}

fn collect_focused_tests(
    raw: &str,
    declared_tools: &[String],
    source: &str,
    toolchain_path: &std::ffi::OsStr,
) -> Vec<FocusedTestEntry> {
    let raw = focused_section(raw);
    let raw = raw.as_str();
    let mut entries: Vec<FocusedTestEntry> = section_bullets(raw, "focused tests")
        .into_iter()
        .map(|bullet| classify_focused_test(&bullet, declared_tools, source, toolchain_path))
        .collect();
    entries.extend(fenced_commands(raw, declared_tools, source, toolchain_path));
    entries
}

fn focused_section(raw: &str) -> String {
    let mut depth = None;
    let mut fenced = false;
    let mut flattened = String::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if depth.is_some_and(|parent| level > parent) {
                flattened.push('\n');
                continue;
            }
            depth = headings_match(trimmed.trim_start_matches('#').trim(), "focused tests")
                .then_some(level);
        }
        flattened.push_str(line);
        flattened.push('\n');
    }
    flattened
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
fn fenced_commands(
    raw: &str,
    tools: &[String],
    source: &str,
    toolchain_path: &std::ffi::OsStr,
) -> Vec<FocusedTestEntry> {
    let mut commands = Vec::new();
    let mut inside_section = false;
    let mut inside_fence = false;
    let mut pending: Option<String> = None;
    let mut section_depth = 0;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            // A fence only matters within the section; closing one outside it
            // must not flip the flag on.
            if let Some(command) = pending.take() {
                if let Some(command) = focused_commands::focused_test_command_with_path(
                    &command,
                    tools,
                    &focused_commands::repository_root(source),
                    toolchain_path,
                ) {
                    commands.push(FocusedTestEntry::Command(normalize_command(&command)));
                }
            }
            if inside_section {
                inside_fence = !inside_fence;
            }
            continue;
        }
        if !inside_fence && let Some(rest) = trimmed.strip_prefix('#') {
            let depth = trimmed.chars().take_while(|c| *c == '#').count();
            if inside_section && depth > section_depth {
                continue;
            }
            inside_section = headings_match(rest.trim_start_matches('#').trim(), "focused tests");
            section_depth = depth;
            continue;
        }
        if !inside_section || !inside_fence {
            continue;
        }
        let command = match pending.take() {
            Some(mut command) => {
                command.push('\n');
                command.push_str(line);
                command
            }
            None if trimmed.is_empty() || trimmed.starts_with('#') => continue,
            None => trimmed.to_string(),
        };
        if focused_commands::continues(&command) {
            pending = Some(command);
        } else if let Some(command) = focused_commands::focused_test_command_with_path(
            &command,
            tools,
            &focused_commands::repository_root(source),
            toolchain_path,
        ) {
            commands.push(FocusedTestEntry::Command(normalize_command(&command)));
        }
    }
    if let Some(command) = pending.and_then(|command| {
        focused_commands::focused_test_command_with_path(
            &command,
            tools,
            &focused_commands::repository_root(source),
            toolchain_path,
        )
    }) {
        commands.push(FocusedTestEntry::Command(normalize_command(&command)));
    }
    commands
}

fn classify_focused_test(
    bullet: &str,
    tools: &[String],
    source: &str,
    toolchain_path: &std::ffi::OsStr,
) -> FocusedTestEntry {
    match focused_commands::focused_test_command_with_path(
        bullet,
        tools,
        &focused_commands::repository_root(source),
        toolchain_path,
    ) {
        Some(command) => FocusedTestEntry::Command(normalize_command(&command)),
        None => FocusedTestEntry::Prose(bullet.trim().to_string()),
    }
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

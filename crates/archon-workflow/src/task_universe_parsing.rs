//! Parse one decomposed-PRD `TASK-*.md` file into a task-universe record.
//!
//! # Why this parser fails loudly
//!
//! It used to not. A file that did not match the expected shape still produced
//! a record: the task id came from the filename, the title from the first `#`
//! heading, and every contract field — `depends_on`, `blocks`, `required_tools`,
//! `required_env_keys`, `deliverable_contracts` — came back empty. The run then
//! proceeded with an empty dependency graph, no tool enforcement, and no
//! deliverable gate, and reported success. Nothing distinguished "this task
//! genuinely declares no dependencies" from "this parser understood nothing in
//! this file", which is the most expensive kind of silence a planner can emit.
//!
//! So: a task file must carry a fenced ```yaml block (or `---` front matter)
//! that parses as a mapping and declares every key in
//! [`REQUIRED_TASK_KEYS`]. Anything else is a [`WorkflowError::SpecInvalid`]
//! naming the file path and the specific keys that are missing. There is no
//! partial-parse path left.

#[cfg(test)]
use std::fs;
use std::path::Path;

use crate::error::{WorkflowError, WorkflowResult};

#[path = "task_universe_contract_paths.rs"]
mod contract_paths;
use contract_paths::expand_plural_artifact_paths;

use super::{
    WorkflowV2TaskUniverseTask, canonical_task_id_from_ref, sorted_unique, task_id_from_task_path,
};

/// Keys a task file must declare for its record to mean anything.
///
/// These are the contract-bearing keys of the decomposed-PRD task standard.
/// Each one silently changes what a run *does* when it is absent rather than
/// empty: the dependency graph (`depends_on`, `blocks`), tool enforcement
/// (`required_tools`), environment gating (`required_env_keys`), the deliverable
/// gate (`deliverable_contracts`), and task identity/reporting (`task_id`,
/// `title`, `complexity`, `status`). Declaring one empty is a statement; leaving
/// it out is not, and this parser refuses to guess which was meant.
///
/// `implements` is here for the same reason and one more. It is the only link
/// between a task and the PRD requirements it is answerable for, and the
/// requirement-coverage lint is a pure set difference over it: an absent
/// `implements` silently shrinks the claimed set, so a requirement nobody
/// implements and a requirement nobody *declared* would report identically.
/// `implements: []` is the honest declaration an audit or review task makes, and
/// it is accepted; omission is not.
///
/// Purely informational keys the standard also carries — `prd`, `domain`,
/// `workstream`, `source_sections` — are deliberately *not* required: their
/// absence changes nothing a run executes, so demanding them would only reject
/// otherwise-valid files. `prd` and `shared_append_target_files` are read when
/// present (see [`parse_task_file`]) but stay optional for the same reason: the
/// coverage lint falls back to the directory name when no task names a PRD, and
/// an absent shared-append list means "nothing is shared", which is the safe
/// reading rather than a guess.
pub(super) const REQUIRED_TASK_KEYS: &[&str] = &[
    "task_id",
    "title",
    "complexity",
    "status",
    "depends_on",
    "blocks",
    "implements",
    "required_env_keys",
    "required_tools",
    "deliverable_contracts",
];

pub fn parse_task_file(path: &Path, raw: &str) -> WorkflowResult<WorkflowV2TaskUniverseTask> {
    let path_task_id = task_id_from_task_path(path).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow could not parse canonical task_id from {}",
            path.display()
        ))
    })?;
    let metadata = task_metadata(path, raw)?;
    require_declared_keys(path, &metadata)?;

    let field_task_id = metadata_string(&metadata, "task_id").ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow has a non-string task_id in {}",
            path.display()
        ))
    })?;
    let canonical_task_id = canonical_task_id_from_ref(&field_task_id).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow has invalid task_id '{field_task_id}' in {}",
            path.display()
        ))
    })?;
    if canonical_task_id != path_task_id {
        return Err(WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow task_id '{canonical_task_id}' does not match filename task id '{path_task_id}' in {}",
            path.display()
        )));
    }

    // `deliverable_contracts:` with nothing after it is YAML null, which reads
    // as a declared-but-empty list. The loud check above is about a key being
    // *absent*; emptiness is a statement the author is allowed to make.
    let deliverable_contracts = match &metadata["deliverable_contracts"] {
        serde_json::Value::Null => Vec::new(),
        declared => serde_json::from_value(expand_plural_artifact_paths(declared)).map_err(|error| {
            WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow has an unreadable deliverable_contracts block in {}: {error}",
                path.display()
            ))
        })?,
    };

    Ok(WorkflowV2TaskUniverseTask {
        canonical_task_id,
        aliases: Vec::new(),
        source_path: path.display().to_string(),
        dependency_ids: sorted_unique(metadata_strings(&metadata, "depends_on")),
        blocks_ids: sorted_unique(metadata_strings(&metadata, "blocks")),
        // The declared `title` wins over the `#` heading. The heading
        // conventionally carries the task id as a prefix before the title text,
        // so reading it instead produced a title that never matched the file's
        // own declaration.
        title: metadata_string(&metadata, "title"),
        complexity: metadata_string(&metadata, "complexity"),
        status: metadata_string(&metadata, "status"),
        // Required above, so an empty vector here means the author wrote
        // `implements: []` — a task answerable for no requirement — and never
        // that the key was missing.
        implements: sorted_unique(metadata_strings(&metadata, "implements")),
        // Optional, and empty by default: a path is exclusive unless a task
        // names it here, so nothing becomes concurrently written by omission.
        shared_append_target_files: sorted_unique(metadata_strings(
            &metadata,
            "shared_append_target_files",
        )),
        acceptance_criteria: declared_task_section_items(raw, "acceptance criteria"),
        // Additive: the per-task adversarial reviewer reads these verbatim.
        adversarial_review_notes: declared_task_section_items(raw, "adversarial review notes"),
        files_expected_to_change: declared_task_section_items(raw, "files expected to change"),
        files_forbidden_to_change: declared_task_section_items(raw, "files forbidden to change"),
        artifact_requirements: declared_task_artifact_requirements(raw, &metadata),
        required_env_keys: sorted_unique(metadata_strings(&metadata, "required_env_keys")),
        required_tools: sorted_unique(metadata_strings(&metadata, "required_tools")),
        deliverable_contracts,
    })
}

/// Extract and parse the file's YAML block. Errors rather than returning an
/// empty mapping — the empty mapping is what made an unparseable file look like
/// a task with no declarations.
fn task_metadata(path: &Path, raw: &str) -> WorkflowResult<serde_json::Value> {
    let Some(yaml) = yaml_block(raw) else {
        return Err(WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow found no fenced ```yaml task block in {}; a task file must declare {}",
            path.display(),
            REQUIRED_TASK_KEYS.join(", ")
        )));
    };
    let metadata: serde_json::Value = serde_yaml_ng::from_str(&yaml).map_err(|error| {
        WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow could not parse the task YAML block in {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_object() {
        return Err(WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow task YAML block in {} is not a mapping",
            path.display()
        )));
    }
    Ok(metadata)
}

fn require_declared_keys(path: &Path, metadata: &serde_json::Value) -> WorkflowResult<()> {
    let missing = REQUIRED_TASK_KEYS
        .iter()
        .filter(|key| metadata.get(**key).is_none())
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(WorkflowError::SpecInvalid(format!(
        "generated decomposed PRD workflow task file {} is missing required key(s): {}",
        path.display(),
        missing.join(", ")
    )))
}

/// The fenced ```yaml / ```yml block, or leading `---` front matter.
fn yaml_block(raw: &str) -> Option<String> {
    let mut yaml = String::new();
    let mut in_yaml = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if !in_yaml {
            if trimmed == "```yaml" || trimmed == "```yml" || (trimmed == "---" && yaml.is_empty())
            {
                in_yaml = true;
            }
            continue;
        }
        if trimmed == "```" || trimmed == "---" {
            return Some(yaml);
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    // An opened-but-never-closed fence is not a block: treat it as absent so the
    // caller reports the loud "no yaml block" error rather than parsing a
    // truncated mapping.
    None
}

fn metadata_string(metadata: &serde_json::Value, field: &str) -> Option<String> {
    metadata
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn metadata_strings(metadata: &serde_json::Value, field: &str) -> Vec<String> {
    match metadata.get(field) {
        Some(serde_json::Value::String(value)) => vec![value.clone()],
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Union `.archon/project.json` into one task — **environment keys only**.
///
/// The two capability lists look symmetric and are not, because the branch
/// contract treats them differently:
///
/// - A `required_env_keys` entry is **proven**. The branch reports it checked,
///   and a key nothing uses costs a `checked_keys` line and nothing else. That
///   carries no obligation, so granting one project-wide is free.
/// - A `required_tools` entry is **exercised**. An accepted result must show an
///   actual invocation of every declared tool, and declaring a no-op is refused
///   outright while any tool is declared. That is an obligation, and granting
///   one project-wide imposes it on every task.
///
/// So the project manifest is the right home for the first and the wrong home
/// for the second. It used to carry both: the manifest hoisted the ambient
/// toolchain (`archon`, `bash`, `cargo`, `python3` on the reference project)
/// and this function unioned it onto every task, which made all four an
/// invocation obligation for tasks with no reason to run any of them. Every
/// branch was then trapped — accepted required four invocations it had no work
/// for, and noop was forbidden because tools were declared — so a 15-task run
/// produced nothing (#163, failure 3).
///
/// A task's tools now come only from that task. The under-declared-runner
/// problem `workflow lint`'s `## declared capabilities` section reports is
/// real, and its fix is in the task that invokes the runner. `required_tools`
/// and `tool_bundles` in an existing manifest are read by nothing and left in
/// place; `sync-capabilities` reports them as inert rather than removing them.
///
/// `required_env_keys` has since gone the same way, because "granting one
/// project-wide is free" held only while every hoisted key could be satisfied.
/// A key that cannot be does not cost a `checked_keys` line — it fails the
/// branch. Observed live: `AHDM_REVIEW_RUN_ID` is a per-review identifier
/// declared by one task, hoisted into the manifest as the union of all fifteen
/// tasks' keys, and inherited by every one of them. It then failed a TDL-020
/// verification branch outright, on work that had no relationship to it, while
/// the run's real gaps were being fixed elsewhere.
///
/// So both fields are now task-only and this function reads neither. A key
/// genuinely needed by every task is declared by every task, which is the same
/// trade already accepted for tools: explicit repetition over an inherited
/// obligation nobody can see from the task file.
pub fn merge_project_capabilities(
    task: &mut WorkflowV2TaskUniverseTask,
    task_path: &Path,
) -> WorkflowResult<()> {
    // Both capability fields are task-only; nothing is read from the manifest.
    // Kept as a call site so the boundary stays visible where the universe is
    // assembled, rather than silently disappearing into the parser.
    let _ = (task, task_path);
    Ok(())
}

/// Explicit artifact declarations in a TASK-*.md file: list items under an
/// "Artifact Requirements" heading, plus an `artifact_requirements:` list in the
/// YAML block. These declarations flow into every implementation/remediation
/// item for the task, so the agent that owns the task is always instructed to
/// produce them.
fn declared_task_artifact_requirements(raw: &str, metadata: &serde_json::Value) -> Vec<String> {
    let mut requirements = metadata_strings(metadata, "artifact_requirements");
    requirements.extend(declared_task_section_items(raw, "artifact requirements"));
    sorted_unique(
        requirements
            .into_iter()
            .map(|value| value.trim().trim_matches('`').trim().to_string())
            .collect(),
    )
}

fn declared_task_section_items(raw: &str, section: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut in_section = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            in_section = heading
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(item) = list_item_text(trimmed) {
            if !item.is_empty() {
                items.push(item.to_string());
            }
        }
    }
    sorted_unique(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 17 real `TASK-*.md` files of `PRD-TRADING-DATA-LAKE-AHDM-001`,
    /// checked in verbatim, and an expectation table transcribed from them by a
    /// separate reader (see the generator note in the fixture directory). The
    /// parser was previously only ever exercised against task files this
    /// repository wrote itself — a closed loop in which a field the parser
    /// ignored was also a field no test file declared.
    /// The real decomposed-PRD corpus, which lives once at the workspace root
    /// because several crates' tests parse the same seventeen task files. This
    /// crate sits two levels below it, hence the `../..`: copying the corpus in
    /// would let the two copies drift and the round-trip assertion would then
    /// be checking this crate against itself.
    fn fixture_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/prd-trading-data-lake-ahdm-001")
    }

    #[derive(serde::Deserialize)]
    struct ExpectedContract {
        kind: String,
        artifact_path: String,
    }

    #[derive(serde::Deserialize)]
    struct ExpectedTask {
        file: String,
        task_id: String,
        title: String,
        complexity: String,
        status: String,
        depends_on: Vec<String>,
        blocks: Vec<String>,
        required_env_keys: Vec<String>,
        required_tools: Vec<String>,
        deliverable_contracts: Vec<ExpectedContract>,
        acceptance_criteria: Vec<String>,
        files_expected_to_change: Vec<String>,
        files_forbidden_to_change: Vec<String>,
    }

    fn expected_tasks() -> Vec<ExpectedTask> {
        let path = fixture_root().join("expected-parse.json");
        let raw = fs::read_to_string(&path).expect("read expectation table");
        serde_json::from_str(&raw).expect("expectation table parses")
    }

    /// Every declared field of all 17 real task files, compared one field at a
    /// time so a failure names the task and the field rather than diffing two
    /// structs. Failures are collected, not short-circuited: the point is to
    /// report *every* field that does not round-trip.
    #[test]
    fn every_declared_field_of_all_seventeen_real_tasks_round_trips() {
        let root = fixture_root();
        let expected = expected_tasks();
        assert_eq!(expected.len(), 17, "the PRD decomposes into 17 tasks");
        let mut failures = Vec::new();

        for want in &expected {
            let path = root.join(&want.file);
            let raw = fs::read_to_string(&path).expect("read fixture task file");
            let got = match parse_task_file(&path, &raw) {
                Ok(parsed) => parsed,
                Err(error) => {
                    failures.push(format!("{}: parse failed: {error}", want.task_id));
                    continue;
                }
            };
            let contracts = got
                .deliverable_contracts
                .iter()
                .map(|contract| (contract.kind.clone(), contract.artifact_path.clone()))
                .collect::<Vec<_>>();
            let want_contracts = want
                .deliverable_contracts
                .iter()
                .map(|contract| (contract.kind.clone(), contract.artifact_path.clone()))
                .collect::<Vec<_>>();
            #[rustfmt::skip]
            let fields: Vec<(&str, String, String)> = vec![
                ("task_id", got.canonical_task_id.clone(), want.task_id.clone()),
                ("title", format!("{:?}", got.title), format!("{:?}", Some(&want.title))),
                ("complexity", format!("{:?}", got.complexity), format!("{:?}", Some(&want.complexity))),
                ("status", format!("{:?}", got.status), format!("{:?}", Some(&want.status))),
                ("depends_on", format!("{:?}", got.dependency_ids), format!("{:?}", want.depends_on)),
                ("blocks", format!("{:?}", got.blocks_ids), format!("{:?}", want.blocks)),
                ("required_tools", format!("{:?}", got.required_tools), format!("{:?}", want.required_tools)),
                ("required_env_keys", format!("{:?}", got.required_env_keys), format!("{:?}", want.required_env_keys)),
                ("deliverable_contracts", format!("{contracts:?}"), format!("{want_contracts:?}")),
                ("acceptance_criteria", format!("{:?}", got.acceptance_criteria), format!("{:?}", want.acceptance_criteria)),
                ("files_expected_to_change", format!("{:?}", got.files_expected_to_change), format!("{:?}", want.files_expected_to_change)),
                ("files_forbidden_to_change", format!("{:?}", got.files_forbidden_to_change), format!("{:?}", want.files_forbidden_to_change)),
            ];
            for (field, parsed, declared) in fields {
                if parsed != declared {
                    failures.push(format!(
                        "{} {field}:\n  parsed   {parsed}\n  declared {declared}",
                        want.task_id
                    ));
                }
            }
        }

        assert!(
            failures.is_empty(),
            "fields that did not round-trip:\n{}",
            failures.join("\n")
        );
    }

    /// The richest deliverable contract in the real set — 30-odd verifier
    /// fields. They deserialize into typed fields or they silently do not, and a
    /// contract that lost its thresholds gates nothing.
    #[test]
    fn the_richest_deliverable_contract_keeps_its_verifier_fields() {
        let path = fixture_root().join("TASK-TDL-080-coverage-matrix-command.md");
        let raw = fs::read_to_string(&path).expect("read fixture");
        let parsed = parse_task_file(&path, &raw).expect("parse fixture");
        let contract = &parsed.deliverable_contracts[0];
        assert!(contract.required_universe);
        assert_eq!(contract.registry_minimum_count, 200);
        assert_eq!(contract.minimum_count_fields.get("row_count"), Some(&200));
        assert_eq!(contract.payload_format.as_deref(), Some("jsonl"));
        assert_eq!(contract.observed_time_field.as_deref(), Some("timestamp"));
        assert_eq!(contract.series_overlap_min_rows, 5);
        assert_eq!(
            contract
                .registry_identity_fields
                .get("canonical_instrument"),
            Some(&"symbol".to_string())
        );
        assert_eq!(
            contract.required_fields,
            ["timestamp", "open", "high", "low", "close", "volume"]
        );
        assert_eq!(contract.registry_allowed_statuses, ["Healthy"]);
    }
}

#[path = "task_universe_list_items.rs"]
mod list_items;
use list_items::list_item_text;

#[cfg(test)]
#[path = "task_universe_parsing_list_tests.rs"]
mod list_tests;

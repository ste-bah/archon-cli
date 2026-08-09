//! Derive `.archon/project.json` from a decomposed task set.
//!
//! `merge_project_capabilities` unions this manifest into every task before a
//! run, which makes it the right home for anything the *project* needs rather
//! than any one task: the toolchain its focused tests invoke, the credentials
//! its providers read. Nothing wrote it. It was read-only from the day it was
//! added, undocumented in every cookbook, and the one on disk had sat empty
//! since July — so each task had to redeclare everything, and any task that
//! forgot silently lost it. Measured when this landed: one 15-task set clean,
//! the next 2 of 15 under-declaring, an older 22-task set wrong in all 22.
//!
//! The derivation is deterministic and host-owned on purpose. Asking the
//! authoring agent to keep the manifest current would make it a prompt rule,
//! and a prompt rule is not an invariant — this codebase has re-learned that
//! often enough. Here the host reads what the tasks declare and what their
//! commands actually invoke, and writes the union.
//!
//! **It only ever adds.** A project accumulates PRDs, and a decomposition that
//! replaced the manifest would silently strip the capabilities an earlier PRD
//! depends on. Union is also safe in the other direction: a key that no task
//! uses costs a proof-of-checking, never a failure — the verification contract
//! states that a provider with no declared keys is satisfied by `checked_keys:
//! []`, and that where a contract permits success *or* unavailable, a
//! fail-closed unavailable result is a valid outcome rather than an
//! implementation failure.
//!
//! Nothing here knows any project's toolchain or any PRD's identifiers, and
//! nothing should: it reports what the task files themselves say.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use archon_workflow::task_universe::parsing::parse_task_file;
use archon_workflow::task_universe::task_files_under;

/// Schema tag written for a manifest this command creates.
const SCHEMA_VERSION: &str = "archon.project.capabilities.v1";

/// First tokens that mean "a shell will execute this", matching the
/// focused-test classifier so the two cannot disagree about what a command is.
const KNOWN_RUNNERS: &[&str] = &[
    "archon", "bash", "cargo", "deno", "go", "gradle", "just", "make", "mvn", "node", "npm",
    "pnpm", "pytest", "python", "python3", "sh", "tox", "yarn",
];

/// What a sync did, so the caller can report it without re-reading the file.
#[derive(Debug)]
pub(crate) struct CapabilitySync {
    pub(crate) manifest_path: std::path::PathBuf,
    pub(crate) added_tools: Vec<String>,
    pub(crate) added_env_keys: Vec<String>,
    pub(crate) created: bool,
    pub(crate) tasks_read: usize,
}

impl CapabilitySync {
    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        if self.tasks_read == 0 {
            out.push_str("no task files were read; the manifest was left untouched.\n");
            return out;
        }
        out.push_str(&format!(
            "{} task(s) read; {} {}\n",
            self.tasks_read,
            self.manifest_path.display(),
            if self.created { "created" } else { "updated" }
        ));
        if self.added_tools.is_empty() && self.added_env_keys.is_empty() {
            out.push_str("  already covered every tool and environment key the tasks name.\n");
            return out;
        }
        if !self.added_tools.is_empty() {
            out.push_str(&format!(
                "  added required_tools: {}\n",
                self.added_tools.join(", ")
            ));
        }
        if !self.added_env_keys.is_empty() {
            out.push_str(&format!(
                "  added required_env_keys: {}\n",
                self.added_env_keys.join(", ")
            ));
        }
        out.push_str("  nothing was removed; a manifest only ever grows.\n");
        out
    }
}

/// Union what `tasks_root` needs into the project manifest.
///
/// `project_root` is where `.archon/` lives. `dry_run` computes and reports
/// without writing, so a caller can see what a decomposition would change.
pub(crate) fn sync_capabilities(
    project_root: &Path,
    tasks_root: &Path,
    dry_run: bool,
) -> Result<CapabilitySync> {
    let mut tools: BTreeSet<String> = BTreeSet::new();
    let mut env_keys: BTreeSet<String> = BTreeSet::new();
    let mut tasks_read = 0usize;

    for path in task_files_under(tasks_root).context("reading the task directory")? {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        // A file the run itself would refuse contributes nothing. The lint
        // reports those; silently widening the manifest from a file nobody can
        // parse would be guessing.
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        tasks_read += 1;
        env_keys.extend(task.required_env_keys.iter().cloned());
        // Only what the commands actually invoke — deliberately NOT the task's
        // declared `required_tools`.
        //
        // The manifest merges into every task, so anything hoisted here is
        // granted to all of them. A task's declared tools are already scoped to
        // the task that needs them, and hoisting them would defeat that: run
        // against a real corpus, the naive version lifted six MCP tool names
        // out of the two tasks that declared them and handed them to all
        // fifteen. What belongs at project level is the ambient toolchain every
        // task assumes and no task thinks to declare, which is exactly what a
        // command's leading runner is.
        for command in focused_test_commands(&raw) {
            if let Some(first) = command.split_whitespace().next()
                && KNOWN_RUNNERS.contains(&first)
            {
                tools.insert(first.to_string());
            }
        }
    }

    let manifest_path = project_root.join(".archon").join("project.json");
    let existing = read_manifest(&manifest_path)?;
    let created = existing.is_none();
    let mut manifest = existing.unwrap_or_default();

    let added_tools = merge_list(&mut manifest, "required_tools", &tools);
    let added_env_keys = merge_list(&mut manifest, "required_env_keys", &env_keys);
    manifest
        .entry("schema_version".to_string())
        .or_insert_with(|| Value::String(SCHEMA_VERSION.to_string()));
    manifest
        .entry("tool_bundles".to_string())
        .or_insert_with(|| Value::Object(Map::new()));

    let sync = CapabilitySync {
        manifest_path: manifest_path.clone(),
        added_tools,
        added_env_keys,
        created,
        tasks_read,
    };

    if !dry_run && tasks_read > 0 && (created || !sync.added_tools.is_empty() || !sync.added_env_keys.is_empty())
    {
        if let Some(parent) = manifest_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(&Value::Object(manifest))?;
        fs::write(&manifest_path, format!("{body}\n"))
            .with_context(|| format!("writing {}", manifest_path.display()))?;
    }
    Ok(sync)
}

/// Existing manifest, or `None` when there is no file yet.
///
/// A malformed manifest is an error rather than an empty start: overwriting one
/// the operator hand-edited, because this could not parse it, is how a hand
/// edit disappears without anyone noticing.
fn read_manifest(path: &Path) -> Result<Option<Map<String, Value>>> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON; refusing to overwrite it", path.display()))?;
    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => anyhow::bail!(
            "{} is not a JSON object; refusing to overwrite it",
            path.display()
        ),
    }
}

/// Union `values` into `manifest[key]`, returning only what was newly added.
fn merge_list(manifest: &mut Map<String, Value>, key: &str, values: &BTreeSet<String>) -> Vec<String> {
    let mut existing: BTreeSet<String> = manifest
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let added: Vec<String> = values.difference(&existing).cloned().collect();
    existing.extend(values.iter().cloned());
    manifest.insert(
        key.to_string(),
        Value::Array(existing.into_iter().map(Value::String).collect()),
    );
    added
}

/// Backticked spans under `## Focused Tests`, heading matched with the same
/// whole-word-prefix tolerance the traceability reader uses.
fn focused_test_commands(raw: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut inside = false;
        // Commands inside a fenced block count too — the traceability reader
        // reads them, and a lint that disagreed would report "no runnable
        // tests" for a spec the engine parses fine.
        let mut in_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if inside {
                in_fence = !in_fence;
            }
            continue;
        }
        if in_fence {
            if inside && !trimmed.is_empty() {
                commands.push(trimmed.to_string());
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            inside = heading_matches(rest.trim_start_matches('#').trim(), "focused tests");
            continue;
        }
        if !inside {
            continue;
        }
        let mut rest = trimmed;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            let span = after[..close].trim();
            if !span.is_empty() {
                commands.push(span.to_string());
            }
            rest = &after[close + 1..];
        }
    }
    commands
}

fn heading_matches(found: &str, requested: &str) -> bool {
    let found = found.to_ascii_lowercase();
    if found == requested {
        return true;
    }
    let (shorter, longer) = if found.len() < requested.len() {
        (found.as_str(), requested)
    } else {
        (requested, found.as_str())
    };
    !shorter.is_empty()
        && longer.starts_with(shorter)
        && longer.as_bytes().get(shorter.len()) == Some(&b' ')
}

#[cfg(test)]
#[path = "workflow_capabilities_tests.rs"]
mod workflow_capabilities_tests;

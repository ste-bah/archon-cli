//! Derive `.archon/project.json` from a decomposed task set.
//!
//! `merge_project_capabilities` unions this manifest into every task before a
//! run, which makes it the right home for the credentials the project's
//! providers read. Nothing wrote it. It was read-only from the day it was
//! added, undocumented in every cookbook, and the one on disk had sat empty
//! since July — so each task had to redeclare everything, and any task that
//! forgot silently lost it. Measured when this landed: one 15-task set clean,
//! the next 2 of 15 under-declaring, an older 22-task set wrong in all 22.
//!
//! # Environment keys only, and why tools are not the same shape
//!
//! An environment key is **proven**: the branch reports it checked, and a key
//! nothing uses costs a proof-of-checking and never a failure — the
//! verification contract states that a provider with no declared keys is
//! satisfied by `checked_keys: []`, and that where a contract permits success
//! *or* unavailable, a fail-closed unavailable result is a valid outcome rather
//! than an implementation failure. Proof carries no obligation, so granting a
//! key to every task is free.
//!
//! A tool is **exercised**: an accepted result must show an actual invocation
//! of every tool the task declares, and while any tool is declared, declaring a
//! no-op is refused. That is an obligation, and the manifest merges into every
//! task, so hoisting a tool imposes it on all of them. This command used to
//! hoist the leading runner of each focused-test command for exactly that
//! reason — "the ambient toolchain every task assumes" — and on the reference
//! project it lifted `archon`, `bash`, `cargo`, `python3` into every task's
//! declaration. Each branch was then trapped: accepted demanded four
//! invocations it had no work for, and noop was forbidden because tools were
//! declared. A clean 15-task run produced nothing (#163, failure 3).
//!
//! So the derivation is now env keys only. Under-declared runners are still
//! reported — `workflow lint`'s `## declared capabilities` section names any
//! task whose commands use a runner the task never declared — and the fix
//! belongs in that task, where the obligation is scoped to the work that
//! actually incurs it.
//!
//! The derivation is deterministic and host-owned on purpose. Asking the
//! authoring agent to keep the manifest current would make it a prompt rule,
//! and a prompt rule is not an invariant — this codebase has re-learned that
//! often enough. Here the host reads what the tasks declare and writes the
//! union.
//!
//! **It only ever adds.** A project accumulates PRDs, and a decomposition that
//! replaced the manifest would silently strip the capabilities an earlier PRD
//! depends on. That applies to the now-inert `required_tools` and
//! `tool_bundles` keys too: an existing manifest keeps them untouched, and a
//! sync reports them as no longer merged rather than deleting what someone
//! hand-wrote.
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

/// Manifest keys a run no longer reads, reported so a hand edit to one is not
/// silently inert.
const INERT_TOOL_KEYS: &[&str] = &["required_tools", "tool_bundles"];

/// What a sync did, so the caller can report it without re-reading the file.
#[derive(Debug)]
pub(crate) struct CapabilitySync {
    pub(crate) manifest_path: std::path::PathBuf,
    pub(crate) added_env_keys: Vec<String>,
    /// Tool names already in the manifest under a key nothing merges any more.
    /// Left on disk untouched; named here so an operator sees they do nothing.
    pub(crate) inert_tools: Vec<String>,
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
        if self.added_env_keys.is_empty() {
            out.push_str("  already covered every environment key the tasks name.\n");
        } else {
            out.push_str(&format!(
                "  added required_env_keys: {}\n",
                self.added_env_keys.join(", ")
            ));
            out.push_str("  nothing was removed; a manifest only ever grows.\n");
        }
        if !self.inert_tools.is_empty() {
            out.push_str(&format!(
                "  note: {} in this manifest name {} and are no longer merged into \
                 any task; declare a tool in the task that invokes it.\n",
                INERT_TOOL_KEYS.join("/"),
                self.inert_tools.join(", ")
            ));
        }
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
        // No tool is read from the task, in either direction. The manifest
        // merges into every task, so anything hoisted here becomes an
        // invocation obligation for all of them — see the module header.
        // A task's declared tools were never hoisted (run against a real
        // corpus, the naive version lifted six MCP tool names out of the two
        // tasks that declared them and handed them to all fifteen), and the
        // leading runner of a focused-test command is no longer hoisted
        // either: it has the identical consequence by another route.
    }

    let manifest_path = project_root.join(".archon").join("project.json");
    let existing = read_manifest(&manifest_path)?;
    let created = existing.is_none();
    let mut manifest = existing.unwrap_or_default();

    let inert_tools = inert_tool_names(&manifest);
    let added_env_keys = merge_list(&mut manifest, "required_env_keys", &env_keys);
    manifest
        .entry("schema_version".to_string())
        .or_insert_with(|| Value::String(SCHEMA_VERSION.to_string()));

    let sync = CapabilitySync {
        manifest_path: manifest_path.clone(),
        added_env_keys,
        inert_tools,
        created,
        tasks_read,
    };

    if !dry_run && tasks_read > 0 && (created || !sync.added_env_keys.is_empty()) {
        if let Some(parent) = manifest_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
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
    let value: Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "{} is not valid JSON; refusing to overwrite it",
            path.display()
        )
    })?;
    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => anyhow::bail!(
            "{} is not a JSON object; refusing to overwrite it",
            path.display()
        ),
    }
}

/// Union `values` into `manifest[key]`, returning only what was newly added.
fn merge_list(
    manifest: &mut Map<String, Value>,
    key: &str,
    values: &BTreeSet<String>,
) -> Vec<String> {
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

/// Tool names sitting in the manifest under a key nothing merges any more.
///
/// Reported, never removed: a manifest only ever grows, and deleting a key an
/// operator hand-wrote is exactly the silent loss `read_manifest` refuses to
/// commit. Reading it back and naming it is what makes the key visibly inert.
fn inert_tool_names(manifest: &Map<String, Value>) -> Vec<String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for key in INERT_TOOL_KEYS {
        match manifest.get(*key) {
            Some(Value::Array(items)) => {
                names.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
            }
            Some(Value::Object(bundles)) => {
                for bundle in bundles.values() {
                    if let Value::Array(items) = bundle {
                        names.extend(items.iter().filter_map(Value::as_str).map(str::to_string));
                    }
                }
            }
            _ => {}
        }
    }
    names.into_iter().collect()
}

#[cfg(test)]
#[path = "workflow_capabilities_tests.rs"]
mod workflow_capabilities_tests;

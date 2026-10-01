//! Batch O2 (PLAN-9): every file a task's focused tests run has an owner.
//!
//! A task's `## Focused Tests` commands run test files. When no task
//! declares such a file, the run can write it only through the declaring
//! task's own widening (Issue-71), and every other path that asks "whose
//! file is this" -- a baseline failure, a residual gap, a regression the
//! host found -- answers "nobody": the failure cannot be routed, and a
//! regression in it has no owner to fix it. So the set gate requires each
//! such file to be declared by some task: in a `Files Expected to Change`,
//! a shared-append target or a deliverable contract (the owned set the PRD
//! owner check uses), a declared directory covering it included.
//!
//! The files are read from the command itself, for any runner:
//!
//! - `cargo test` / `cargo nextest run` with `--test NAME`: the target
//!   `<pkg>/tests/NAME.rs` or `<pkg>/tests/NAME/main.rs` of the package(s)
//!   the command selects (`-p`/`--package`, else every package), whether or
//!   not it exists yet (the task may be the one creating it);
//! - a cargo module filter `a::b::c`: the module file it names under the
//!   package's `src/` at the base commit, longest prefix first (`a/b/c.rs`,
//!   `a/b/c/mod.rs`, the `#[path]` sibling `a/b_tests.rs` for a `tests`
//!   segment); a crate root, an absent module or an ambiguous package names
//!   nothing, as at run time;
//! - any word that is a file or directory at the base commit (a `pytest`
//!   path, a `node --test` file, a script), with a `::name` or `:line`
//!   selector stripped.
//!
//! A violation is a BODY finding of the declaring task, routed to the set
//! gate's re-author like every other set-lint body finding.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use archon_workflow::repository_record::{RepositoryTree, read_repository_record};
use archon_workflow::task_universe::{parsing::parse_task_file, task_files_under};

use crate::command::workflow_gate::{GateFinding, GateId};

/// cargo options that take one value (`--opt v`; `--opt=v` is one word).
const VALUED: [&str; 17] = [
    "-p",
    "--package",
    "--bin",
    "--test",
    "--example",
    "--bench",
    "--exclude",
    "--features",
    "-F",
    "-j",
    "--jobs",
    "--profile",
    "--target",
    "--target-dir",
    "--manifest-path",
    "--color",
    "--test-threads",
];

/// One file (or directory) a command runs; declaring ANY alternative covers
/// it (a target file that may live in either of two shapes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Required {
    pub alternatives: Vec<String>,
    pub directory: bool,
}

/// The files `command` runs, against the base tree's `paths` and the
/// workspace's `packages` (name -> repository-relative directory, `""` for
/// the root package).
pub(super) fn required_files(
    command: &str,
    paths: &BTreeSet<String>,
    packages: &BTreeMap<String, String>,
) -> Vec<Required> {
    let words: Vec<&str> = command
        .split_whitespace()
        .map(|word| word.trim_matches(['\'', '"']))
        .collect();
    let mut out: Vec<Required> = Vec::new();
    if let Some(start) = cargo_test_start(&words) {
        out.extend(cargo_files(&words[start..], paths, packages));
    }
    let is_dir = |path: &str| {
        let under = format!("{path}/");
        paths
            .range(under.clone()..)
            .next()
            .is_some_and(|p| p.starts_with(&under))
    };
    for (at, word) in words.iter().enumerate() {
        // m4: an option's value (`--manifest-path x`, `-p x`) is no file the
        // command runs.
        if at > 0 && VALUED.contains(&words[at - 1]) {
            continue;
        }
        let Some(path) = path_word(word) else {
            continue;
        };
        if paths.contains(&path) {
            let directory = is_dir(&path);
            out.push(Required {
                alternatives: vec![path],
                directory,
            });
        }
    }
    let mut seen = BTreeSet::new();
    out.retain(|required| seen.insert(required.alternatives.clone()));
    out
}

/// The packages `command` selects (`-p`, `--package`) that no manifest at
/// the base commit declares: what it runs cannot be resolved (m4).
pub(super) fn unknown_packages(command: &str, packages: &BTreeMap<String, String>) -> Vec<String> {
    let words: Vec<&str> = command
        .split_whitespace()
        .map(|word| word.trim_matches(['\'', '"']))
        .collect();
    let Some(start) = cargo_test_start(&words) else {
        return Vec::new();
    };
    let args = &words[start..];
    let args = args
        .iter()
        .position(|w| *w == "--")
        .map_or(args, |at| &args[..at]);
    let mut unknown = Vec::new();
    for (at, word) in args.iter().enumerate() {
        let name = match word.split_once('=') {
            Some(("-p" | "--package", value)) => Some(value),
            _ if matches!(*word, "-p" | "--package") => args.get(at + 1).copied(),
            _ => None,
        };
        if let Some(name) = name.filter(|name| !packages.contains_key(*name)) {
            unknown.push(name.to_string());
        }
    }
    unknown
}

/// Where the cargo test arguments start: after `cargo test` or `cargo
/// nextest run` (an environment prefix before `cargo` is allowed).
fn cargo_test_start(words: &[&str]) -> Option<usize> {
    let at = words.iter().position(|word| *word == "cargo")?;
    match &words[at + 1..] {
        ["test", ..] => Some(at + 2),
        ["nextest", "run", ..] => Some(at + 3),
        _ => None,
    }
}

fn cargo_files(
    args: &[&str],
    paths: &BTreeSet<String>,
    packages: &BTreeMap<String, String>,
) -> Vec<Required> {
    let cargo_args = args
        .iter()
        .position(|word| *word == "--")
        .map_or(args, |at| &args[..at]);
    let mut selected: Vec<&str> = Vec::new();
    let mut targets: Vec<&str> = Vec::new();
    let mut filter: Option<&str> = None;
    let mut at = 0;
    while at < cargo_args.len() {
        let word = cargo_args[at];
        let (option, inline) = match word.split_once('=') {
            Some((option, value)) if option.starts_with('-') => (option, Some(value)),
            _ => (word, None),
        };
        if VALUED.contains(&option) {
            let value = inline.or_else(|| cargo_args.get(at + 1).copied());
            at += if inline.is_some() { 1 } else { 2 };
            match (option, value) {
                ("-p" | "--package", Some(value)) => selected.push(value),
                ("--test", Some(value)) => targets.push(value),
                _ => {}
            }
            continue;
        }
        if !word.starts_with('-') && filter.is_none() {
            filter = Some(word);
        }
        at += 1;
    }
    let dirs: Vec<&str> = if selected.is_empty() {
        packages.values().map(String::as_str).collect()
    } else {
        selected
            .iter()
            .filter_map(|name| packages.get(*name).map(String::as_str))
            .collect()
    };
    let join = |dir: &str, rest: &str| {
        if dir.is_empty() {
            rest.to_string()
        } else {
            format!("{dir}/{rest}")
        }
    };
    let mut out = Vec::new();
    for name in &targets {
        let shapes: Vec<String> = dirs
            .iter()
            .flat_map(|dir| {
                [
                    join(dir, &format!("tests/{name}.rs")),
                    join(dir, &format!("tests/{name}/main.rs")),
                ]
            })
            .collect();
        let existing: Vec<String> = shapes
            .iter()
            .filter(|shape| paths.contains(*shape))
            .cloned()
            .collect();
        let alternatives = if existing.is_empty() {
            shapes
        } else {
            existing
        };
        if !alternatives.is_empty() {
            out.push(Required {
                alternatives,
                directory: false,
            });
        }
    }
    if targets.is_empty()
        && let Some(filter) = filter
    {
        let hits: BTreeSet<String> = dirs
            .iter()
            .filter_map(|dir| module_file(&join(dir, "src"), filter, paths))
            .collect();
        if hits.len() == 1 {
            out.push(Required {
                alternatives: hits.into_iter().collect(),
                directory: false,
            });
        }
    }
    out
}

/// The module file a filter names under `src`, longest prefix first.
fn module_file(src: &str, filter: &str, paths: &BTreeSet<String>) -> Option<String> {
    let segments: Vec<&str> = filter.split("::").filter(|s| !s.is_empty()).collect();
    if segments.iter().any(|segment| {
        !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    }) {
        return None;
    }
    for len in (1..=segments.len()).rev() {
        let module = segments[..len].join("/");
        let mut shapes = vec![
            format!("{src}/{module}.rs"),
            format!("{src}/{module}/mod.rs"),
        ];
        if len >= 2 && segments[len - 1] == "tests" {
            shapes.push(format!("{src}/{}_tests.rs", segments[..len - 1].join("/")));
        }
        if let Some(hit) = shapes.into_iter().find(|shape| paths.contains(shape)) {
            // A crate root names every test; it is no test file of its own.
            let root = [format!("{src}/lib.rs"), format!("{src}/main.rs")];
            return (!root.contains(&hit)).then_some(hit);
        }
    }
    None
}

/// A word as the repository path it may name: selectors and punctuation
/// stripped; `None` for an option, a URL or anything that is no path shape.
fn path_word(word: &str) -> Option<String> {
    if word.starts_with('-') || word.contains("://") || word.contains(['*', '$', '<', '>', '|']) {
        return None;
    }
    let word = word.split("::").next().unwrap_or(word);
    let word = match word.rsplit_once(':') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => word,
    };
    let word = word.trim_end_matches([',', ';']);
    if !word.contains('/') && !word.contains('.') {
        return None;
    }
    let path = archon_workflow::repository_record::normalize_relative(word);
    (!path.is_empty() && !path.split('/').any(|segment| segment == "..")).then_some(path)
}

/// The workspace's packages at the base commit: each `Cargo.toml` with a
/// `[package]` name, by name.
fn packages(tree: &RepositoryTree) -> Result<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    for manifest in tree
        .paths_at_base()
        .iter()
        .filter(|path| *path == "Cargo.toml" || path.ends_with("/Cargo.toml"))
    {
        // m4: a manifest git cannot read fails the check, never a package
        // silently missing.
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(tree.root())
            .args(["show", &format!("{}:{manifest}", tree.base_commit())])
            .output()
            .with_context(|| format!("reading {manifest} at the base commit"))?;
        if !output.status.success() {
            anyhow::bail!(
                "reading {manifest} at base commit {}: {}",
                tree.base_commit(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(name) = package_name(&text) {
            let dir = manifest
                .strip_suffix("Cargo.toml")
                .unwrap_or_default()
                .trim_end_matches('/');
            found.insert(name, dir.to_string());
        }
    }
    Ok(found)
}

/// The `name` of a manifest's `[package]` table.
pub(super) fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "name"
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// The command(s) one declared item holds: its backticked spans, else its
/// whole text.
fn commands_of(item: &str) -> Vec<String> {
    let spans: Vec<String> = item
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|span| !span.is_empty())
        .map(str::to_string)
        .collect();
    if spans.is_empty() {
        vec![item.trim().to_string()]
    } else {
        spans
    }
}

/// Whether some owned path covers `required`.
fn covered(required: &Required, owned: &BTreeSet<String>) -> bool {
    required.alternatives.iter().any(|path| {
        owned.iter().any(|owner| {
            owner == path
                || path.starts_with(&format!("{owner}/"))
                || (required.directory && owner.starts_with(&format!("{path}/")))
        })
    })
}

/// The set gate's findings: one BODY finding per (task, uncovered file).
/// Empty for a task set without a repository record.
pub(super) fn set_findings(root: &Path) -> Result<Vec<GateFinding>> {
    let Some(record) = read_repository_record(root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    let owned: BTreeSet<String> = super::owner_coverage::set_owned_paths(root)
        .iter()
        .flat_map(|entry| super::unowned_obligations::entry_paths(entry))
        .filter_map(|path| tree.relative_to_root(&path))
        .filter(|path| !path.is_empty())
        .collect();
    let mut packages_cache: Option<BTreeMap<String, String>> = None;
    let mut findings = Vec::new();
    // m4: fails closed -- a task directory, task file or manifest it cannot
    // read is an error; a task it cannot parse is a finding of its own.
    let files = task_files_under(root)
        .map_err(|error| anyhow::anyhow!("listing task files under {}: {error}", root.display()))?;
    for path in files {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading task file {}", path.display()))?;
        let task = match parse_task_file(&path, &raw) {
            Ok(task) => task,
            Err(error) => {
                findings.push(GateFinding::new(
                    GateId::WorkflowLintTaskSet,
                    format!(
                        "task file {} cannot be parsed ({error}), so the files its focused tests run are unknown and none of them can be shown to have an owner",
                        path.display()
                    ),
                    &path.display().to_string(),
                    Some(path.clone()),
                    archon_workflow::RemediationScope::Body,
                ));
                continue;
            }
        };
        // A declared item is its backticked command(s), or its whole text.
        for command in task.focused_tests.iter().flat_map(|item| commands_of(item)) {
            let command = &command;
            let words: Vec<&str> = command.split_whitespace().collect();
            let packages = if cargo_test_start(&words).is_some() {
                if packages_cache.is_none() {
                    packages_cache = Some(packages(&tree)?);
                }
                packages_cache.clone().unwrap_or_default()
            } else {
                BTreeMap::new()
            };
            for name in unknown_packages(command, &packages) {
                findings.push(GateFinding::new(
                    GateId::WorkflowLintTaskSet,
                    format!(
                        "task {}: focused test `{}` selects package `{name}`, which no manifest at the base commit declares, so the files it runs cannot be resolved; name a package the repository holds",
                        task.canonical_task_id,
                        command.trim()
                    ),
                    &task.canonical_task_id,
                    Some(path.clone()),
                    archon_workflow::RemediationScope::Body,
                ));
            }
            for required in required_files(command, tree.paths_at_base(), &packages) {
                if covered(&required, &owned) {
                    continue;
                }
                let shown = required.alternatives.join("` or `");
                findings.push(GateFinding::new(
                    GateId::WorkflowLintTaskSet,
                    format!(
                        "task {}: focused test `{}` runs `{shown}`, which no task declares, so no task owns it and a failure or regression in it can be routed to nobody; declare it in the Files Expected to Change of the task that owns that test (this one, when it writes it), or run a test file a task declares",
                        task.canonical_task_id,
                        command.trim()
                    ),
                    &task.canonical_task_id,
                    Some(path.clone()),
                    archon_workflow::RemediationScope::Body,
                ));
            }
        }
    }
    Ok(findings)
}

#[cfg(test)]
#[path = "focused_test_files_tests.rs"]
mod tests;

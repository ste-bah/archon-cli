//! Which file a failing test lives in, and whose task that file is.
//!
//! # From test id to file
//!
//! A libtest id is a module path: `v2::write::grant::tests::widens` is the
//! test `widens` in module `v2::write::grant::tests`. The module tree is
//! walked back to a file under the package's `src/`, longest prefix first,
//! by the three shapes Rust modules take: `<path>.rs`, `<path>/mod.rs`, and
//! — for a `tests` module declared with `#[path = "<mod>_tests.rs"]`, which
//! is how this workspace holds its test sources beside the code — the
//! sibling `<parent>_tests.rs`. An inline `#[cfg(test)] mod tests {}` falls
//! through to the file that declares it, which is the right owner: the test
//! IS that file's. A path that reaches no file (an integration test binary,
//! a generated module, a crate the command does not name) is unresolvable,
//! and an unresolvable failure is the current task's to answer for.
//!
//! The package is read from the command (`-p`, `--package`) and located by
//! its manifest's `name` line under the workspace root's immediate package
//! directories; a command naming no package is resolved against every
//! package, and counts as resolved only when exactly one holds the file.
//!
//! # From file to task
//!
//! Ownership is the task universe's `files_expected_to_change`, read by the
//! same `declared_writes` the author-wave planner uses, so "whose file" is
//! answered once. A branch's own declared targets count as its own too. A
//! file two tasks declare is the CURRENT task's: it owns it as much as the
//! other, and routing away an obligation it can meet leaves it with nobody.

use std::path::Path;

use crate::task_universe::WorkflowV2TaskUniverse;

/// Who answers for a baseline failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ownership {
    /// The current task's: it declares the file.
    Current,
    /// Another task in the universe declares the file; routed to it.
    Other(String),
    /// No task declares the file: the current task's by default, and the
    /// first branch in a wave to meet it keeps it.
    Unowned,
}

/// The repo-relative file `test_id` lives in, when the module tree reaches
/// one under a package `command` names (or under exactly one package, when
/// it names none).
pub(crate) fn test_file(repo_root: &Path, command: &str, test_id: &str) -> Option<String> {
    let segments: Vec<&str> = test_id.split("::").collect();
    let (_, modules) = segments.split_last()?;
    let roots = package_source_roots(repo_root, command);
    let mut found: Vec<String> = roots
        .iter()
        .filter_map(|src| resolve_under(repo_root, src, modules))
        .collect();
    found.dedup();
    match found.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// The `src` directories (repo-relative) the command's package(s) hold.
pub(super) fn package_source_roots(repo_root: &Path, command: &str) -> Vec<String> {
    package_dirs_for(repo_root, command)
        .into_iter()
        .map(|dir| {
            if dir.is_empty() {
                "src".to_string()
            } else {
                format!("{dir}/src")
            }
        })
        .filter(|src| repo_root.join(src).is_dir())
        .collect()
}

/// The directories (repo-relative; empty for the workspace root) of the
/// package the command names, or of every package when it names none.
pub(super) fn package_dirs_for(repo_root: &Path, command: &str) -> Vec<String> {
    let wanted = super::test_baseline_parse::cargo_package(command);
    let mut dirs = Vec::new();
    for dir in package_dirs(repo_root) {
        let manifest = repo_root.join(&dir).join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let name = manifest_package_name(&text);
        let matches = match (&wanted, name.as_deref()) {
            (Some(wanted), Some(name)) => wanted == name,
            (Some(wanted), None) => dir.rsplit('/').next() == Some(wanted.as_str()),
            (None, _) => true,
        };
        if matches {
            dirs.push(dir);
        }
    }
    dirs
}

/// The workspace root itself, then every directory one and two levels down
/// that holds a `Cargo.toml` — `crates/<name>/`, `<name>/` — sorted.
fn package_dirs(repo_root: &Path) -> Vec<String> {
    let mut dirs = vec![String::new()];
    let Ok(top) = std::fs::read_dir(repo_root) else {
        return dirs;
    };
    for entry in top.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.join("Cargo.toml").is_file() {
            dirs.push(name.to_string());
        }
        let Ok(nested) = std::fs::read_dir(&path) else {
            continue;
        };
        for child in nested.flatten() {
            let child_path = child.path();
            if child_path.is_dir()
                && child_path.join("Cargo.toml").is_file()
                && let Some(child_name) = child_path.file_name().and_then(|n| n.to_str())
            {
                dirs.push(format!("{name}/{child_name}"));
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// The `name = "..."` of a manifest's `[package]` table, read by line so a
/// workspace-only manifest (no package) answers `None`.
fn manifest_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// The file the module path `modules` reaches under `src`, longest prefix
/// first; the crate root when the path names no module but `tests`.
pub(super) fn resolve_under(repo_root: &Path, src: &str, modules: &[&str]) -> Option<String> {
    for len in (1..=modules.len()).rev() {
        let prefix = &modules[..len];
        let joined = prefix.join("/");
        let mut candidates = vec![
            format!("{src}/{joined}.rs"),
            format!("{src}/{joined}/mod.rs"),
        ];
        if prefix[len - 1] == "tests" && len >= 2 {
            candidates.push(format!("{src}/{}_tests.rs", prefix[..len - 1].join("/")));
        }
        if let Some(hit) = candidates.into_iter().find(|c| repo_root.join(c).is_file()) {
            return Some(hit);
        }
    }
    // Only a test AT the crate root (`smoke`, `tests::smoke`) falls back to
    // the root file; a module path that reaches nothing is unresolved here,
    // or every package would claim every unknown test through its `lib.rs`.
    if !modules.iter().all(|module| *module == "tests") {
        return None;
    }
    ["lib.rs", "main.rs"]
        .iter()
        .map(|root| format!("{src}/{root}"))
        .find(|c| repo_root.join(c).is_file())
}

/// Whether `task_id`'s declared writes cover `file` — a declared file, or a
/// declared directory above it.
pub(crate) fn task_owns(universe: &WorkflowV2TaskUniverse, task_id: &str, file: &str) -> bool {
    crate::v2::script::declared_writes(universe, task_id)
        .iter()
        .any(|declared| covers(declared, file))
}

fn covers(declared: &str, file: &str) -> bool {
    let declared = declared.trim_end_matches('/');
    declared == file
        || file
            .strip_prefix(declared)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Who answers for a failure in `file`: the current branch when its tasks
/// or its declared targets cover the file, else the first other task in
/// universe order that declares it, else nobody.
pub(crate) fn ownership(
    universe: Option<&WorkflowV2TaskUniverse>,
    own_task_ids: &[String],
    own_targets: &[String],
    file: &str,
) -> Ownership {
    if own_targets.iter().any(|target| covers(target, file)) {
        return Ownership::Current;
    }
    let Some(universe) = universe else {
        return Ownership::Unowned;
    };
    if own_task_ids
        .iter()
        .any(|task| task_owns(universe, task, file))
    {
        return Ownership::Current;
    }
    universe
        .tasks
        .iter()
        .filter(|task| !own_task_ids.contains(&task.canonical_task_id))
        .find(|task| task_owns(universe, &task.canonical_task_id, file))
        .map(|task| Ownership::Other(task.canonical_task_id.clone()))
        .unwrap_or(Ownership::Unowned)
}

#[cfg(test)]
#[path = "test_baseline_owner_tests.rs"]
mod tests;

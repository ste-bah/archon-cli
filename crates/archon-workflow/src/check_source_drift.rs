//! Where a tree and a check's pinned sources disagree (PLAN-11): in a write
//! branch's worktree before it lands ([`landing_changes`]), and in the tree
//! an acceptance round runs against ([`tree_drift`]).
//!
//! A change is judged when it can weaken what a check asserts: a pinned
//! source whose bytes differ (edited, created where it was absent, deleted),
//! a source the check now resolves to that is SPECIFIC to it (a test target,
//! a module file one loads, a script, a pytest file or conftest), or one that
//! satisfies a watch (the test a check names, defined for the first time). A
//! newly added test that merely joins a suite or a substring filter only
//! adds assertions, and is not held.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::check_source_pins::{CheckPins, CheckSourcePins, current_bytes};
use crate::check_source_resolve::{Found, Roots, SourceRoot, Watch, resolve};
use crate::check_source_rust::mod_children;
use crate::task_set_contract::content_digest;

/// One source whose tree bytes differ from its pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChange {
    pub check_ids: BTreeSet<String>,
    pub root: SourceRoot,
    pub path: String,
    pub item: Option<String>,
    /// Whether the check pinned this source (a `None` digest then means
    /// pinned absent); an unpinned one is new to the check.
    pub was_pinned: bool,
    pub pinned: Option<String>,
    pub actual: Option<String>,
}

impl SourceChange {
    pub fn key(&self) -> (SourceRoot, String, Option<String>) {
        (self.root, self.path.clone(), self.item.clone())
    }

    pub fn label(&self) -> String {
        match &self.item {
            Some(item) => format!("{} ({item})", self.path),
            None => self.path.clone(),
        }
    }
}

/// Roles a newly resolved source can have that make it the check's own:
/// anything else only widens a suite.
const SPECIFIC_ROLES: [&str; 11] = [
    "integration test target",
    "executed script",
    "pytest file",
    "pytest conftest",
    "script",
    "sourced script",
    crate::check_source_follow::ROLE_INCLUDED,
    crate::check_source_follow::ROLE_IMPORTED,
    crate::check_source_follow::ROLE_FIXTURE,
    "build script",
    "cargo configuration",
];

fn specific(found: &Found) -> bool {
    SPECIFIC_ROLES
        .iter()
        .any(|role| found.role == *role || found.role.ends_with(" script"))
}

fn under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

/// Whether `found` is the source `watch` waits for.
pub(crate) fn satisfies(watch: &Watch, found: &Found) -> bool {
    watch.root == found.root
        && under(&found.path, &watch.dir)
        && !watch.excluded.iter().any(|other| under(&found.path, other))
        && match (&watch.test_name, &found.item) {
            (None, _) => true,
            (Some(name), Some(key)) => {
                let actual = crate::check_source_rust::key_name(key);
                if watch.exact {
                    actual == name
                } else {
                    actual.contains(name.as_str())
                }
            }
            (Some(_), None) => found.role == "file defining the filtered test",
        }
}

fn pinned_in(pins: &CheckPins, root: SourceRoot, path: &str, item: Option<&str>) -> bool {
    pins.sources
        .iter()
        .any(|source| source.same_source(root, path, item))
}

fn merge(changes: Vec<(String, SourceChange)>) -> Vec<SourceChange> {
    let mut by_key: BTreeMap<(SourceRoot, String, Option<String>), SourceChange> = BTreeMap::new();
    for (id, change) in changes {
        by_key
            .entry(change.key())
            .and_modify(|existing| {
                existing.check_ids.insert(id.clone());
                existing.was_pinned |= change.was_pinned;
            })
            .or_insert_with(|| SourceChange {
                check_ids: BTreeSet::from([id]),
                ..change
            });
    }
    by_key.into_values().collect()
}

fn digest_of(roots: &Roots, root: SourceRoot, path: &str, item: Option<&str>) -> Option<String> {
    current_bytes(roots, root, path, item).map(|bytes| content_digest(&bytes))
}

/// At acceptance: every pinned source whose bytes differ from its pin, and
/// every source the check resolves to NOW that it did not pin and that is
/// specific to it or satisfies one of its watches.
pub fn tree_drift(pins: &CheckSourcePins, roots: &Roots) -> Vec<SourceChange> {
    let mut changes = Vec::new();
    for (id, check) in &pins.checks {
        for source in &check.sources {
            let actual = digest_of(roots, source.root, &source.path, source.item.as_deref());
            if actual != source.digest {
                changes.push((
                    id.clone(),
                    SourceChange {
                        check_ids: BTreeSet::new(),
                        root: source.root,
                        path: source.path.clone(),
                        item: source.item.clone(),
                        was_pinned: true,
                        pinned: source.digest.clone(),
                        actual,
                    },
                ));
            }
        }
        let now = resolve(&check.command, roots.of(check.cwd), roots);
        for found in now.found {
            if !is_new_source(check, &found) {
                continue;
            }
            let actual = digest_of(roots, found.root, &found.path, found.item.as_deref());
            if actual.is_none() {
                continue;
            }
            changes.push((id.clone(), new_source(&found, actual)));
        }
    }
    merge(changes)
}

fn new_source(found: &Found, actual: Option<String>) -> SourceChange {
    SourceChange {
        check_ids: BTreeSet::new(),
        root: found.root,
        path: found.path.clone(),
        item: found.item.clone(),
        was_pinned: false,
        pinned: None,
        actual,
    }
}

/// The rule both a landing and an acceptance round judge a newly resolved
/// source by: not pinned, and either specific to the check or the source
/// one of its watches waits for.
fn is_new_source(check: &CheckPins, found: &Found) -> bool {
    !pinned_in(check, found.root, &found.path, found.item.as_deref())
        && (specific(found) || check.watches.iter().any(|w| satisfies(w, found)))
}

/// A write branch's worktree as the landing sees it: the repository it
/// changed, the project's own root, and which project paths are acceptance
/// inputs -- seeded into the worktree at the same relative paths, and
/// landed back into the project with the branch.
pub struct LandingView<'a> {
    pub worktree: &'a Path,
    pub project: &'a Path,
    pub project_input: &'a dyn Fn(&str) -> bool,
}

impl LandingView<'_> {
    /// Where the branch's copy of a source is, if the branch can change it.
    fn copy(&self, root: SourceRoot, path: &str) -> Option<std::path::PathBuf> {
        match root {
            SourceRoot::Repository => Some(self.worktree.join(path)),
            SourceRoot::Project if (self.project_input)(path) => Some(self.worktree.join(path)),
            SourceRoot::Project => None,
        }
    }
}

fn digest_at(path: &Path, item: Option<&str>) -> Option<String> {
    crate::check_source_pins::bytes_at(path, item).map(|bytes| content_digest(&bytes))
}

/// In a write branch's worktree, by the same rule as acceptance: every
/// pinned source the branch can change -- a repository file among `changed`
/// (repository-relative paths the worktree changed), or a project-input
/// copy -- whose bytes differ from its pin, and every source a check now
/// resolves to in the worktree that is new by [`is_new_source`] and that
/// the branch created or changed, plus every module file a held Rust source
/// loads that the worktree also changed (part of the same proposal).
pub fn landing_changes(
    pins: &CheckSourcePins,
    changed: &[String],
    view: &LandingView<'_>,
) -> Vec<SourceChange> {
    let changed: BTreeSet<&str> = changed.iter().map(String::as_str).collect();
    let touched = |root: SourceRoot, path: &str| match root {
        SourceRoot::Repository => changed.contains(path),
        SourceRoot::Project => view.copy(root, path).is_some_and(|copy| {
            std::fs::read(&copy).ok() != std::fs::read(view.project.join(path)).ok()
        }),
    };
    let worktree_roots = Roots {
        repository: view.worktree,
        project: view.worktree,
    };
    let mut changes = Vec::new();
    for (id, check) in &pins.checks {
        for source in &check.sources {
            let Some(copy) = view.copy(source.root, &source.path) else {
                continue;
            };
            if !touched(source.root, &source.path) {
                continue;
            }
            let actual = digest_at(&copy, source.item.as_deref());
            if actual != source.digest {
                changes.push((
                    id.clone(),
                    SourceChange {
                        check_ids: BTreeSet::new(),
                        root: source.root,
                        path: source.path.clone(),
                        item: source.item.clone(),
                        was_pinned: true,
                        pinned: source.digest.clone(),
                        actual,
                    },
                ));
            }
        }
        for mut found in resolve(&check.command, view.worktree, &worktree_roots).found {
            if (view.project_input)(&found.path) {
                found.root = SourceRoot::Project;
            }
            if !is_new_source(check, &found) || !touched(found.root, &found.path) {
                continue;
            }
            let actual = digest_at(&view.worktree.join(&found.path), found.item.as_deref());
            if actual.is_some() {
                changes.push((id.clone(), new_source(&found, actual)));
            }
        }
    }
    with_new_modules(merge(changes), &changed, view.worktree)
}

/// `merged` and every module file a held Rust file loads that the worktree
/// also changed, owned by the same checks.
fn with_new_modules(
    merged: Vec<SourceChange>,
    changed: &BTreeSet<&str>,
    worktree: &Path,
) -> Vec<SourceChange> {
    let repo = SourceRoot::Repository;
    let mut extra: Vec<SourceChange> = Vec::new();
    let mut queue: Vec<String> = merged
        .iter()
        .filter(|c| c.root == repo && c.item.is_none())
        .map(|c| c.path.clone())
        .collect();
    let mut seen: BTreeSet<String> = merged.iter().map(|c| c.path.clone()).collect();
    while let Some(file) = queue.pop() {
        if !file.ends_with(".rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(worktree.join(&file)) else {
            continue;
        };
        let owners = merged
            .iter()
            .chain(&extra)
            .find(|change| change.path == file)
            .map(|change| change.check_ids.clone())
            .unwrap_or_default();
        let crate_root = file.split('/').rev().nth(1) == Some("tests");
        for child in mod_children(worktree, &file, &text, crate_root) {
            if child.escapes {
                continue;
            }
            let child = child.path;
            if changed.contains(child.as_str()) && seen.insert(child.clone()) {
                queue.push(child.clone());
                extra.push(SourceChange {
                    check_ids: owners.clone(),
                    root: repo,
                    path: child.clone(),
                    item: None,
                    was_pinned: false,
                    pinned: None,
                    actual: digest_at(&worktree.join(&child), None),
                });
            }
        }
    }
    let mut all = merged;
    all.extend(extra);
    all
}

#[cfg(test)]
#[path = "check_source_drift_tests.rs"]
mod tests;

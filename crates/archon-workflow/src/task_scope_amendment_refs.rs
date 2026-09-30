//! Batch O: which tasks' code references a source file no task declares --
//! the code-dependency tier of the ownerless-file assignment.
//!
//! A file no task declares is still some task's to keep working when that
//! task's code uses it. The host reads that from the code itself, never
//! from agent text. A source file references
//!
//! - its child modules: `mod name;` (`name.rs` or `name/mod.rs` beside the
//!   module's directory) and `#[path = "..."] mod name;` (relative to the
//!   declaring file's directory);
//! - the files that define an item it names -- after `::` in any path
//!   (`crate::`, `super::`, `self::`, a crate name; `use` lists included) or
//!   as a call (`name(`) -- when exactly ONE indexed file defines it. A
//!   `mod name;` declaration defines `name` as its child file, so a path
//!   through a module reaches that module's file. A name defined in several
//!   files references none of them. Items a test file defines count only
//!   for references from test files.
//!
//! Ownership: a file a task's declared code references directly is that
//! task's (ties shared). Then, to a fixpoint, a file EVERY user of which is
//! owned belongs to its nearest users' owners (a declared user before one
//! owned at depth 1, and so on) -- a private dependency of task code,
//! a child module of an owned file -- while a file some unowned code also
//! uses is shared infrastructure and stays unowned. An integration test
//! file or a test module belongs to the owners of the nearest code its
//! paths name (a declared file before code a task merely uses), whoever
//! holds it as a module. A crate root or a module file whose children include a
//! declared file is a hub: its references are no usage; it is owned only
//! when all its declared children are one task's. Only the crates holding a
//! declared source file are indexed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// What the code says about the files no task declares.
#[derive(Debug, Default)]
pub struct CodeUsage {
    /// Each file some task's code owns -> those tasks.
    pub owners: BTreeMap<String, BTreeSet<String>>,
    /// Each indexed file -> the files that use it (hubs' references aside).
    pub users: BTreeMap<String, BTreeSet<String>>,
    /// Each owned file -> how many references from a declared file it was
    /// reached at (1: used by declared code directly).
    pub depth: BTreeMap<String, usize>,
}

/// The owners of those of `files` nearest a task's declared code (a
/// declared file is nearest; then by reference depth); ties shared.
fn nearest<'a>(
    owners: &BTreeMap<String, BTreeSet<String>>,
    depth: &BTreeMap<String, usize>,
    declared: &BTreeSet<String>,
    files: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    let distance = |file: &str| {
        if declared.contains(file) {
            Some(0)
        } else {
            depth.get(file).copied()
        }
    };
    let owned: Vec<(&str, usize)> = files
        .filter(|file| owners.contains_key(*file))
        .filter_map(|file| distance(file).map(|d| (file, d)))
        .collect();
    let Some(min) = owned.iter().map(|(_, d)| *d).min() else {
        return BTreeSet::new();
    };
    owned
        .iter()
        .filter(|(_, d)| *d == min)
        .flat_map(|(file, _)| owners[*file].iter().cloned())
        .collect()
}

/// Repository-relative source file no task declares -> the tasks whose
/// code references it (see the module doc).
pub fn referencing_tasks(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
) -> BTreeMap<String, BTreeSet<String>> {
    code_usage(universe, root).owners
}

/// [`referencing_tasks`], with every indexed file's users.
pub fn code_usage(universe: &WorkflowV2TaskUniverse, root: &Path) -> CodeUsage {
    let declared = declared_sources(universe, root);
    let declared_files: BTreeSet<String> = declared.values().flatten().cloned().collect();
    let crates: BTreeSet<PathBuf> = declared_files
        .iter()
        .filter_map(|file| crate_dir(root, file))
        .collect();
    let (mut sources, mut tests) = (Vec::new(), Vec::new());
    for dir in &crates {
        collect(root, &dir.join("src"), &mut sources);
        collect(root, &dir.join("tests"), &mut tests);
    }
    let index = Index::build(
        root,
        crate_names(root, &crates),
        sources.iter().chain(&tests),
    );
    let hubs = hubs(root, &sources, &declared_files, &index);
    // Every file's references, once; a hub's references are no usage.
    let refs: BTreeMap<&String, BTreeSet<String>> = sources
        .iter()
        .chain(&tests)
        .map(|file| (file, index.references(root, file)))
        .collect();
    let mut referrers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (file, targets) in &refs {
        if hubs.contains(*file) {
            continue;
        }
        for target in targets {
            referrers
                .entry(target.as_str())
                .or_default()
                .insert(file.as_str());
        }
    }
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (task, files) in &declared {
        for file in files {
            owners.entry(file.clone()).or_default().insert(task.clone());
        }
    }
    let mut assigned: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    // Direct use by a task's declared code: that task's (ties shared).
    for file in &declared_files {
        let by = owners.get(file).cloned().unwrap_or_default();
        for target in refs.get(file).into_iter().flatten() {
            if !declared_files.contains(target) {
                assigned
                    .entry(target.clone())
                    .or_default()
                    .extend(by.iter().cloned());
            }
        }
    }
    for (file, by) in &assigned {
        owners.insert(file.clone(), by.clone());
        depth.insert(file.clone(), 1);
    }
    // A hub whose declared children are all ONE task's is that task's (its
    // references still never spread ownership); one several tasks' children
    // share stays unowned.
    for hub in &hubs {
        if owners.contains_key(hub) {
            continue;
        }
        let by: BTreeSet<String> = index
            .children(root, hub)
            .iter()
            .filter(|child| declared_files.contains(*child))
            .filter_map(|child| owners.get(child))
            .flatten()
            .cloned()
            .collect();
        if by.len() == 1 {
            assigned.insert(hub.clone(), by.clone());
            owners.insert(hub.clone(), by);
            depth.insert(hub.clone(), 1);
        }
    }
    // To a fixpoint: a file every one of whose users is owned is theirs; an
    // integration test of owned code is its code's owners'. A file some
    // unowned code also uses is shared infrastructure and stays unowned.
    let mut round = 1;
    loop {
        round += 1;
        let mut grew: Vec<(String, BTreeSet<String>)> = Vec::new();
        for file in sources.iter().chain(&tests) {
            if owners.contains_key(file) || hubs.contains(file) {
                continue;
            }
            // A test belongs to the owners of the code it tests, read from
            // the paths it names, whoever holds it as a module -- the
            // nearest of that code only (a declared file before code a
            // task merely uses).
            let tested: BTreeSet<String> = if is_test_file(file) {
                nearest(
                    &owners,
                    &depth,
                    &declared_files,
                    refs[file].iter().map(String::as_str),
                )
            } else {
                BTreeSet::new()
            };
            // Any file every user of which is owned is its nearest users'
            // owners'.
            let used: Option<BTreeSet<String>> = referrers
                .get(file.as_str())
                .filter(|users| users.iter().all(|user| owners.contains_key(*user)))
                .map(|users| nearest(&owners, &depth, &declared_files, users.iter().copied()));
            let by = if tested.is_empty() {
                used
            } else {
                Some(tested)
            };
            if let Some(by) = by.filter(|by| !by.is_empty()) {
                grew.push((file.clone(), by));
            }
        }
        if grew.is_empty() {
            break;
        }
        for (file, by) in grew {
            assigned.insert(file.clone(), by.clone());
            depth.insert(file.clone(), round);
            owners.insert(file, by);
        }
    }
    CodeUsage {
        owners: assigned,
        depth,
        users: referrers
            .into_iter()
            .map(|(file, users)| {
                (
                    file.to_string(),
                    users.into_iter().map(str::to_string).collect(),
                )
            })
            .collect(),
    }
}

/// Each task's declared Rust source files that exist, repository-relative.
fn declared_sources(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
) -> BTreeMap<String, BTreeSet<String>> {
    universe
        .tasks
        .iter()
        .map(|task| {
            let files = task
                .files_expected_to_change
                .iter()
                .chain(&task.shared_append_target_files)
                .filter_map(|entry| crate::v2::script::declared_path(entry))
                .filter_map(|raw| match declared_path_form(&raw, root) {
                    DeclaredPathForm::Repo(path) => Some(path),
                    _ => None,
                })
                .filter(|path| path.ends_with(".rs") && root.join(path).is_file())
                .collect();
            (task.canonical_task_id.clone(), files)
        })
        .collect()
}

/// The crate holding `file`: the nearest ancestor with a `Cargo.toml`.
fn crate_dir(root: &Path, file: &str) -> Option<PathBuf> {
    let mut dir = Path::new(file).parent();
    while let Some(current) = dir {
        if root.join(current).join("Cargo.toml").is_file() {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    root.join("Cargo.toml").is_file().then(PathBuf::new)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let rel = dir.join(entry.file_name());
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect(root, &rel, out),
            Ok(kind) if kind.is_file() && rel.extension().is_some_and(|ext| ext == "rs") => {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
            _ => {}
        }
    }
}

/// A test source: a `*_tests.rs` or `tests.rs` file, or any file below a
/// `tests` or `*_tests` directory.
fn is_test_file(path: &str) -> bool {
    let mut parts = path.split('/').rev();
    let name = parts.next().unwrap_or(path);
    name.ends_with("_tests.rs")
        || name == "tests.rs"
        || parts.any(|dir| dir == "tests" || dir.ends_with("_tests"))
}

/// Crate roots, and module files whose children include a declared file.
fn hubs(
    root: &Path,
    sources: &[String],
    declared: &BTreeSet<String>,
    index: &Index,
) -> BTreeSet<String> {
    sources
        .iter()
        .filter(|file| {
            let name = file.rsplit('/').next().unwrap_or(file);
            matches!(name, "lib.rs" | "main.rs")
                || index
                    .children(root, file)
                    .iter()
                    .any(|child| declared.contains(child))
        })
        .cloned()
        .collect()
}

struct Index {
    /// Item name -> (defining file, whether that file is a test file).
    defs: BTreeMap<String, BTreeSet<(String, bool)>>,
    /// Crate name as code spells it -> that crate's `src` directory.
    crates: BTreeMap<String, PathBuf>,
    module: Regex,
    named: Regex,
    paths: Regex,
    path_attr: Regex,
}

impl Index {
    fn build<'a>(
        root: &Path,
        crates: BTreeMap<String, PathBuf>,
        files: impl Iterator<Item = &'a String>,
    ) -> Self {
        let item = Regex::new(
            r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|const\s+|unsafe\s+)*(?:fn|struct|enum|trait|type|const|static)\s+([A-Za-z_][A-Za-z0-9_]*)|macro_rules!\s*([A-Za-z_][A-Za-z0-9_]*)|(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{",
        )
        .expect("item pattern");
        let mut index = Self {
            defs: BTreeMap::new(),
            module: Regex::new(
                r#"(?m)^\s*((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"#,
            )
            .expect("mod pattern"),
            named: Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("call pattern"),
            paths: Regex::new(
                r"\b(crate|super|self|[a-z_][a-z0-9_]*)((?:\s*::\s*(?:[A-Za-z_][A-Za-z0-9_]*|\{[^}]*\}))+)",
            )
            .expect("path pattern"),
            path_attr: Regex::new(r#"#\[path\s*=\s*"([^"]+)"\]"#).expect("path attribute"),
            crates,
        };
        for file in files {
            let Ok(text) = std::fs::read_to_string(root.join(file)) else {
                continue;
            };
            let test = is_test_file(file);
            let mut found: Vec<(String, String, bool)> = item
                .captures_iter(&text)
                .filter_map(|caps| caps.get(1).or_else(|| caps.get(2)).or_else(|| caps.get(3)))
                .map(|name| (name.as_str().to_string(), file.clone(), test))
                .collect();
            // `mod name;` defines `name` as its child file.
            for (name, child) in index.child_modules(root, file, &text) {
                let child_test = is_test_file(&child);
                found.push((name, child, child_test));
            }
            for (name, by, test) in found {
                index.defs.entry(name).or_default().insert((by, test));
            }
        }
        index
    }

    /// `(module name, child file)` for every file-backed module `file`
    /// declares.
    fn child_modules(&self, root: &Path, file: &str, text: &str) -> Vec<(String, String)> {
        let path = Path::new(file);
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let module_dir = module_dir(file);
        let mut out = Vec::new();
        for caps in self.module.captures_iter(text) {
            let name = caps[2].to_string();
            // `#[path = "..."]` among the module's attributes, in any order.
            let explicit = caps
                .get(1)
                .and_then(|attrs| self.path_attr.captures(attrs.as_str()))
                .map(|found| found[1].to_string());
            let candidates = match explicit {
                Some(explicit) => vec![parent.join(explicit)],
                None => vec![
                    module_dir.join(format!("{name}.rs")),
                    module_dir.join(&name).join("mod.rs"),
                ],
            };
            for candidate in candidates {
                let rel = normalize(&candidate);
                if root.join(&rel).is_file() {
                    out.push((name.clone(), rel));
                }
            }
        }
        out
    }

    fn children(&self, root: &Path, file: &str) -> Vec<String> {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        self.child_modules(root, file, &text)
            .into_iter()
            .map(|(_, child)| child)
            .collect()
    }

    /// The files `file` references (see the module doc).
    fn references(&self, root: &Path, file: &str) -> BTreeSet<String> {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            return BTreeSet::new();
        };
        let from_test = is_test_file(file);
        let mut out: BTreeSet<String> = self
            .child_modules(root, file, &text)
            .into_iter()
            .map(|(_, child)| child)
            .collect();
        out.extend(self.resolved_paths(root, file, &text));
        // A test names items by their paths: a bare name it calls says
        // nothing about which file it means.
        if from_test {
            out.remove(file);
            return out;
        }
        // The names a file uses: where each path ENDS (a module it walks
        // through is a namespace), every item of a `use` list, and every
        // bare call (`name(`, not a method or a path segment).
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for caps in self.paths.captures_iter(&text) {
            let tail = caps.get(2).map_or("", |m| m.as_str());
            match tail.rsplit("::").next().map(str::trim) {
                Some(list) if list.starts_with('{') => names.extend(
                    list.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .filter(|word| !word.is_empty()),
                ),
                Some(last) if !last.is_empty() => {
                    names.insert(last);
                }
                _ => {}
            }
        }
        // A bare call names an item in scope: its module's neighbourhood --
        // the file's own directory or the one above it -- never a file
        // elsewhere that happens to define the same name.
        let mut calls: BTreeSet<&str> = BTreeSet::new();
        for caps in self.named.captures_iter(&text) {
            let Some(call) = caps.get(1) else {
                continue;
            };
            let before = text[..call.start()].trim_end();
            if !(before.ends_with('.') || before.ends_with("::")) {
                calls.insert(call.as_str());
            }
        }
        let here = Path::new(file).parent().unwrap_or_else(|| Path::new(""));
        let near = |by: &str| {
            let dir = Path::new(by).parent().unwrap_or_else(|| Path::new(""));
            dir == here || Some(dir) == here.parent() || dir.parent() == Some(here)
        };
        for name in names.iter().copied().chain(calls.iter().copied()) {
            let Some(defining) = self.defs.get(name) else {
                continue;
            };
            let bare = calls.contains(name) && !names.contains(name);
            let usable: Vec<&String> = defining
                .iter()
                .filter(|(_, test)| from_test || !test)
                .map(|(by, _)| by)
                .filter(|by| !bare || near(by))
                .collect();
            if let [only] = usable.as_slice()
                && only.as_str() != file
            {
                out.insert((*only).clone());
            }
        }
        out.remove(file);
        out
    }
}

#[path = "task_scope_amendment_refs_paths.rs"]
mod paths;
use paths::{crate_names, module_dir, normalize};

#[cfg(test)]
#[path = "task_scope_amendment_refs_tests.rs"]
mod tests;

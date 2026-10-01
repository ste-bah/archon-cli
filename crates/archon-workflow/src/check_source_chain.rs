//! A Rust source's place in its crate, pinned with it (PLAN-11,
//! [`crate::check_source_rust`]).
//!
//! A pinned test runs only while every module on the way to it is compiled.
//! So beside a pinned test function the resolver pins the inline modules
//! around it, and for its file every `mod x;` declaration (with its
//! attributes) from the crate root down to it, plus the inner `#![cfg]`
//! attributes of each file on that path: switching a module off with a
//! `#[cfg]`, pointing its `#[path]` elsewhere, or dropping its `mod` line is
//! a pinned-source change like editing the test.
//!
//! Paths are relative to the tree (repository or project) the package sits
//! in, so a `#[path]` reaching a sibling crate resolves; one reaching out of
//! the tree is refused and recorded as unresolved, as is a module chain that
//! cycles back on itself through `#[path]`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::check_source_resolve::{Found, Resolution, Roots, files_under};
use crate::check_source_rust::{
    CFG_KEY, enclosing_mods, keyed_tests, matching_tests, mod_children,
};

pub(crate) const ROLE_DECLARATION: &str = "module declaration";
pub(crate) const ROLE_CFG: &str = "module cfg";
pub(crate) const ROLE_HELPER: &str = "test helper";

/// The tree a package sits in: its root directory, for joining tree-relative
/// paths back to files.
struct Tree<'a> {
    roots: &'a Roots<'a>,
    base: PathBuf,
}

impl<'a> Tree<'a> {
    fn of(roots: &'a Roots<'a>, package: &Path) -> Self {
        let base = roots.relative(package).map_or_else(
            || package.to_path_buf(),
            |(root, _)| roots.of(root).to_path_buf(),
        );
        Self { roots, base }
    }

    fn rel(&self, abs: &Path) -> Option<String> {
        self.roots.relative(abs).map(|(_, path)| path)
    }
}

/// The inline module path of a `fn:` key (`tests::inner` for
/// `fn:tests::inner::works`), empty at the top of the file.
fn module_of(key: &str) -> String {
    let path = key.strip_prefix("fn:").unwrap_or(key);
    let path = path.rsplit_once('#').map_or(path, |(p, _)| p);
    path.rsplit_once("::")
        .map(|(m, _)| m.to_string())
        .unwrap_or_default()
}

/// The declarations from `file` up to its crate root: (parent file, key),
/// nearest first, and why the walk stopped short when it did (a cycle).
fn chain_of(
    parents: &BTreeMap<String, (String, String)>,
    file: &str,
) -> (Vec<(String, String)>, Option<String>) {
    let mut chain = Vec::new();
    let mut seen = BTreeSet::from([file.to_string()]);
    let mut current = file.to_string();
    while let Some((parent, decl)) = parents.get(&current) {
        if !seen.insert(parent.clone()) {
            return (
                chain,
                Some(format!(
                    "the module chain of {file} cycles back through {parent}; the modules on its way are not all pinned"
                )),
            );
        }
        chain.push((parent.clone(), decl.clone()));
        current = parent.clone();
    }
    (chain, None)
}

/// Whether `file` is compiled only for tests: its name says so, or a
/// declaration on its way from the crate root is `#[cfg(test)]`.
fn test_only_file(tree: &Tree, parents: &BTreeMap<String, (String, String)>, file: &str) -> bool {
    let name = file.rsplit('/').next().unwrap_or(file);
    if name.ends_with("_tests.rs") || name == "tests.rs" || file.contains("/tests/") {
        return true;
    }
    chain_of(parents, file).0.iter().any(|(parent, decl)| {
        std::fs::read_to_string(tree.base.join(parent))
            .ok()
            .and_then(|text| crate::check_source_rust::item_text(&text, decl))
            .is_some_and(|declared| declared.contains("cfg(test)"))
    })
}

/// A Rust test file and every module file it loads, recursively, each
/// pinned whole. `crate_root` for the target's top file.
pub(crate) fn rust_file_tree(
    roots: &Roots,
    abs: &Path,
    crate_root: bool,
    role: &str,
    out: &mut Resolution,
) {
    let mut pending = vec![(abs.to_path_buf(), crate_root)];
    let mut seen = BTreeSet::new();
    while let Some((file, top)) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        let Some((root, rel)) = out.file(roots, &file, role) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let base = roots.of(root);
        for child in mod_children(base, &rel, &text, top) {
            match child.escapes {
                true => out.unresolved.push(format!(
                    "{rel} loads a module through a path leaving its tree ({}); it is not pinned",
                    child.path
                )),
                false => pending.push((base.join(child.path), false)),
            }
        }
    }
}

/// Every module file reachable from `crate_roots` (tree-relative), and the
/// file and `mod:` key declaring it; module paths leaving the tree are
/// returned as unresolved.
fn module_parents(
    tree: &Tree,
    crate_roots: &[String],
) -> (BTreeMap<String, (String, String)>, Vec<String>) {
    let mut parents = BTreeMap::new();
    let mut escapes = Vec::new();
    let mut pending: Vec<(String, bool)> = crate_roots.iter().map(|r| (r.clone(), true)).collect();
    let mut seen = BTreeSet::new();
    while let Some((file, top)) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(tree.base.join(&file)) else {
            continue;
        };
        for child in mod_children(&tree.base, &file, &text, top) {
            if child.escapes {
                escapes.push(format!(
                    "{file} loads a module through a path leaving its tree ({}); it is not pinned",
                    child.path
                ));
                continue;
            }
            parents
                .entry(child.path.clone())
                .or_insert((file.clone(), child.decl));
            pending.push((child.path, false));
        }
    }
    (parents, escapes)
}

/// The crate roots of `package`'s library and binaries, tree-relative.
fn src_roots(tree: &Tree, package: &Path) -> Vec<String> {
    let mut files = vec![package.join("src/lib.rs"), package.join("src/main.rs")];
    files.extend(files_under(&package.join("src/bin")));
    files.iter().filter_map(|file| tree.rel(file)).collect()
}

/// The integration targets' roots: every `tests/*.rs`, tree-relative.
fn test_roots(tree: &Tree, package: &Path) -> Vec<String> {
    std::fs::read_dir(package.join("tests"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "rs"))
        .filter_map(|path| tree.rel(&path))
        .collect()
}

fn item(tree: &Tree, file: &str, key: String, role: &str, out: &mut Resolution) {
    if let Some((root, path)) = tree.roots.relative(&tree.base.join(file)) {
        out.found.push(Found {
            root,
            path,
            item: Some(key),
            role: role.to_string(),
        });
    }
}

/// Pin the declarations from the crate root down to `file`, and the inner
/// cfg attributes of `file` and of each file on the way. A chain that
/// cycles is recorded as unresolved, never silently cut short.
fn pin_chain(
    tree: &Tree,
    parents: &BTreeMap<String, (String, String)>,
    file: &str,
    out: &mut Resolution,
) {
    item(tree, file, CFG_KEY.into(), ROLE_CFG, out);
    let (chain, cycle) = chain_of(parents, file);
    for (parent, decl) in chain {
        for key in enclosing_mods(&decl).into_iter().chain([decl.clone()]) {
            item(tree, &parent, key, ROLE_DECLARATION, out);
        }
        item(tree, &parent, CFG_KEY.into(), ROLE_CFG, out);
    }
    out.unresolved.extend(cycle);
}

/// Test functions under `package/src` a filter selects (`None`: every one),
/// each pinned as an item with its helpers and the modules on its way.
/// Whether any matched.
pub(crate) fn src_items(
    roots: &Roots,
    package: &Path,
    filter: Option<(&str, bool)>,
    out: &mut Resolution,
) -> bool {
    let tree = Tree::of(roots, package);
    let (parents, escapes) = module_parents(&tree, &src_roots(&tree, package));
    out.unresolved.extend(escapes);
    let mut any = false;
    for file in files_under(&package.join("src")) {
        if file.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let hits = match filter {
            Some((name, exact)) => matching_tests(&text, name, exact),
            None => keyed_tests(&text),
        };
        let Some(file_rel) = tree.rel(&file).filter(|_| !hits.is_empty()) else {
            continue;
        };
        let test_only = test_only_file(&tree, &parents, &file_rel);
        let all_items = crate::check_source_rust::items(&text);
        for hit in hits {
            any = true;
            // The helpers a test calls sit beside it in its test module (or
            // anywhere in a test-only file): every non-test fn there is pinned
            // with it.
            let module = module_of(&hit.key);
            if !module.is_empty() || test_only {
                for helper in all_items.iter().filter(|i| {
                    !i.is_test && i.key.starts_with("fn:") && module_of(&i.key) == module
                }) {
                    item(&tree, &file_rel, helper.key.clone(), ROLE_HELPER, out);
                }
            }
            for key in enclosing_mods(&hit.key) {
                item(&tree, &file_rel, key, ROLE_DECLARATION, out);
            }
            item(&tree, &file_rel, hit.key, "unit test function", out);
        }
        pin_chain(&tree, &parents, &file_rel, out);
    }
    any
}

/// A `tests/` module file that defines a filtered test: pinned whole with
/// its module files, and the declarations from its target root down to it.
pub(crate) fn test_module(roots: &Roots, package: &Path, file: &Path, out: &mut Resolution) {
    let top = file.parent() == Some(package.join("tests").as_path());
    rust_file_tree(roots, file, top, "file defining the filtered test", out);
    if !top {
        let tree = Tree::of(roots, package);
        let (parents, escapes) = module_parents(&tree, &test_roots(&tree, package));
        out.unresolved.extend(escapes);
        if let Some(rel) = tree.rel(file) {
            pin_chain(&tree, &parents, &rel, out);
        }
    }
}

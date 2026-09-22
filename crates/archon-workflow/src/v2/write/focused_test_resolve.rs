//! Where a lib-test filter's module segments land under one package's
//! `src`: the exact walk from the root, then the suffix search across the
//! tree, longest prefix first (Issue-72). See [`super::focused_test_targets`]
//! for why both are needed and which wins.

use std::path::Path;

use super::focused_test_targets::{FocusedTestTarget, Resolution, module_dir};

/// `modules` under one package's `src`, longest prefix first: the exact
/// walk from the root at that length, else the suffix search at that
/// length (one hit resolves, several tie, none goes shorter). A prefix of
/// nothing but `tests` segments names no module of its own and is not
/// suffix-searched; the crate root is never a target.
pub(super) fn resolve_module(repo_root: &Path, src: &str, modules: &[&str]) -> Resolution {
    for len in (1..=modules.len()).rev() {
        let prefix = &modules[..len];
        if let Some(file) = exact_at(repo_root, src, prefix) {
            return Resolution::One(FocusedTestTarget {
                dir: module_dir(&file),
                file,
            });
        }
        if prefix.iter().all(|segment| *segment == "tests") {
            continue;
        }
        let hits = suffix_matches(repo_root, src, prefix);
        match hits.as_slice() {
            [] => {}
            [one] => {
                return Resolution::One(FocusedTestTarget {
                    dir: module_dir(one),
                    file: one.clone(),
                });
            }
            _ => {
                return Resolution::Ambiguous {
                    filter: modules.join("::"),
                    candidates: hits,
                };
            }
        }
    }
    Resolution::None
}

/// The file the whole of `prefix` reaches from the src root, by the same
/// three shapes [`super::test_baseline_owner::resolve_under`] walks: `a/b.rs`,
/// `a/b/mod.rs`, and for a `tests` leaf the `#[path]` sibling `a_tests.rs`.
fn exact_at(repo_root: &Path, src: &str, prefix: &[&str]) -> Option<String> {
    let joined = prefix.join("/");
    let mut candidates = vec![
        format!("{src}/{joined}.rs"),
        format!("{src}/{joined}/mod.rs"),
    ];
    if let Some((&"tests", parents)) = prefix.split_last()
        && !parents.is_empty()
    {
        candidates.push(format!("{src}/{}_tests.rs", parents.join("/")));
    }
    candidates
        .into_iter()
        .find(|candidate| repo_root.join(candidate).is_file())
}

/// Cap on the files read while suffix-searching one package's `src` tree.
const SUFFIX_WALK_MAX_ENTRIES: usize = 20_000;
/// Cap on directory depth below `src` for the suffix search.
const SUFFIX_WALK_MAX_DEPTH: usize = 16;

/// The module files under `src` (repo-relative, sorted) whose module path
/// ends with `suffix`. A file `p/q/r.rs` is module `p::q::r`, `p/q/r/mod.rs`
/// is `p::q::r`, and `p/q_tests.rs` is both `p::q_tests` and (its `#[path]`
/// reading) `p::q::tests`. The crate root, hidden directories, `target`
/// and symlinks are skipped; the walk is capped in depth and entries.
fn suffix_matches(repo_root: &Path, src: &str, suffix: &[&str]) -> Vec<String> {
    let mut hits = Vec::new();
    let mut budget = SUFFIX_WALK_MAX_ENTRIES;
    let mut stack: Vec<(Vec<String>, usize)> = vec![(Vec::new(), 0)];
    while let Some((dir_modules, depth)) = stack.pop() {
        let dir = dir_modules
            .iter()
            .fold(repo_root.join(src), |path, module| path.join(module));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if budget == 0 {
                return finish(hits);
            }
            budget -= 1;
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || name == "target" || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth < SUFFIX_WALK_MAX_DEPTH {
                    let mut modules = dir_modules.clone();
                    modules.push(name.to_string());
                    stack.push((modules, depth + 1));
                }
                continue;
            }
            let Some(stem) = name.strip_suffix(".rs") else {
                continue;
            };
            if dir_modules.is_empty() && matches!(stem, "lib" | "main") {
                continue;
            }
            let rel = if dir_modules.is_empty() {
                format!("{src}/{name}")
            } else {
                format!("{src}/{}/{name}", dir_modules.join("/"))
            };
            for module_path in module_paths(&dir_modules, stem) {
                if module_path.ends_with(suffix) {
                    hits.push(rel.clone());
                    break;
                }
            }
        }
    }
    finish(hits)
}

fn finish(mut hits: Vec<String>) -> Vec<String> {
    hits.sort();
    hits.dedup();
    hits
}

/// The module path(s) a file `<dir_modules>/<stem>.rs` declares.
fn module_paths<'a>(dir_modules: &'a [String], stem: &'a str) -> Vec<Vec<&'a str>> {
    let base: Vec<&str> = dir_modules.iter().map(String::as_str).collect();
    if stem == "mod" {
        return vec![base];
    }
    let mut literal = base.clone();
    literal.push(stem);
    let mut paths = vec![literal];
    if let Some(parent) = stem.strip_suffix("_tests")
        && !parent.is_empty()
    {
        let mut aliased = base;
        aliased.push(parent);
        aliased.push("tests");
        paths.push(aliased);
    }
    paths
}

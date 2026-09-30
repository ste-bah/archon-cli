//! Batch O: which tasks' declared files reference a source file no task
//! declares -- the code-dependency tier of the ownerless-file assignment.
//!
//! A file no task declares is still some task's to keep working when that
//! task's own code uses it. The host reads that from the code itself, never
//! from agent text: for every declared Rust source file of every task, the
//! files it references are
//!
//! - its child modules (`mod name;` -> `name.rs` or `name/mod.rs` beside the
//!   module's directory), and
//! - the files that define an item it names -- after `::` in a path (`use`
//!   lists included) or as a call (`name(`) -- when exactly ONE source file
//!   of the indexed crates defines an item of that name (`fn`, `struct`,
//!   `enum`, `trait`, `type`, `const`, `static`, `mod`, `macro_rules!`). A
//!   name defined in several files references none of them: it proves
//!   nothing.
//!
//! Only the crates holding a declared source file are indexed. The result
//! maps each referenced file to the tasks whose declared files reference it
//! directly (the nearest: a direct reference is distance one; ties are all
//! kept, and the caller shares the file among them).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};

/// Repository-relative source file -> the tasks whose declared files
/// reference it directly.
pub fn referencing_tasks(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
) -> BTreeMap<String, BTreeSet<String>> {
    let declared = declared_sources(universe, root);
    let crates: BTreeSet<PathBuf> = declared
        .values()
        .flatten()
        .filter_map(|file| crate_src(root, file))
        .collect();
    let mut files: Vec<String> = Vec::new();
    for src in &crates {
        collect_sources(root, src, &mut files);
    }
    let index = definitions(root, &files);
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (task, sources) in &declared {
        for file in sources {
            for target in references(root, file, &index) {
                if &target != file {
                    out.entry(target).or_default().insert(task.clone());
                }
            }
        }
    }
    out
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

/// The `src` directory of the crate holding `file` (the nearest ancestor
/// with a `Cargo.toml`), repository-relative.
fn crate_src(root: &Path, file: &str) -> Option<PathBuf> {
    let mut dir = Path::new(file).parent();
    while let Some(current) = dir {
        if root.join(current).join("Cargo.toml").is_file() {
            return Some(current.join("src"));
        }
        dir = current.parent();
    }
    root.join("Cargo.toml")
        .is_file()
        .then(|| PathBuf::from("src"))
}

fn collect_sources(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let rel = dir.join(entry.file_name());
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect_sources(root, &rel, out),
            Ok(kind) if kind.is_file() && rel.extension().is_some_and(|ext| ext == "rs") => {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
            _ => {}
        }
    }
}

/// Item name -> the files defining an item of that name.
fn definitions(root: &Path, files: &[String]) -> BTreeMap<String, BTreeSet<String>> {
    let item = Regex::new(
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|const\s+|unsafe\s+)*(?:fn|struct|enum|trait|type|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)|macro_rules!\s*([A-Za-z_][A-Za-z0-9_]*)",
    )
    .expect("item pattern");
    let mut index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        for caps in item.captures_iter(&text) {
            if let Some(name) = caps.get(1).or_else(|| caps.get(2)) {
                index
                    .entry(name.as_str().to_string())
                    .or_default()
                    .insert(file.clone());
            }
        }
    }
    index
}

/// The files `file` references (see the module doc).
fn references(
    root: &Path,
    file: &str,
    index: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(text) = std::fs::read_to_string(root.join(file)) else {
        return out;
    };
    let child = Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
        .expect("mod pattern");
    let path = Path::new(file);
    let module_dir = match path.file_name().and_then(|name| name.to_str()) {
        Some("mod.rs" | "lib.rs" | "main.rs") => path.parent().map(Path::to_path_buf),
        _ => path
            .parent()
            .map(|parent| parent.join(path.file_stem().unwrap_or_default())),
    }
    .unwrap_or_default();
    for caps in child.captures_iter(&text) {
        let name = &caps[1];
        for candidate in [
            module_dir.join(format!("{name}.rs")),
            module_dir.join(name).join("mod.rs"),
        ] {
            if root.join(&candidate).is_file() {
                out.insert(candidate.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let named = Regex::new(r"::\s*([A-Za-z_][A-Za-z0-9_]*)|\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .expect("name pattern");
    let braces = Regex::new(r"::\s*\{([^}]*)\}").expect("use list pattern");
    let mut names: BTreeSet<&str> = named
        .captures_iter(&text)
        .filter_map(|caps| caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str()))
        .collect();
    for caps in braces.captures_iter(&text) {
        let list = caps.get(1).map_or("", |m| m.as_str());
        names.extend(
            list.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|word| !word.is_empty()),
        );
    }
    for name in names {
        if let Some(defining) = index.get(name)
            && defining.len() == 1
        {
            out.extend(defining.iter().cloned());
        }
    }
    out
}

#[cfg(test)]
#[path = "task_scope_amendment_refs_tests.rs"]
mod tests;

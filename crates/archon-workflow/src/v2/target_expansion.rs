//! Expanding a declared Rust target into the module files it really owns.
//!
//! A task that declares `foo.rs` also owns the file-backed modules that file
//! declares, and the module directory `foo/` those splits land in. Both the
//! source graph and the write coordinator have to agree on that set, so the
//! expansion lives beside the write plan it feeds rather than in the binary.
//!
//! A hub is the exception (Issue-48): a directory's `mod.rs` whose module
//! fan-out is above [`HUB_MODULE_FAN_OUT_CAP`] is a registry, not a split,
//! and owns itself only.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::v2::{WorkflowV2WriteSafetyError, normalize_targets_for_repository};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedTargetFiles {
    pub declared_target_files: Vec<String>,
    pub target_files: Vec<String>,
    pub target_dir_scopes: Vec<String>,
    pub target_file_expansions: Vec<TargetFileExpansion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetFileExpansion {
    pub source: String,
    pub expanded: Vec<String>,
    pub dir_scopes: Vec<String>,
    pub notes: Vec<String>,
}

/// The most file-backed modules a directory's `mod.rs` may fan out to
/// (counted transitively, since ownership walks the module tree) and still
/// own them. Above this it is a hub.
///
/// Live (Issue-48): a task declared a crate-level command registry module, a
/// `mod.rs` whose only job is to declare several hundred sibling modules. The
/// split rule made that one target own 651 files, the whole command tree: the
/// per-wave repository audit assessed 644 "added declared paths" one record
/// at a time (2.7 hours for one wave), the task "owned" files other tasks
/// declared, and its write scope meant nothing. A registry is not a split of
/// the declaring file, so a hub owns itself and its `include!`d files only,
/// and contributes no module-directory scope.
///
/// The cap applies to a `mod.rs`, whose declared modules are the siblings it
/// shares a directory with, and not to a `foo.rs` whose modules live in
/// `foo/`: that directory exists only as the split of `foo.rs`, and a real
/// split of one large file into its module directory runs to dozens of files
/// (the WF98 fixture's declared parent carries 26 direct children), so no
/// count separates a split from a registry. A `mod.rs` above sixteen is a
/// registry of the modules other tasks declare, not a file that split.
pub const HUB_MODULE_FAN_OUT_CAP: usize = 16;

pub fn expand_declared_rust_module_targets(
    item_id: &str,
    targets: &[String],
    repository_root: Option<&str>,
) -> Result<ExpandedTargetFiles, WorkflowV2WriteSafetyError> {
    let declared_target_files =
        normalize_targets_for_repository(item_id, targets, repository_root)?;
    let repository_root = repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from);
    let mut effective_targets = BTreeSet::new();
    let mut effective_scopes = BTreeSet::new();
    let mut target_file_expansions = Vec::new();
    // Ownership is transitive: a declared `a.rs` owns `a/b.rs`, and `a/b.rs`
    // owns `a/b/c.rs` just as directly. Expanding the declared list in one
    // pass stopped at the first generation, so a grandchild module was left
    // unowned and the branch that edited it failed write-scope on its own
    // file. Walk until nothing new appears instead.
    let declared_lookup: BTreeSet<String> = declared_target_files.iter().cloned().collect();
    let mut pending: Vec<String> = declared_target_files.clone();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    while let Some(target) = pending.pop() {
        if !visited.insert(target.clone()) {
            // A module cycle would otherwise queue forever.
            continue;
        }
        effective_targets.insert(target.clone());
        let Some(root) = repository_root.as_deref() else {
            continue;
        };
        if let Some(expansion) = target_file_expansion(root, &target) {
            for expanded in &expansion.expanded {
                effective_targets.insert(expanded.clone());
                if !visited.contains(expanded) {
                    pending.push(expanded.clone());
                }
            }
            // Directory scope stays with the *declared* targets. It exists so
            // a declared file at the size cap can split into its own module
            // directory; granting it for every transitively owned file would
            // widen write scope well past the ownership this fix restores.
            if declared_lookup.contains(&target) {
                for scope in &expansion.dir_scopes {
                    effective_scopes.insert(scope.clone());
                }
            }
            if !expansion.expanded.is_empty()
                || !expansion.dir_scopes.is_empty()
                || !expansion.notes.is_empty()
            {
                target_file_expansions.push(expansion);
            }
        }
    }
    Ok(ExpandedTargetFiles {
        declared_target_files,
        target_files: effective_targets.into_iter().collect(),
        target_dir_scopes: effective_scopes.into_iter().collect(),
        target_file_expansions,
    })
}

/// The expansion of one file with the hub cap applied: a split owns its
/// modules, its includes and its module directory; a hub owns its includes.
fn target_file_expansion(repository_root: &Path, target: &str) -> Option<TargetFileExpansion> {
    let expansion = rust_module_expansion(repository_root, target)?;
    let fan_out = if is_directory_registry(target) {
        transitive_module_fan_out(repository_root, target, HUB_MODULE_FAN_OUT_CAP)
    } else {
        0
    };
    let mut notes = expansion.notes;
    let (expanded, dir_scopes) = if fan_out > HUB_MODULE_FAN_OUT_CAP {
        let direct = expansion.modules.len();
        notes.push(format!(
            "hub module: declares {direct} file-backed modules (more than \
             {HUB_MODULE_FAN_OUT_CAP} transitively), above the cap of \
             {HUB_MODULE_FAN_OUT_CAP}; owning the declaring file only"
        ));
        (expansion.included, Vec::new())
    } else {
        let mut expanded = expansion.modules;
        expanded.extend(expansion.included);
        (
            expanded,
            module_dir_scope(repository_root, &expansion.module_dir),
        )
    };
    Some(TargetFileExpansion {
        source: target.to_string(),
        expanded: expanded
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        dir_scopes,
        notes,
    })
}

/// A `mod.rs`: the file that registers a directory's modules, as opposed to a
/// `foo.rs` whose modules are the split of itself into `foo/`.
fn is_directory_registry(target: &str) -> bool {
    Path::new(target)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "mod")
}

/// How many file-backed modules `target` reaches through `mod` declarations,
/// counted transitively (ownership reaches grandchildren, so the cap must
/// too) and stopped as soon as the count passes `cap`, so a registry of
/// hundreds costs `cap + 1` module reads rather than hundreds.
fn transitive_module_fan_out(repository_root: &Path, target: &str, cap: usize) -> usize {
    let mut reached = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![target.to_string()];
    while let Some(file) = pending.pop() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let Some(expansion) = rust_module_expansion(repository_root, &file) else {
            continue;
        };
        for module in expansion.modules {
            if reached.insert(module.clone()) {
                if reached.len() > cap {
                    return reached.len();
                }
                pending.push(module);
            }
        }
    }
    reached.len()
}

/// What one file declares, before the hub cap decides what it owns.
struct RustModuleExpansion {
    modules: Vec<String>,
    included: Vec<String>,
    module_dir: PathBuf,
    notes: Vec<String>,
}

fn rust_module_expansion(repository_root: &Path, target: &str) -> Option<RustModuleExpansion> {
    let target_path = Path::new(target);
    if target_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return None;
    }
    if target_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "lib.rs" | "main.rs"))
    {
        return None;
    }
    let absolute_target = repository_root.join(target_path);
    if !absolute_target.is_file() {
        return None;
    }
    let source = fs::read_to_string(&absolute_target).ok()?;
    let module_dir = module_directory_for_target(target_path)?;
    let mut modules = BTreeSet::new();
    let mut notes = Vec::new();
    // `#[path]` is relative to the directory holding the declaring file, not
    // to the module directory the convention would use.
    let declaring_dir = target_path.parent().unwrap_or_else(|| Path::new(""));
    for module in declared_file_modules(&source) {
        let module_name = &module.name;
        let candidates = match &module.explicit_path {
            Some(path) => vec![repository_root.join(declaring_dir).join(path)],
            None => vec![
                repository_root
                    .join(&module_dir)
                    .join(format!("{module_name}.rs")),
                repository_root
                    .join(&module_dir)
                    .join(module_name)
                    .join("mod.rs"),
            ],
        };
        match candidates.iter().find(|candidate| candidate.is_file()) {
            Some(resolved) => {
                if let Some(relative) = repo_relative(repository_root, resolved) {
                    modules.insert(relative);
                }
            }
            None => notes.push(format!(
                "declared module '{module_name}' from '{target}' has no file-backed target"
            )),
        }
    }
    // `include!` splices a file's text into this one, so the included file is
    // literally part of the declaring file — editing it edits the owner. It is
    // not a `mod`, so module resolution never saw it and the file stayed
    // unowned while its owner was owned.
    let mut included_targets = BTreeSet::new();
    for included in included_files(&source) {
        let candidate = repository_root.join(declaring_dir).join(&included);
        match repo_relative(repository_root, &candidate) {
            Some(relative) if candidate.is_file() => {
                included_targets.insert(relative);
            }
            _ => notes.push(format!(
                "included file '{included}' from '{target}' does not resolve"
            )),
        }
    }
    Some(RustModuleExpansion {
        modules: modules.into_iter().collect(),
        included: included_targets.into_iter().collect(),
        module_dir,
        notes,
    })
}

/// Owning a declared `foo.rs` also owns its module directory `foo/`, even when
/// that directory has no files yet. Without this a task whose declared target
/// is already at the file-size cap is deadlocked: it cannot add lines (hygiene
/// gate) and cannot split the file either (the split destinations would be
/// undeclared paths). Generic to any Rust target in any PRD.
fn module_dir_scope(repository_root: &Path, module_dir: &Path) -> Vec<String> {
    repo_relative(repository_root, &repository_root.join(module_dir))
        .into_iter()
        .collect()
}

fn module_directory_for_target(target: &Path) -> Option<PathBuf> {
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    let stem = target.file_stem()?.to_str()?;
    if stem == "mod" {
        return Some(parent.to_path_buf());
    }
    Some(parent.join(stem))
}

/// Files spliced in with `include!("literal")`, relative to the declaring
/// file's directory.
///
/// Only a bare string literal is taken. `include!(concat!(env!("OUT_DIR"), …))`
/// names a build-time artefact, not a repository file, and resolving it would
/// invent a target that no task can own.
fn included_files(source: &str) -> Vec<String> {
    let mut included = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let Some(rest) = line.split_once("include!(") else {
            continue;
        };
        let Some(quoted) = rest.1.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(value) = quoted.split('"').next() else {
            continue;
        };
        if !value.trim().is_empty() {
            included.push(value.to_string());
        }
    }
    included
}

/// A module a file declares, and the `#[path]` it was given if any.
///
/// `#[path]` is not decoration: it moves the module's file somewhere the
/// name-to-path convention will never look. Resolving by convention alone
/// reported "no file-backed target" for a file that plainly exists, so the
/// task never owned it, and the agent that had to edit it lost its whole
/// branch to a write-scope escape on a file it legitimately owned.
struct DeclaredModule {
    name: String,
    explicit_path: Option<String>,
}

fn declared_file_modules(source: &str) -> Vec<DeclaredModule> {
    let mut modules = Vec::new();
    let mut seen = BTreeSet::new();
    // `#[path = "..."]` applies to the next `mod` declaration, and is written
    // on its own line as often as inline.
    let mut pending_path = None;
    for line in source.lines() {
        let mut line = line.split("//").next().unwrap_or("").trim();
        if let Some((path, remainder)) = path_attribute(line) {
            pending_path = Some(path);
            // The attribute may lead a `mod` on the same line; keep parsing
            // what follows it rather than discarding the declaration.
            line = remainder;
            if line.is_empty() {
                continue;
            }
        }
        if !line.ends_with(';') || line.contains('{') {
            // An attribute only survives to the declaration it precedes — but
            // other attributes may sit between the two. `#[cfg(test)]` and
            // `#[path]` are written in either order, and clearing on any
            // non-empty line dropped the path for one of those orders.
            if !line.is_empty() && !line.starts_with("#[") {
                pending_path = None;
            }
            continue;
        }
        let declaration = line.trim_end_matches(';').trim();
        let tokens = declaration.split_whitespace().collect::<Vec<_>>();
        let mut declared_here = false;
        for (index, token) in tokens.iter().enumerate() {
            if *token != "mod" || index + 1 >= tokens.len() {
                continue;
            }
            let prefix_allowed = tokens[..index].iter().all(|prefix| {
                *prefix == "pub" || prefix.starts_with("pub(") || prefix.starts_with("#[")
            });
            if !prefix_allowed {
                continue;
            }
            let module_name = tokens[index + 1].trim_start_matches("r#");
            if is_rust_identifier(module_name) && seen.insert(module_name.to_string()) {
                modules.push(DeclaredModule {
                    name: module_name.to_string(),
                    explicit_path: pending_path.clone(),
                });
                declared_here = true;
            }
        }
        if declared_here || !declaration.is_empty() {
            pending_path = None;
        }
    }
    modules
}

/// The value of a `#[path = "..."]` attribute plus whatever follows it on the
/// line, so an attribute leading a `mod` on the same line does not hide the
/// declaration it applies to.
fn path_attribute(line: &str) -> Option<(String, &str)> {
    let start = line.find("#[path")?;
    let rest = line[start..].strip_prefix("#[path")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quoted = rest.strip_prefix('"')?;
    let value = quoted.split('"').next()?;
    if value.trim().is_empty() {
        return None;
    }
    let after = quoted.get(value.len()..)?;
    let remainder = after
        .split_once(']')
        .map(|(_, tail)| tail.trim())
        .unwrap_or("");
    Some((value.to_string(), remainder))
}

fn is_rust_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// The repository-relative path with `.` and `..` folded away, so a module
/// reached through `#[path = "../a.rs"]` is the same target as `a.rs` and a
/// declaration cycle is seen as one. Left unfolded, each lap of a cycle
/// minted a new `src/a/../a/../a.rs` spelling and the walk only stopped when
/// the path outgrew the OS limit. A path that climbs above the root is none.
fn repo_relative(repository_root: &Path, path: &Path) -> Option<String> {
    use std::path::Component;
    let relative = path.strip_prefix(repository_root).ok()?;
    let mut segments: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(segment) => segments.push(segment.to_string_lossy().into_owned()),
            Component::ParentDir => {
                segments.pop()?;
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

#[cfg(test)]
#[path = "target_expansion_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "target_expansion_hub_tests.rs"]
mod hub_tests;

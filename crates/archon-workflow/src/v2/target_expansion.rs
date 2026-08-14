//! Expanding a declared Rust target into the module files it really owns.
//!
//! A task that declares `foo.rs` also owns the file-backed modules that file
//! declares, and the module directory `foo/` those splits land in. Both the
//! source graph and the write coordinator have to agree on that set, so the
//! expansion lives beside the write plan it feeds rather than in the binary.

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
        if let Some(expansion) = rust_module_expansion(root, &target) {
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

fn rust_module_expansion(repository_root: &Path, target: &str) -> Option<TargetFileExpansion> {
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
    let mut expanded = BTreeSet::new();
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
                    expanded.insert(relative);
                }
            }
            None => notes.push(format!(
                "declared module '{module_name}' from '{target}' has no file-backed target"
            )),
        }
    }
    let dir_scopes = module_dir_scope(repository_root, &module_dir);
    Some(TargetFileExpansion {
        source: target.to_string(),
        expanded: expanded.into_iter().collect(),
        dir_scopes,
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

fn repo_relative(repository_root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(repository_root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_declared_file_backed_modules_from_sibling_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/foo")).expect("module dir");
        fs::write(repo.join("src/foo.rs"), "mod bar;\npub mod baz;\n").expect("foo");
        fs::write(repo.join("src/foo/bar.rs"), "").expect("bar");
        fs::write(repo.join("src/foo/baz.rs"), "").expect("baz");

        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/foo.rs".to_string()],
            Some(&repo.display().to_string()),
        )
        .expect("expanded");

        assert_eq!(expanded.declared_target_files, vec!["src/foo.rs"]);
        assert_eq!(
            expanded.target_files,
            vec!["src/foo.rs", "src/foo/bar.rs", "src/foo/baz.rs"]
        );
        assert_eq!(expanded.target_dir_scopes, vec!["src/foo"]);
        assert_eq!(expanded.target_file_expansions[0].source, "src/foo.rs");
    }

    #[test]
    fn module_directory_expansion_allows_new_child_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/data_store")).expect("module dir");
        fs::write(repo.join("src/data_store.rs"), "mod io;\n").expect("module");
        fs::write(repo.join("src/data_store/io.rs"), "").expect("child");

        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/data_store.rs".to_string()],
            Some(&repo.display().to_string()),
        )
        .expect("expanded");

        assert!(
            !expanded
                .target_files
                .contains(&"src/data_store".to_string())
        );
        assert!(
            expanded
                .target_dir_scopes
                .contains(&"src/data_store".to_string())
        );
    }

    /// `#[cfg(test)]` and `#[path]` are written in either order in this
    /// repository. Both must resolve, or whichever order is unhandled silently
    /// loses ownership of the file.
    #[test]
    fn a_path_attribute_survives_a_neighbouring_attribute() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/compression")).expect("dir");
        fs::write(repo.join("src/compression/tests.rs"), "").expect("tests");

        for source in [
            "#[cfg(test)]\n#[path = \"compression/tests.rs\"]\nmod tests;\n",
            "#[path = \"compression/tests.rs\"]\n#[cfg(test)]\nmod tests;\n",
        ] {
            fs::write(repo.join("src/compression.rs"), source).expect("declaring file");
            let expanded = expand_declared_rust_module_targets(
                "item",
                &["src/compression.rs".to_string()],
                repo.to_str(),
            )
            .expect("expansion");
            assert!(
                expanded
                    .target_files
                    .contains(&"src/compression/tests.rs".to_string()),
                "attribute order must not change ownership: {source:?} -> {:?}",
                expanded.target_files
            );
        }
    }

    /// The live failure after the `#[path]` fix. `data_lake.rs` declares
    /// `mod tests;`, and `data_lake/tests.rs` declares `mod
    /// artifact_tolerance;`. One pass found the child and stopped, so the
    /// grandchild was unowned and its branch failed write-scope.
    #[test]
    fn ownership_reaches_a_grandchild_module() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/data_lake/tests")).expect("dirs");
        fs::write(repo.join("src/data_lake.rs"), "mod tests;\n").expect("root");
        fs::write(
            repo.join("src/data_lake/tests.rs"),
            "mod artifact_tolerance;\n",
        )
        .expect("child");
        fs::write(repo.join("src/data_lake/tests/artifact_tolerance.rs"), "").expect("grandchild");

        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/data_lake.rs".to_string()],
            repo.to_str(),
        )
        .expect("expansion");

        assert!(
            expanded
                .target_files
                .contains(&"src/data_lake/tests.rs".to_string())
        );
        assert!(
            expanded
                .target_files
                .contains(&"src/data_lake/tests/artifact_tolerance.rs".to_string()),
            "a grandchild module must be owned: {:?}",
            expanded.target_files
        );
    }

    /// Transitive walking must terminate even if two files declare each other.
    #[test]
    fn a_module_cycle_terminates() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/a")).expect("dir a");
        fs::create_dir_all(repo.join("src/a/b")).expect("dir b");
        fs::write(repo.join("src/a.rs"), "mod b;\n").expect("a");
        // `a/b.rs` points back at `a.rs` through an explicit path.
        fs::write(repo.join("src/a/b.rs"), "#[path = \"../a.rs\"]\nmod a;\n").expect("b");

        let expanded =
            expand_declared_rust_module_targets("item", &["src/a.rs".to_string()], repo.to_str())
                .expect("expansion");

        assert!(expanded.target_files.contains(&"src/a/b.rs".to_string()));
    }

    #[test]
    fn inline_modules_do_not_invent_file_targets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/foo")).expect("module dir");
        fs::write(repo.join("src/foo.rs"), "mod inline {}\nmod missing;\n").expect("foo");

        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/foo.rs".to_string()],
            Some(&repo.display().to_string()),
        )
        .expect("expanded");

        assert_eq!(expanded.target_files, vec!["src/foo.rs"]);
        assert!(expanded.target_file_expansions[0].notes[0].contains("declared module 'missing'"));
    }

    #[test]
    fn lib_and_main_are_not_broadly_expanded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/sub")).expect("module dir");
        fs::write(repo.join("src/lib.rs"), "mod sub;\n").expect("lib");
        fs::write(repo.join("src/sub.rs"), "").expect("sub");

        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/lib.rs".to_string()],
            Some(&repo.display().to_string()),
        )
        .expect("expanded");

        assert_eq!(expanded.target_files, vec!["src/lib.rs"]);
        assert!(expanded.target_file_expansions.is_empty());
    }

    #[test]
    fn unsafe_targets_still_reject() {
        let error = expand_declared_rust_module_targets(
            "item",
            &["../outside.rs".to_string()],
            Some("/repo"),
        )
        .expect_err("unsafe target");

        assert!(error.to_string().contains("unsafe"));
    }

    /// The live failure. `validation_tests.rs` declares its cases with
    /// `#[path = "tests/…"]`, which puts them outside the module directory the
    /// convention searches. Resolving by convention left them unowned, and the
    /// branch that edited one lost on write-scope for touching its own file.
    #[test]
    fn a_module_moved_by_a_path_attribute_is_still_owned() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/data_store/tests")).expect("tests dir");
        fs::create_dir_all(repo.join("src/data_store/validation_tests")).expect("module dir");
        fs::write(
            repo.join("src/data_store/validation_tests.rs"),
            "#[path = \"tests/validation_atomicity.rs\"]\nmod validation_atomicity;\n\
             #[path = \"validation_tests/contract_core.rs\"]\nmod contract_core;\n",
        )
        .expect("declaring file");
        fs::write(
            repo.join("src/data_store/tests/validation_atomicity.rs"),
            "",
        )
        .expect("relocated");
        fs::write(
            repo.join("src/data_store/validation_tests/contract_core.rs"),
            "",
        )
        .expect("conventional");

        let expanded = expand_declared_rust_module_targets(
            "item",
            &["src/data_store/validation_tests.rs".to_string()],
            repo.to_str(),
        )
        .expect("expansion");

        assert!(
            expanded
                .target_files
                .contains(&"src/data_store/tests/validation_atomicity.rs".to_string()),
            "a #[path]-relocated module must be owned: {:?}",
            expanded.target_files
        );
        assert!(
            expanded
                .target_files
                .contains(&"src/data_store/validation_tests/contract_core.rs".to_string())
        );
        assert!(
            expanded
                .target_file_expansions
                .iter()
                .all(|expansion| expansion.notes.is_empty()),
            "no module should be reported unresolvable"
        );
    }

    /// An inline attribute is the same declaration written differently.
    #[test]
    fn an_inline_path_attribute_resolves_too() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/elsewhere")).expect("dir");
        fs::write(
            repo.join("src/foo.rs"),
            "#[path = \"elsewhere/bar.rs\"] mod bar;\n",
        )
        .expect("foo");
        fs::write(repo.join("src/elsewhere/bar.rs"), "").expect("bar");

        let expanded =
            expand_declared_rust_module_targets("item", &["src/foo.rs".to_string()], repo.to_str())
                .expect("expansion");

        assert!(
            expanded
                .target_files
                .contains(&"src/elsewhere/bar.rs".to_string()),
            "{:?}",
            expanded.target_files
        );
    }

    /// A `#[path]` must not leak onto an unrelated later declaration.
    #[test]
    fn a_path_attribute_applies_only_to_the_next_module() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path();
        fs::create_dir_all(repo.join("src/foo")).expect("dir");
        fs::write(
            repo.join("src/foo.rs"),
            "#[path = \"foo/moved.rs\"]\nmod moved;\nmod plain;\n",
        )
        .expect("foo");
        fs::write(repo.join("src/foo/moved.rs"), "").expect("moved");
        fs::write(repo.join("src/foo/plain.rs"), "").expect("plain");

        let expanded =
            expand_declared_rust_module_targets("item", &["src/foo.rs".to_string()], repo.to_str())
                .expect("expansion");

        assert!(
            expanded
                .target_files
                .contains(&"src/foo/moved.rs".to_string())
        );
        assert!(
            expanded
                .target_files
                .contains(&"src/foo/plain.rs".to_string()),
            "the second module resolves by convention, not by the earlier attribute: {:?}",
            expanded.target_files
        );
    }
}

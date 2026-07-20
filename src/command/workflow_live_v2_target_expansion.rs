use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowV2WriteSafetyError, normalize_targets_for_repository};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpandedTargetFiles {
    pub(super) declared_target_files: Vec<String>,
    pub(super) target_files: Vec<String>,
    pub(super) target_dir_scopes: Vec<String>,
    pub(super) target_file_expansions: Vec<TargetFileExpansion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TargetFileExpansion {
    pub(super) source: String,
    pub(super) expanded: Vec<String>,
    pub(super) dir_scopes: Vec<String>,
    pub(super) notes: Vec<String>,
}

pub(super) fn expand_declared_rust_module_targets(
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
    for target in &declared_target_files {
        effective_targets.insert(target.clone());
        let Some(root) = repository_root.as_deref() else {
            continue;
        };
        if let Some(expansion) = rust_module_expansion(root, target) {
            for expanded in &expansion.expanded {
                effective_targets.insert(expanded.clone());
            }
            for scope in &expansion.dir_scopes {
                effective_scopes.insert(scope.clone());
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
    for module_name in declared_file_modules(&source) {
        let file_candidate = repository_root
            .join(&module_dir)
            .join(format!("{module_name}.rs"));
        let mod_rs_candidate = repository_root
            .join(&module_dir)
            .join(&module_name)
            .join("mod.rs");
        if file_candidate.is_file() {
            if let Some(relative) = repo_relative(repository_root, &file_candidate) {
                expanded.insert(relative);
            }
        } else if mod_rs_candidate.is_file() {
            if let Some(relative) = repo_relative(repository_root, &mod_rs_candidate) {
                expanded.insert(relative);
            }
        } else {
            notes.push(format!(
                "declared module '{module_name}' from '{target}' has no file-backed target"
            ));
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

fn declared_file_modules(source: &str) -> Vec<String> {
    let mut modules = BTreeSet::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if !line.ends_with(';') || line.contains('{') {
            continue;
        }
        let line = line.trim_end_matches(';').trim();
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        for (index, token) in tokens.iter().enumerate() {
            if *token != "mod" || index + 1 >= tokens.len() {
                continue;
            }
            let prefix_allowed = tokens[..index]
                .iter()
                .all(|prefix| *prefix == "pub" || prefix.starts_with("pub("));
            if !prefix_allowed {
                continue;
            }
            let module_name = tokens[index + 1].trim_start_matches("r#");
            if is_rust_identifier(module_name) {
                modules.insert(module_name.to_string());
            }
        }
    }
    modules.into_iter().collect()
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
}

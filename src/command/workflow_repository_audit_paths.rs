//! Repository declarations use the existing root and artifact namespaces.
//! A declared path the repository ignores is a project artifact (Issue-26):
//! the write layer retains it as a run artifact and never commits it, so the
//! audit is never asked to see it delivered.
use archon_workflow::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};
use std::{collections::BTreeSet, path::Path};

pub(super) fn repository_paths(
    paths: BTreeSet<String>,
    root: Option<&str>,
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<Vec<String>> {
    let context = archon_workflow::project_artifact_context_from_v2_root(store.root());
    let mut repository = BTreeSet::new();
    for raw in paths {
        let expanded = archon_workflow::v2::artifact_path_guard::expand_project_root_template(
            &raw,
            context.project_root.as_deref(),
        )
        .map_err(|error| WorkflowError::SpecInvalid(format!("audit declaration: {error}")))?;
        let path = Path::new(&expanded);
        let artifact = context.artifact_roots.iter().any(|artifact_root| {
            let artifact_root = Path::new(artifact_root);
            path.starts_with(artifact_root)
                || context.project_root.as_ref().is_some_and(|project| {
                    path.is_absolute() && path.starts_with(Path::new(project).join(artifact_root))
                })
        });
        if artifact {
            continue;
        }
        // Without a repository, deliverables belong to project verification;
        // the lifecycle still records its explicit no-repository assessment.
        let Some(root) = root else {
            continue;
        };
        let normalized = archon_workflow::v2::normalize_target_for_repository(
            "repository-audit",
            expanded.trim_end_matches('/'),
            Some(root),
        )
        .map_err(|error| WorkflowError::SpecInvalid(format!("audit declaration: {error}")))?;
        archon_workflow::repository_audit::contract::validate_path(&normalized)
            .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
        repository.insert(normalized);
    }
    let Some(root) = root else {
        return Ok(repository.into_iter().collect());
    };
    Ok(
        archon_workflow::repository_audit::ignored::repository_deliverables(
            Path::new(root),
            repository.into_iter().collect(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::repository_paths;
    use archon_workflow::WorkflowV2ResultStore;
    use std::collections::BTreeSet;

    /// A repository whose `.gitignore` ignores `docs/*`, with `src/y.rs`
    /// committed and `docs/tracked.md` committed before the rule existed.
    fn repository(temp: &std::path::Path) -> std::path::PathBuf {
        let repo = temp.join("repo");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/y.rs"), "fn y() {}\n").unwrap();
        std::fs::write(repo.join("docs/tracked.md"), "kept\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "/docs/*\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "-f", "src/y.rs", "docs/tracked.md", ".gitignore"],
            vec![
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&repo)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        repo
    }

    fn declared(repo: &std::path::Path, paths: &[&str]) -> BTreeSet<String> {
        paths
            .iter()
            .map(|path| repo.join(path).display().to_string())
            .collect()
    }

    /// Issue-26: a declared path the repository ignores is a project artifact
    /// the write layer retains; the audit is not asked to see it delivered. A
    /// tracked file under an ignored directory is still a deliverable.
    #[test]
    fn a_gitignored_declaration_is_not_a_repository_deliverable() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(temp.path());
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let root = repo.display().to_string();
        let paths = repository_paths(
            declared(&repo, &["docs/x.md", "src/y.rs", "docs/tracked.md"]),
            Some(&root),
            &store,
        )
        .unwrap();
        assert_eq!(
            paths,
            vec!["docs/tracked.md".to_string(), "src/y.rs".to_string()]
        );
    }

    /// Without a repository nothing is a repository deliverable, as before.
    #[test]
    fn without_a_repository_root_the_declaration_set_is_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(temp.path());
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let paths =
            repository_paths(declared(&repo, &["docs/x.md", "src/y.rs"]), None, &store).unwrap();
        assert!(paths.is_empty(), "{paths:?}");
    }
}

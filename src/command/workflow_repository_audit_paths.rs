//! Repository declarations use the existing root and artifact namespaces.
use archon_workflow::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};
use std::{collections::BTreeSet, path::Path};

pub(super) fn repository_paths(
    paths: BTreeSet<String>, root: Option<&str>, store: &WorkflowV2ResultStore,
) -> WorkflowResult<Vec<String>> {
    let context = archon_workflow::project_artifact_context_from_v2_root(store.root());
    let mut repository = BTreeSet::new();
    for raw in paths {
        let expanded = archon_workflow::v2::artifact_path_guard::expand_project_root_template(
            &raw, context.project_root.as_deref())
            .map_err(|error| WorkflowError::SpecInvalid(format!("audit declaration: {error}")))?;
        let path = Path::new(&expanded);
        let artifact = context.artifact_roots.iter().any(|artifact_root| {
            let artifact_root = Path::new(artifact_root);
            path.starts_with(artifact_root) || context.project_root.as_ref().is_some_and(|project|
                path.is_absolute() && path.starts_with(Path::new(project).join(artifact_root)))
        });
        if artifact { continue; }
        // Without a repository, deliverables belong to project verification;
        // the lifecycle still records its explicit no-repository assessment.
        let Some(root) = root else { continue; };
        let normalized = archon_workflow::v2::normalize_target_for_repository(
            "repository-audit", expanded.trim_end_matches('/'), Some(root))
            .map_err(|error| WorkflowError::SpecInvalid(format!("audit declaration: {error}")))?;
        archon_workflow::repository_audit::contract::validate_path(&normalized)
            .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
        repository.insert(normalized);
    }
    Ok(repository.into_iter().collect())
}

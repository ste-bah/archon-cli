fn templated_artifact_path(
    item_id: &str,
    raw: &str,
    project_root: &Path,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<ProjectArtifactPath, WorkflowV2WriteSafetyError> {
    let (relative, display) = if Path::new(raw).is_absolute() {
        let clean = clean_absolute_artifact_path(item_id, raw)?;
        let Ok(relative) = clean.strip_prefix(project_root) else {
            return Ok(ProjectArtifactPath::NotArtifact);
        };
        (
            normalize_relative_path(item_id, &relative.to_string_lossy())?,
            clean.display().to_string(),
        )
    } else {
        let relative = normalize_relative_path(item_id, raw)?;
        (relative.clone(), relative)
    };
    if !allowed_relative_artifact(&relative, context) {
        return Ok(ProjectArtifactPath::NotArtifact);
    }
    ensure_project_path_parent_safe(
        item_id,
        project_root,
        &project_root.join(&relative),
        &relative,
    )?;
    Ok(ProjectArtifactPath::Templated(display))
}

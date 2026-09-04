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

/// An unexpanded template placeholder is never satisfied evidence.
///
/// # Supersedes D76
///
/// D76 excluded a templated path from literal evidence checks and left the
/// result `Accepted`, on the reasoning that reporting it "missing" would
/// manufacture an unsatisfiable gap. The first half was right and is kept: a
/// path containing `<dataset-id>` is not a file, so it is still dropped from
/// `artifacts` and never checked literally. The second half is what prior-run
/// finding F4 (`wf-ee4a92fc`) caught — an artifact recorded as present against a
/// wildcard path, on "observed or contract-required" rather than on a file
/// anyone opened. Passing silently is the failure mode, not the safeguard.
///
/// The gap it raises is *not* unsatisfiable, which is why it is raised: the
/// remedy is to name the expanded instance path that was actually written. A
/// distinct id keeps it separable from `missing_project_artifact_*`, which
/// remains reserved for a concrete path that is genuinely absent.
fn note_templated_project_artifact(result: &mut WorkflowV2Result, path: &str) {
    let summary =
        format!("templated artifact requirement excluded from literal evidence checks: {path}");
    if !result.evidence.iter().any(|entry| entry.summary == summary) {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            summary,
        ));
    }
    let id = format!(
        "unexpanded_artifact_template_{}",
        artifact_id_for_path(path)
    );
    if !result.residual_gaps.iter().any(|gap| gap.id == id) {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id,
            description: format!(
                "declared artifact path {path} still carries unexpanded template placeholder(s); \
                 report the expanded instance path that was written, or bind the contract's \
                 instance fields so its instances can be enumerated"
            ),
            severity: Some("blocking".to_string()),
        });
    }
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        result.status = WorkflowV2Status::NeedsReview;
    }
}

fn note_missing_project_artifact(result: &mut WorkflowV2Result, path: &str, defect: &'static str) {
    let id = format!("missing_project_artifact_{}", artifact_id_for_path(path));
    if !result.residual_gaps.iter().any(|gap| gap.id == id) {
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id,
            description: format!("missing project artifact evidence at {path}: it {defect}"),
            severity: Some("blocking".to_string()),
        });
    }
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        result.status = WorkflowV2Status::NeedsReview;
    }
}

fn unsafe_target(item_id: &str, target: &str) -> WorkflowV2WriteSafetyError {
    WorkflowV2WriteSafetyError::UnsafeTarget {
        item_id: item_id.to_string(),
        target: target.to_string(),
    }
}

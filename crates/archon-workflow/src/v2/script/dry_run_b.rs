use super::*;
use crate::v2::artifact_path_guard::{ArtifactPathRejection, validate_declared_artifact_path};

pub(super) fn dry_run_stub_result(method: WorkflowV2HostMethod) -> String {
    // The stub must carry the same envelope keys the live result view exposes
    // ({status, summary, data, result, ...}): reference-following scripts read
    // `x.result`/`x.data` fields, and a stub without them throws in the
    // pre-flight rehearsal, falsely rejecting a script that runs fine live.
    serde_json::json!({
        "status": "accepted",
        "summary": format!("dry-run stub result for w.{}", method.as_str()),
        "items": [],
        "outcomes": [],
        "data": {},
        "result": { "status": "accepted", "summary": "dry-run stub", "data": {} },
        "dry_run": true,
    })
    .to_string()
}

/// Parse a script's `requiredArtifacts` option into a declared artifact
/// contract, refusing any entry that is not a path.
///
/// # Issue #168: the first place a sentence could become a directory
///
/// This accepted any non-empty string. `workflow.js` is agent-authored, so
/// `requiredArtifacts: task.acceptance_criteria` is one plausible line away,
/// and from here the value travels straight into the agent prompt under
/// "Resolved Project Artifact Paths ... write every file listed above". A
/// criterion containing `/` then reads as an instruction to create a nested
/// tree — the litter observed in run `wf-67dd2599`.
///
/// The whole call is refused rather than the offending entry dropped: a script
/// that declared a criterion as a deliverable has a defect its author needs to
/// see, and silently honouring the rest of the list hides it.
pub(super) fn artifact_requirements(
    value: &serde_json::Value,
) -> WorkflowResult<Vec<WorkflowV2ArtifactRequirement>> {
    let mut requirements = Vec::new();
    for entry in value.as_array().into_iter().flatten() {
        let (raw, kind) = match entry {
            serde_json::Value::String(path) => (path.trim(), None),
            serde_json::Value::Object(object) => {
                let Some(path) = object.get("path").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                (
                    path.trim(),
                    object
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                )
            }
            _ => continue,
        };
        if raw.is_empty() {
            continue;
        }
        // A surviving `${...}` / `<...>` is NOT refused here: this parse has no
        // project root to bind against, and the context-aware expansion in
        // `project_artifact_prompt::declared_project_artifacts` is the place
        // that can either expand it or name the variable nothing binds.
        // Everything else — prose, oversize, control characters — is refused
        // now, before the value can reach a prompt.
        let path = match validate_declared_artifact_path(raw) {
            Ok(path) => path,
            Err(ArtifactPathRejection::UnexpandedTemplate { .. }) => raw.to_string(),
            Err(rejection) => {
                return Err(WorkflowError::SpecInvalid(format!(
                    "workflow.js requiredArtifacts entry is not a path: {rejection}"
                )));
            }
        };
        let mut requirement = WorkflowV2ArtifactRequirement::new(path);
        requirement.kind = kind;
        requirements.push(requirement);
    }
    Ok(requirements)
}

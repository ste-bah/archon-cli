use std::path::Path;

use serde_json::json;

use crate::write_coordinator::patch_manifest::ManifestStatus;

const MAX_EMBEDDED_CHARS: usize = 24_000;

pub(super) fn coordinated_item_body(
    run_dir: &Path,
    stage_id: &str,
    item_id: &str,
    status: &ManifestStatus,
    fallback_target_files: &[String],
) -> String {
    let manifest_path = manifest_path(run_dir, stage_id, item_id);
    let agent_output_path = agent_output_path(run_dir, stage_id, item_id);
    let patch_path = patch_path(run_dir, stage_id, item_id);
    let mut body = format!(
        "# Coordinated Item `{item_id}`\n\nstatus: {}\n\n",
        status_label(status)
    );
    if let Some(reason) = status_reason(status) {
        body.push_str(&format!("reason: {reason}\n\n"));
    }
    if let Some(item) =
        remediation_item_json(run_dir, stage_id, item_id, status, fallback_target_files)
    {
        body.push_str("## Remediation Item\n\n");
        body.push_str("```json\n");
        body.push_str(&item);
        body.push_str("\n```\n\n");
    }
    body.push_str("## Evidence Paths\n\n");
    body.push_str(&format!("- manifest: `{}`\n", manifest_path.display()));
    body.push_str(&format!("- patch: `{}`\n", patch_path.display()));
    body.push_str(&format!(
        "- agent_output: `{}`\n\n",
        agent_output_path.display()
    ));
    append_file(&mut body, "Manifest", &manifest_path, "json");
    append_file(&mut body, "Agent Output", &agent_output_path, "json");
    append_file(&mut body, "Patch", &patch_path, "diff");
    body
}

fn remediation_item_json(
    run_dir: &Path,
    stage_id: &str,
    item_id: &str,
    status: &ManifestStatus,
    fallback_target_files: &[String],
) -> Option<String> {
    let reason = status_reason(status)?;
    let mut target_files = manifest_target_files(&manifest_path(run_dir, stage_id, item_id));
    if target_files.is_empty() {
        target_files = fallback_target_files.to_vec();
    }
    if target_files.is_empty() {
        return None;
    }
    serde_json::to_string_pretty(&json!({
        "items": [{
            "finding_id": format!("{stage_id}:{item_id}:write_coordination"),
            "related_task_id": item_id,
            "target_files": target_files,
            "failure": reason,
            "required_fix": format!("Repair the write-coordination failure for `{item_id}` and rerun the focused verification for the changed target files."),
            "required_tests": [
                "Rerun the focused verification command reported by the failed item after applying the fix."
            ],
        }]
    }))
    .ok()
}

fn manifest_target_files(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    value
        .get("declared_target_files")
        .or_else(|| value.get("changed_files"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
}

fn status_label(status: &ManifestStatus) -> &'static str {
    match status {
        ManifestStatus::Applied | ManifestStatus::IdempotentNoop => "accepted",
        ManifestStatus::Failed { .. } | ManifestStatus::Conflicted => "failed",
        ManifestStatus::PendingApply => "pending_apply",
    }
}

fn status_reason(status: &ManifestStatus) -> Option<&str> {
    match status {
        ManifestStatus::Failed { reason } => Some(reason.as_str()),
        ManifestStatus::Conflicted => Some("patch conflicted while applying to canonical tree"),
        ManifestStatus::PendingApply => Some("patch captured but not applied"),
        _ => None,
    }
}

fn append_file(body: &mut String, title: &str, path: &Path, fence: &str) {
    body.push_str(&format!("\n## {title}\n\n"));
    match std::fs::read_to_string(path) {
        Ok(text) => {
            body.push_str(&format!("```{fence}\n"));
            body.push_str(&truncate(&text));
            body.push_str("\n```\n");
        }
        Err(err) => body.push_str(&format!("unavailable: {err}\n")),
    }
}

fn truncate(text: &str) -> String {
    let mut out = text.chars().take(MAX_EMBEDDED_CHARS).collect::<String>();
    if text.chars().count() > MAX_EMBEDDED_CHARS {
        out.push_str("\n...truncated...");
    }
    out
}

fn manifest_path(run_dir: &Path, stage_id: &str, item_id: &str) -> std::path::PathBuf {
    run_dir
        .join("write-coordination/stages")
        .join(stage_id)
        .join("manifests")
        .join(format!("{item_id}.json"))
}

fn patch_path(run_dir: &Path, stage_id: &str, item_id: &str) -> std::path::PathBuf {
    run_dir
        .join("write-coordination/stages")
        .join(stage_id)
        .join("patches")
        .join(format!("{item_id}.patch"))
}

fn agent_output_path(run_dir: &Path, stage_id: &str, item_id: &str) -> std::path::PathBuf {
    run_dir
        .join("agent-outputs")
        .join(stage_id)
        .join(format!("{item_id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_coordinated_item_embeds_parseable_remediation_item() {
        let temp = tempfile::tempdir().unwrap();
        let manifest_dir = temp.path().join("write-coordination/stages/impl/manifests");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        std::fs::write(
            manifest_dir.join("impl-1.json"),
            r#"{"declared_target_files":["src/lib.rs"],"changed_files":["src/lib.rs"]}"#,
        )
        .unwrap();

        let body = coordinated_item_body(
            temp.path(),
            "impl",
            "impl-1",
            &ManifestStatus::Failed {
                reason: "verification blocked after patch".into(),
            },
            &[],
        );
        let items = crate::remediation_items::items_from_text(&body)
            .expect("failed coordinated item should expose remediation items");

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]["target_files"].as_array().unwrap()[0].as_str(),
            Some("src/lib.rs")
        );
        assert_eq!(
            items[0]["failure"].as_str(),
            Some("verification blocked after patch")
        );
    }

    #[test]
    fn failed_coordinated_item_uses_fallback_targets_without_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let body = coordinated_item_body(
            temp.path(),
            "impl",
            "impl-1",
            &ManifestStatus::Failed {
                reason: "wave aborted before manifest".into(),
            },
            &["src/lib.rs".to_string()],
        );
        let items = crate::remediation_items::items_from_text(&body)
            .expect("failed coordinated item should expose fallback remediation items");

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]["target_files"].as_array().unwrap()[0].as_str(),
            Some("src/lib.rs")
        );
    }
}

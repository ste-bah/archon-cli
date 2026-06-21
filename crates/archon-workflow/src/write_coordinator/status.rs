//! TASK-WC-008 — write-coordination status (§17 compact block).

use crate::error::WorkflowResult;
use crate::store::WorkflowStore;
use crate::write_coordinator::patch_manifest::{ManifestStatus, PatchManifest};

/// Renderable status for an active (or fallen-back) coordinated stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCoordinationStatus {
    pub enabled: bool,
    pub stage_id: String,
    pub wave_index: usize,
    pub wave_total: usize,
    pub width: usize,
    pub items_running: usize,
    pub items_failed: usize,
    pub items_accepted: usize,
    pub apply_state: String,
    pub fallback_reason: Option<String>,
    pub failed_item: Option<String>,
    pub failure_reason: Option<String>,
    pub failed_worktree: Option<String>,
    pub manifest_path: Option<String>,
    pub verify_command: Option<String>,
    pub verify_duration_ms: Option<u64>,
}

/// Read persisted coordinator artifacts into a renderable status. Returns None
/// when the stage has no write-coordination state on disk.
pub fn read_status(
    store: &WorkflowStore,
    run_id: &str,
    stage_id: &str,
) -> WorkflowResult<Option<WriteCoordinationStatus>> {
    let stage_root = store
        .run_dir(run_id)
        .join("write-coordination")
        .join("stages")
        .join(stage_id);
    if !stage_root.exists() {
        return Ok(None);
    }
    let apply_dir = stage_root.join("apply");
    let manifest_dir = stage_root.join("manifests");
    let tests_dir = stage_root.join("tests");
    let mut wave_total = 0usize;
    let mut accepted = 0usize;
    let mut failed = 0usize;
    let mut pending = 0usize;
    let mut last_apply = "pending".to_string();
    let mut latest_wave_id = None::<u64>;
    let mut latest_width = 0usize;
    let mut failed_item = None;
    let mut failure_reason = None;
    let mut manifest_path = None;
    let mut verify_command = None;
    let mut verify_duration_ms = None;
    if let Ok(entries) = std::fs::read_dir(&manifest_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(manifest) = serde_json::from_str::<PatchManifest>(&text)
            {
                match &manifest.status {
                    ManifestStatus::Applied | ManifestStatus::IdempotentNoop => accepted += 1,
                    ManifestStatus::Failed { reason } => {
                        failed += 1;
                        failed_item = Some(manifest.item_id.clone());
                        failure_reason = Some(reason.clone());
                        manifest_path = Some(entry.path().display().to_string());
                    }
                    ManifestStatus::PendingApply | ManifestStatus::Conflicted => pending += 1,
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(&apply_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            wave_total += 1;
            if let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            {
                let width =
                    count_array(&value, "items_applied") + count_array(&value, "items_failed");
                let wave_id = value
                    .get("wave_id")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(wave_total as u64);
                if latest_wave_id.is_none_or(|latest| wave_id >= latest) {
                    latest_wave_id = Some(wave_id);
                    latest_width = width;
                }
            }
        }
    }
    if let Some((command, duration_ms)) = latest_verify_result(&tests_dir) {
        verify_command = command;
        verify_duration_ms = duration_ms;
    }
    if failed > 0 {
        last_apply = "failed".into();
    } else if accepted > 0 && pending == 0 {
        last_apply = "applied".into();
    }
    if wave_total == 0 && accepted + failed + pending > 0 {
        wave_total = 1;
    }
    let failed_worktree = failed_item.as_ref().map(|item| {
        store
            .run_dir(run_id)
            .join("wc")
            .join("worktrees")
            .join(stage_id)
            .join(item)
            .display()
            .to_string()
    });
    Ok(Some(WriteCoordinationStatus {
        enabled: true,
        stage_id: stage_id.to_string(),
        wave_index: wave_total,
        wave_total,
        width: latest_width.max(1),
        items_running: 0,
        items_failed: failed,
        items_accepted: accepted,
        apply_state: last_apply,
        fallback_reason: None,
        failed_item,
        failure_reason,
        failed_worktree,
        manifest_path,
        verify_command,
        verify_duration_ms,
    }))
}

/// Stage ids that left coordinated write state on disk for this run.
pub fn coordinated_stage_ids(store: &WorkflowStore, run_id: &str) -> Vec<String> {
    let dir = store
        .run_dir(run_id)
        .join("write-coordination")
        .join("stages");
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                ids.push(name.to_string());
            }
        }
    }
    ids
}

fn count_array(value: &serde_json::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len)
}

/// §17 compact block: 6 lines for an active stage, 1 line for a fallback.
pub fn render_compact(status: &WriteCoordinationStatus) -> String {
    if let Some(reason) = &status.fallback_reason {
        return format!("write_coordination: serial_fallback ({reason})\n");
    }
    let mut out = format!(
        "write_coordination: enabled\n\
         stage: {}\n\
         wave: {}/{}\n\
         width: {}\n\
         items: {} running, {} failed, {} accepted\n\
         apply: {}\n",
        status.stage_id,
        status.wave_index,
        status.wave_total,
        status.width,
        status.items_running,
        status.items_failed,
        status.items_accepted,
        status.apply_state,
    );
    if let Some(item) = &status.failed_item {
        out.push_str(&format!("failed_item: {item}\n"));
    }
    if let Some(reason) = &status.failure_reason {
        out.push_str(&format!("failure: {}\n", one_line(reason)));
    }
    if let Some(path) = &status.manifest_path {
        out.push_str(&format!("manifest: {path}\n"));
    }
    if let Some(path) = &status.failed_worktree {
        out.push_str(&format!("worktree: {path}\n"));
    }
    if let Some(command) = &status.verify_command {
        let duration = status
            .verify_duration_ms
            .map(|ms| format!(" ({ms}ms)"))
            .unwrap_or_default();
        out.push_str(&format!("verify: {}{}\n", one_line(command), duration));
    }
    out
}

fn latest_verify_result(dir: &std::path::Path) -> Option<(Option<String>, Option<u64>)> {
    let mut latest = None::<(u64, Option<String>, Option<u64>)>;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let wave = entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u64>().ok())
            .unwrap_or(0);
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let command = value
            .get("command")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let duration = value.get("duration_ms").and_then(serde_json::Value::as_u64);
        if latest
            .as_ref()
            .is_none_or(|(latest_wave, _, _)| wave >= *latest_wave)
        {
            latest = Some((wave, command, duration));
        }
    }
    latest.map(|(_, command, duration)| (command, duration))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active() -> WriteCoordinationStatus {
        WriteCoordinationStatus {
            enabled: true,
            stage_id: "implement".into(),
            wave_index: 1,
            wave_total: 2,
            width: 2,
            items_running: 1,
            items_failed: 0,
            items_accepted: 1,
            apply_state: "applied".into(),
            fallback_reason: None,
            failed_item: None,
            failure_reason: None,
            failed_worktree: None,
            manifest_path: None,
            verify_command: None,
            verify_duration_ms: None,
        }
    }

    #[test]
    fn active_renders_six_lines() {
        let out = render_compact(&active());
        assert_eq!(out.lines().count(), 6, "got: {out}");
        assert!(out.starts_with("write_coordination: enabled\n"));
        assert!(out.contains("items: 1 running, 0 failed, 1 accepted"));
    }

    #[test]
    fn fallback_renders_one_line() {
        let mut s = active();
        s.fallback_reason = Some("boundary_unavailable".into());
        let out = render_compact(&s);
        assert_eq!(out.lines().count(), 1);
        assert_eq!(
            out,
            "write_coordination: serial_fallback (boundary_unavailable)\n"
        );
    }

    #[test]
    fn failed_item_renders_reason_and_paths() {
        let mut s = active();
        s.items_failed = 1;
        s.apply_state = "failed".into();
        s.failed_item = Some("implement-0".into());
        s.failure_reason = Some("verification blocked after patch: missing module".into());
        s.manifest_path = Some("/tmp/manifest.json".into());
        s.failed_worktree = Some("/tmp/worktree".into());
        let out = render_compact(&s);
        assert!(out.contains("apply: failed"));
        assert!(out.contains("failed_item: implement-0"));
        assert!(out.contains("verification blocked after patch"));
        assert!(out.contains("manifest: /tmp/manifest.json"));
        assert!(out.contains("worktree: /tmp/worktree"));
    }

    #[test]
    fn verify_command_renders_with_duration() {
        let mut s = active();
        s.verify_command = Some("cargo test -p archon-workflow status".into());
        s.verify_duration_ms = Some(1234);
        let out = render_compact(&s);

        assert!(out.contains("verify: cargo test -p archon-workflow status (1234ms)"));
    }

    #[test]
    fn latest_verify_result_skips_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("1.json"),
            r#"{"command":"cargo test","duration_ms":7}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("9.json"), "{not json").unwrap();

        let (command, duration) = latest_verify_result(dir.path()).unwrap();

        assert_eq!(command.as_deref(), Some("cargo test"));
        assert_eq!(duration, Some(7));
    }

    #[test]
    fn read_status_does_not_double_count_apply_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(dir.path());
        let stage = store
            .run_dir("run1")
            .join("write-coordination/stages/implement");
        std::fs::create_dir_all(stage.join("manifests")).unwrap();
        std::fs::create_dir_all(stage.join("apply")).unwrap();
        write_manifest(&stage, "item-0", r#"{"status":"failed","reason":"boom"}"#);
        write_manifest(&stage, "item-1", r#"{"status":"applied"}"#);
        std::fs::write(
            stage.join("apply/0.json"),
            r#"{"wave_id":0,"items_applied":[],"items_failed":[["item-0","boom"]]}"#,
        )
        .unwrap();
        std::fs::write(
            stage.join("apply/1.json"),
            r#"{"wave_id":1,"items_applied":["item-1"],"items_failed":[]}"#,
        )
        .unwrap();

        let status = read_status(&store, "run1", "implement").unwrap().unwrap();

        assert_eq!(status.items_failed, 1);
        assert_eq!(status.items_accepted, 1);
        assert_eq!(status.width, 1);
        assert_eq!(status.wave_total, 2);
        assert_eq!(status.apply_state, "failed");
    }

    fn write_manifest(stage: &std::path::Path, item_id: &str, status: &str) {
        let body = format!(
            r#"{{
              "schema":"archon.workflow.patch_manifest.v1",
              "run_id":"run1",
              "stage_id":"implement",
              "item_id":"{item_id}",
              "baseline_commit":"abc",
              "patch_path":"x.patch",
              "declared_target_files":["src/a.rs"],
              "changed_files":["src/a.rs"],
              "created_files":[],
              "deleted_files":[],
              "pre_hashes":{{}},
              "post_hashes":{{}},
              "verify_command":null,
              "agent_artifact_path":null,
              "status":{status}
            }}"#
        );
        std::fs::write(
            stage.join("manifests").join(format!("{item_id}.json")),
            body,
        )
        .unwrap();
    }
}

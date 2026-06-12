//! TASK-WC-008 — write-coordination status (§17 compact block).

use crate::error::WorkflowResult;
use crate::store::WorkflowStore;

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
    let apply_dir = stage_root.join("apply");
    if !apply_dir.exists() {
        return Ok(None);
    }
    let mut wave_total = 0usize;
    let mut accepted = 0usize;
    let mut failed = 0usize;
    let mut last_apply = "pending".to_string();
    if let Ok(entries) = std::fs::read_dir(&apply_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            wave_total += 1;
            if let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            {
                accepted += count_array(&value, "items_applied");
                failed += count_array(&value, "items_failed");
                last_apply = if failed > 0 { "failed" } else { "applied" }.to_string();
            }
        }
    }
    Ok(Some(WriteCoordinationStatus {
        enabled: true,
        stage_id: stage_id.to_string(),
        wave_index: wave_total,
        wave_total,
        width: (accepted + failed).max(1),
        items_running: 0,
        items_failed: failed,
        items_accepted: accepted,
        apply_state: last_apply,
        fallback_reason: None,
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
    format!(
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
    )
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
}

//! The PRD requirement-id inventory a run froze (Issue 387, item 4).
//!
//! The launch freezes the PRD digest and its acceptance criteria, but every
//! resume derives the requirement ids (and texts) from the PRD again with
//! this build's parser. A parser change could add or drop an id with nothing
//! saying so. The launch therefore records the id inventory and its digest
//! beside the run's arguments; a resume derives it again and names every id
//! a changed parser added or removed, in the operator log, as a
//! `BinaryRevisionDrift` event and on the resume's output. The record stays
//! as frozen, so each resume compares against the launch.
//!
//! A run launched before this record existed has none: its first resume
//! records the inventory it derives, as its baseline, and reports nothing.
//! The ids' texts are compared elsewhere (#381); only the set is here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::{WorkflowEventKind, WorkflowEventLog, WorkflowStore};

pub(crate) const INVENTORY_PATH: &str = "decomposition/requirement-inventory.json";
const INVENTORY_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequirementInventoryV1 {
    pub(crate) schema_version: u32,
    /// The PRD digest the ids were derived from.
    pub(crate) prd_digest: String,
    pub(crate) ids: BTreeSet<String>,
    /// `content_digest` of the ids, one per line, in order.
    pub(crate) digest: String,
    /// `launch`, or `first-resume` for a run launched before the record.
    pub(crate) recorded_at: String,
}

impl RequirementInventoryV1 {
    fn of(prd_digest: &str, texts: &BTreeMap<String, String>, recorded_at: &str) -> Self {
        let ids: BTreeSet<String> = texts.keys().cloned().collect();
        Self {
            schema_version: INVENTORY_SCHEMA,
            prd_digest: prd_digest.to_string(),
            digest: inventory_digest(&ids),
            ids,
            recorded_at: recorded_at.to_string(),
        }
    }
}

fn inventory_digest(ids: &BTreeSet<String>) -> String {
    let joined = ids.iter().cloned().collect::<Vec<_>>().join("\n");
    archon_workflow::task_set_contract::content_digest(joined.as_bytes())
}

/// Record the inventory the launch derived from `prd_text`.
pub(crate) fn record_launch(
    store: &WorkflowStore,
    run_id: &str,
    prd_digest: &str,
    prd_text: &str,
) -> Result<()> {
    let texts = archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    let record = RequirementInventoryV1::of(prd_digest, &texts, "launch");
    store.write_run_json(run_id, INVENTORY_PATH, &record)?;
    Ok(())
}

/// How the inventory a resume derives differs from the frozen one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InventoryChange {
    pub(crate) added: Vec<String>,
    pub(crate) removed: Vec<String>,
    pub(crate) frozen: RequirementInventoryV1,
    pub(crate) current_digest: String,
}

impl InventoryChange {
    pub(crate) fn line(&self) -> String {
        let list = |ids: &[String]| {
            if ids.is_empty() {
                "none".to_string()
            } else {
                ids.join(",")
            }
        };
        format!(
            "requirement_inventory_changed added={} removed={} frozen_digest={} current_digest={} frozen_at={}",
            list(&self.added),
            list(&self.removed),
            self.frozen.digest,
            self.current_digest,
            self.frozen.recorded_at
        )
    }
}

/// Compare the inventory derived now with the frozen one. Absent, it is
/// recorded now (unless `read_only`) and nothing is reported.
pub(crate) fn compare(
    store: &WorkflowStore,
    run_id: &str,
    prd_digest: &str,
    texts: &BTreeMap<String, String>,
    read_only: bool,
) -> Result<Option<InventoryChange>> {
    let path = store.run_dir(run_id).join(INVENTORY_PATH);
    let current = RequirementInventoryV1::of(prd_digest, texts, "first-resume");
    let frozen: RequirementInventoryV1 = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing requirement inventory {}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if !read_only {
                store.write_run_json(run_id, INVENTORY_PATH, &current)?;
            }
            return Ok(None);
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if frozen.schema_version != INVENTORY_SCHEMA {
        return Err(anyhow!(
            "requirement inventory {} has schema {}; this build reads schema {INVENTORY_SCHEMA}",
            path.display(),
            frozen.schema_version
        ));
    }
    if frozen.ids == current.ids {
        return Ok(None);
    }
    Ok(Some(InventoryChange {
        added: current.ids.difference(&frozen.ids).cloned().collect(),
        removed: frozen.ids.difference(&current.ids).cloned().collect(),
        current_digest: current.digest,
        frozen,
    }))
}

/// [`compare`] on a live resume, naming a change in the operator log at
/// `log_path` and as a `BinaryRevisionDrift` event. Returns the lines the
/// resume prints.
pub(crate) fn check_on_resume(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &Path,
    prd_digest: &str,
    texts: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    let Some(change) = compare(store, run_id, prd_digest, texts, false)? else {
        return Ok(Vec::new());
    };
    let line = change.line();
    crate::command::workflow_decompose_log::append_nofollow_line(log_path, &line)?;
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        WorkflowEventKind::BinaryRevisionDrift,
        serde_json::json!({
            "event": "requirement_inventory_changed",
            "added": change.added,
            "removed": change.removed,
            "frozen_digest": change.frozen.digest,
            "current_digest": change.current_digest,
            "frozen_at": change.frozen.recorded_at,
        }),
    )?;
    Ok(vec![line])
}

#[cfg(test)]
#[path = "workflow_decompose_requirement_inventory_tests.rs"]
mod tests;

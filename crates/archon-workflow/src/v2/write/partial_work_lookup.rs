//! Where a captured partial can be found, and how it survives a rewrite.
//!
//! Issue-18: the lookup used to read only the CURRENT branch-outcome record
//! per item. A resume pass that re-records the item — the dependency gate's
//! "was not dispatched" placeholder, a re-dispatched wave — moves the record
//! that carried `partial_work` into `superseded/` and the partial vanishes
//! from view while its patch still sits under `write-coordination/stages/`.
//! Live: `agents-5-0` on wf-719ff3b0, 22 files and 3h20m of work, re-dispatched
//! onto a clean worktree with no resume preamble.
//!
//! Three sources are read, newest capture wins, one entry per patch file:
//! 1. the partial directory itself, through the `<item>.partial.json` sidecar
//!    written beside every patch at capture time (authoritative: its file
//!    list is the one captured with that patch);
//! 2. the current outcome record per item;
//! 3. every superseded outcome record.
//!
//! A patch with no sidecar (captured by an older binary) is resolved through
//! whichever record names it, current or superseded.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::partial_work::{DATA_KEY, PartialWork};
use crate::v2::{WorkflowV2BranchOutcome, WorkflowV2Result, WorkflowV2Status};
use crate::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};

pub(crate) const SIDECAR_SCHEMA_VERSION: u32 = 1;
const SIDECAR_SUFFIX: &str = ".partial.json";

/// The metadata that makes a patch file self-describing: which tasks it is
/// for, when it was taken, and what it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PartialSidecar {
    pub schema_version: u32,
    pub stage_id: String,
    pub branch_id: String,
    pub canonical_task_ids: Vec<String>,
    /// RFC 3339, UTC.
    pub captured_at: String,
    #[serde(flatten)]
    pub partial: PartialWork,
}

pub(crate) fn sidecar_path(patch_path: &Path) -> PathBuf {
    let stem = patch_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("partial");
    patch_path.with_file_name(format!("{stem}{SIDECAR_SUFFIX}"))
}

pub(crate) fn write_sidecar(
    stage_id: &str,
    branch_id: &str,
    task_ids: &[String],
    partial: &PartialWork,
) -> WorkflowResult<()> {
    let sidecar = PartialSidecar {
        schema_version: SIDECAR_SCHEMA_VERSION,
        stage_id: stage_id.to_string(),
        branch_id: branch_id.to_string(),
        canonical_task_ids: task_ids.to_vec(),
        captured_at: chrono::Utc::now().to_rfc3339(),
        partial: partial.clone(),
    };
    let path = sidecar_path(&partial.patch_path);
    let bytes = serde_json::to_vec_pretty(&sidecar)?;
    std::fs::write(&path, bytes).map_err(|error| WorkflowError::io(&path, error))
}

/// The canonical task ids a branch result names.
pub(crate) fn task_ids_of(result: &WorkflowV2Result) -> Vec<String> {
    result
        .data
        .get("canonical_task_ids")
        .and_then(|ids| ids.as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The partial an outcome record carries, with the tasks it is for. A partial
/// recorded by an older binary has no origin of its own; the record it sits
/// on IS the verdict on that attempt, so the origin is read from there
/// (Issue-20).
pub(crate) fn partial_from_outcome(
    outcome: &WorkflowV2BranchOutcome,
) -> Option<(Vec<String>, PartialWork)> {
    let result = outcome.result.as_ref()?;
    let mut partial = result
        .data
        .get(DATA_KEY)
        .and_then(|value| serde_json::from_value::<PartialWork>(value.clone()).ok())?;
    if partial.origin.is_none() {
        partial.origin = Some(super::partial_work::PartialOrigin::from_result(result));
    }
    Some((task_ids_of(result), partial))
}

struct Candidate {
    task_ids: Vec<String>,
    captured: SystemTime,
    partial: PartialWork,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Every `<stage>/partial/*.partial.json` under the run's write coordination
/// directory, keyed by the patch it describes.
fn sidecar_candidates(run_root: &Path, into: &mut BTreeMap<PathBuf, Candidate>) {
    let stages = run_root.join("write-coordination").join("stages");
    let Ok(stage_dirs) = std::fs::read_dir(&stages) else {
        return;
    };
    for stage in stage_dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(stage.path().join("partial")) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_sidecar = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(SIDECAR_SUFFIX));
            if !is_sidecar {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(sidecar) = serde_json::from_str::<PartialSidecar>(&raw) else {
                continue;
            };
            // Captured without resolvable task ids: let a record that names
            // this patch resolve it instead of shadowing that record.
            if sidecar.canonical_task_ids.is_empty() {
                continue;
            }
            let captured = chrono::DateTime::parse_from_rfc3339(&sidecar.captured_at)
                .ok()
                .map(SystemTime::from)
                .or_else(|| modified(&sidecar.partial.patch_path));
            let Some(captured) = captured else {
                continue;
            };
            into.insert(
                sidecar.partial.patch_path.clone(),
                Candidate {
                    task_ids: sidecar.canonical_task_ids,
                    captured,
                    partial: sidecar.partial,
                },
            );
        }
    }
}

/// Outcome records, current and superseded, for patches no sidecar describes.
///
/// A sidecar keeps its own task ids and file list, but one an older binary
/// wrote has no origin, and that sidecar shadowed the record's derived one:
/// the live rejected branch's 07:38 sidecar would still have resumed as "ran
/// out of time" (Issue-20). So a sidecar candidate without an origin takes
/// the origin of the first record naming its patch, and nothing else.
fn record_candidates(v2_store: &WorkflowV2ResultStore, into: &mut BTreeMap<PathBuf, Candidate>) {
    let current = v2_store.load_branch_outcomes().unwrap_or_default();
    let superseded = v2_store.load_superseded_branch_outcomes();
    for outcome in current.iter().chain(superseded.iter()) {
        let Some((task_ids, partial)) = partial_from_outcome(outcome) else {
            continue;
        };
        if let Some(existing) = into.get_mut(&partial.patch_path) {
            if existing.partial.origin.is_none() {
                existing.partial.origin = partial.origin;
            }
            continue;
        }
        let Some(captured) = modified(&partial.patch_path) else {
            continue;
        };
        into.insert(
            partial.patch_path.clone(),
            Candidate {
                task_ids,
                captured,
                partial,
            },
        );
    }
}

/// The most recent partial patch any earlier branch in this run left for one
/// of the given canonical task ids, from every source named in the module
/// docs. A patch whose file is gone is not a candidate.
pub(crate) fn latest_partial_for_tasks(
    v2_store: &WorkflowV2ResultStore,
    task_ids: &[String],
) -> Option<PartialWork> {
    let run_root = v2_store
        .root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| v2_store.root().to_path_buf());
    let mut candidates = BTreeMap::new();
    sidecar_candidates(&run_root, &mut candidates);
    record_candidates(v2_store, &mut candidates);
    candidates
        .into_values()
        .filter(|candidate| candidate.partial.patch_path.is_file())
        .filter(|candidate| {
            candidate
                .task_ids
                .iter()
                .any(|id| task_ids.iter().any(|t| t == id))
        })
        .max_by_key(|candidate| candidate.captured)
        .map(|candidate| candidate.partial)
}

/// Issue-18, the other half: an outcome rewritten for the same item must not
/// drop the partial the record it replaces carried. The dependency gate's
/// "was not dispatched" placeholder and a re-dispatch that ends without a
/// capture both describe an attempt that produced NOTHING new for the task,
/// so the previous attempt's patch is still the best resume point and the
/// current record must keep naming it. Not carried when the new result is
/// accepted (the work landed), when the task has landed through any other
/// outcome, or when the patch file is gone.
pub(crate) fn carry_forward_partial_work(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    result: &mut WorkflowV2Result,
) {
    if result.data.get(DATA_KEY).is_some()
        || matches!(
            result.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
    {
        return;
    }
    let Ok(Some(previous)) = v2_store.load_branch_outcome(call_id, item_id) else {
        return;
    };
    let Some((task_ids, partial)) = partial_from_outcome(&previous) else {
        return;
    };
    if !partial.patch_path.is_file() {
        return;
    }
    let landed = super::dependency_gate::landed_task_ids(
        &v2_store.load_branch_outcomes().unwrap_or_default(),
    );
    if task_ids.iter().any(|id| landed.contains(id)) {
        return;
    }
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    result.data[DATA_KEY] = serde_json::to_value(&partial).unwrap_or_default();
    result.evidence.push(crate::v2::WorkflowV2Evidence::new(
        crate::v2::WorkflowV2EvidenceKind::Review,
        format!(
            "partial work carried forward from the outcome this record replaces: {} file(s), {} bytes, applied to the next attempt at this task",
            partial.files.len(),
            partial.bytes
        ),
    ));
}

#[cfg(test)]
#[path = "partial_work_lookup_tests.rs"]
mod tests;

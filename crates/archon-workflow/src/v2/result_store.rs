use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::events::sanitize_value;
use crate::{WorkflowError, WorkflowResult};

use super::{WorkflowV2BranchOutcome, WorkflowV2HostCall, WorkflowV2Result, WorkflowV2Status};

const RESULT_SCHEMA_VERSION: &str = "workflow-result-v2";
static SUPERSEDED_ARCHIVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct WorkflowV2ResultStore {
    root: PathBuf,
    /// What this store instance (and its clones -- one run's session)
    /// recorded or replayed; see `result_store_session.rs`.
    session: std::sync::Arc<session::SessionLedger>,
}

#[path = "result_store_session.rs"]
mod session;

impl WorkflowV2ResultStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            session: Default::default(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The RUN's own directory — the parent of this store when the store is
    /// the run's `v2` subdirectory, and the store root itself otherwise.
    ///
    /// The store holds one engine's records inside a run directory that also
    /// holds the run's state, its event log and its artifact area. Callers
    /// that must talk about the whole run — the write boundary that keeps an
    /// agent out of the host's bookkeeping, for one — need that directory and
    /// not this one, and derive it here rather than each re-deriving the
    /// `v2` special case that [`Self::run_id`] already encodes.
    pub fn run_root(&self) -> &Path {
        if self.root.file_name().and_then(|name| name.to_str()) == Some("v2")
            && let Some(parent) = self.root.parent()
        {
            return parent;
        }
        &self.root
    }

    /// The directory every run of this project is kept in — the parent of
    /// [`Self::run_root`], since a run directory is the store root joined with
    /// the run id (`WorkflowStore::run_dir`).
    ///
    /// Stated here because this crate is the one that builds that layout. A
    /// consumer that needs "every run, not just this one" — the boundary that
    /// keeps an agent's walks out of accumulated history — must not guess it
    /// from a path it was handed.
    pub fn run_store_root(&self) -> Option<&Path> {
        self.run_root().parent()
    }

    pub fn run_id(&self) -> String {
        if self.root.file_name().and_then(|name| name.to_str()) == Some("v2") {
            return self
                .root
                .parent()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("unknown-run")
                .to_string();
        }
        self.root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown-run")
            .to_string()
    }

    pub fn result_path(&self, call_id: &str) -> PathBuf {
        let hash = blake3::hash(call_id.as_bytes()).to_hex().to_string();
        self.root
            .join("results")
            .join(format!("{}-{}.json", sanitize_call_id(call_id), hash))
    }

    pub fn checkpoint_path(&self) -> PathBuf {
        self.root.join("checkpoint.json")
    }

    pub fn branch_outcome_path(&self, call_id: &str, item_id: &str) -> PathBuf {
        self.root
            .join("branches")
            .join(sanitize_call_id(call_id))
            .join(format!("{}.json", sanitize_call_id(item_id)))
    }

    pub fn rejected_output_path(&self, branch_id: &str) -> PathBuf {
        self.root
            .join("rejected-outputs")
            .join(format!("{}.json", sanitize_call_id(branch_id)))
    }

    pub fn append_rejected_output(
        &self,
        branch_id: &str,
        record: WorkflowV2RejectedOutput,
    ) -> WorkflowResult<PathBuf> {
        let path = self.rejected_output_path(branch_id);
        let mut log = load_rejected_output_log(&path)?;
        log.branch_id = branch_id.to_string();
        log.rejections.push(record);
        write_json(&path, &log)?;
        Ok(path)
    }

    pub fn save_call_record(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<()> {
        let path = self.result_path(&record.call.id);
        let mut clean = sanitize_for_persistence(record)?;
        clean.output_hash = stable_result_hash(&clean.result);
        archive_superseded_json(&path, |existing: &WorkflowV2CallRecord| {
            existing.input_hash == clean.input_hash && existing.attempt == clean.attempt
        })?;
        self.note_prior_finish(&path, &record.call.id);
        self.note_session_call(&record.call.id);
        write_json(&path, &clean)
    }

    pub fn save_branch_outcome(
        &self,
        call_id: &str,
        outcome: &WorkflowV2BranchOutcome,
    ) -> WorkflowResult<PathBuf> {
        let path = self.branch_outcome_path(call_id, &outcome.item_id);
        let clean = sanitize_for_persistence(outcome)?;
        archive_superseded_json(&path, |existing: &WorkflowV2BranchOutcome| {
            match (&existing.item_input_hash, &clean.item_input_hash) {
                (Some(old), Some(new)) => old == new,
                // Missing identity on either side: treat as an in-place update
                // of the same execution, never a supersede.
                _ => true,
            }
        })?;
        write_json(&path, &clean)?;
        Ok(path)
    }

    pub fn load_branch_outcome(
        &self,
        call_id: &str,
        item_id: &str,
    ) -> WorkflowResult<Option<WorkflowV2BranchOutcome>> {
        let path = self.branch_outcome_path(call_id, item_id);
        if !path.exists() {
            return Ok(None);
        }
        read_store_record(&path)
    }

    /// The current outcome of every branch one call fanned out, sorted by
    /// item id. Empty, not an error, for a call that saved no branches.
    /// Obs-22: the review reduce's roster is built from these records, so a
    /// branch that ran and reported nothing is still on the roster.
    pub fn load_branch_outcomes_for_call(
        &self,
        call_id: &str,
    ) -> WorkflowResult<Vec<WorkflowV2BranchOutcome>> {
        let dir = self.root.join("branches").join(sanitize_call_id(call_id));
        let mut outcomes = Vec::new();
        if dir.is_dir() {
            load_outcomes_from_dir(&dir, &mut outcomes)?;
        }
        outcomes.sort_by(|left, right| left.item_id.cmp(&right.item_id));
        Ok(outcomes)
    }

    pub fn load_branch_outcomes(&self) -> WorkflowResult<Vec<WorkflowV2BranchOutcome>> {
        let root = self.root.join("branches");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut outcomes = Vec::new();
        for call_dir in fs::read_dir(&root).map_err(|err| WorkflowError::io(&root, err))? {
            let call_dir = call_dir.map_err(|err| WorkflowError::io(&root, err))?;
            if !call_dir
                .file_type()
                .map_err(|err| WorkflowError::io(call_dir.path(), err))?
                .is_dir()
            {
                continue;
            }
            load_outcomes_from_dir(&call_dir.path(), &mut outcomes)?;
        }
        outcomes.sort_by(|left, right| left.item_id.cmp(&right.item_id));
        Ok(outcomes)
    }

    /// Every branch outcome a later record replaced, from each call's
    /// `superseded/` directory. `load_branch_outcomes` is the current record
    /// per item; a partial captured by a replaced record (Issue-18) lives
    /// only here. An unreadable archived file is skipped, never fatal: the
    /// archive is history, and one bad row must not hide the rest.
    pub fn load_superseded_branch_outcomes(&self) -> Vec<WorkflowV2BranchOutcome> {
        let root = self.root.join("branches");
        let Ok(calls) = fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut outcomes = Vec::new();
        for call_dir in calls.flatten() {
            let Ok(entries) = fs::read_dir(call_dir.path().join("superseded")) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(raw) = fs::read_to_string(&path)
                    && let Ok(outcome) = serde_json::from_str(&raw)
                {
                    outcomes.push(outcome);
                }
            }
        }
        outcomes
    }

    pub fn delete_branch_outcome(&self, call_id: &str, item_id: &str) -> WorkflowResult<bool> {
        let path = self.branch_outcome_path(call_id, item_id);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path).map_err(|err| WorkflowError::io(&path, err))?;
        Ok(true)
    }

    pub fn delete_branch_outcomes_for_call(&self, call_id: &str) -> WorkflowResult<usize> {
        let dir = self.root.join("branches").join(sanitize_call_id(call_id));
        if !dir.exists() {
            return Ok(0);
        }
        let mut deleted = 0usize;
        for entry in fs::read_dir(&dir).map_err(|err| WorkflowError::io(&dir, err))? {
            let entry = entry.map_err(|err| WorkflowError::io(&dir, err))?;
            if entry
                .file_type()
                .map_err(|err| WorkflowError::io(entry.path(), err))?
                .is_file()
            {
                deleted += 1;
            }
        }
        fs::remove_dir_all(&dir).map_err(|err| WorkflowError::io(&dir, err))?;
        Ok(deleted)
    }

    pub fn load_call_record(&self, call_id: &str) -> WorkflowResult<Option<WorkflowV2CallRecord>> {
        let path = self.result_path(call_id);
        if !path.exists() {
            return Ok(None);
        }
        read_store_record(&path)
    }

    pub fn load_call_records(&self) -> WorkflowResult<Vec<WorkflowV2CallRecord>> {
        let dir = self.root.join("results");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut records: Vec<WorkflowV2CallRecord> = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|err| WorkflowError::io(&dir, err))? {
            let entry = entry.map_err(|err| WorkflowError::io(&dir, err))?;
            if !entry
                .file_type()
                .map_err(|err| WorkflowError::io(entry.path(), err))?
                .is_file()
            {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if let Some(record) = read_store_record(&path)? {
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.call.id.cmp(&right.call.id));
        Ok(records)
    }

    pub fn save_checkpoint(&self, checkpoint: &WorkflowV2Checkpoint) -> WorkflowResult<()> {
        write_json(&self.checkpoint_path(), checkpoint)
    }

    pub fn load_checkpoint(&self) -> WorkflowResult<Option<WorkflowV2Checkpoint>> {
        let path = self.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
        serde_json::from_str(&raw).map(Some).map_err(Into::into)
    }

    pub fn invalidate_call_and_dependents(
        &self,
        executions: &[super::WorkflowV2CallExecution],
        call_id: &str,
    ) -> WorkflowResult<Vec<String>> {
        let invalidated = downstream_call_ids(executions, call_id);
        for id in &invalidated {
            if let Some(mut record) = self.load_call_record(id)? {
                record.invalidated_by = Some(call_id.to_string());
                self.save_call_record(&record)?;
            }
        }
        if let Some(mut checkpoint) = self.load_checkpoint()? {
            checkpoint.remove_completed(&invalidated);
            self.save_checkpoint(&checkpoint)?;
        }
        Ok(invalidated.into_iter().collect())
    }

    pub fn invalidate_dynamic_wave_dependents(&self, call_id: &str) -> WorkflowResult<Vec<String>> {
        let records = self.load_call_records()?;
        let invalidated = dynamic_wave_invalidated_call_ids(&records, call_id);
        for id in &invalidated {
            if let Some(mut record) = self.load_call_record(id)? {
                record.invalidated_by = Some(call_id.to_string());
                self.save_call_record(&record)?;
            }
        }
        if let Some(mut checkpoint) = self.load_checkpoint()? {
            checkpoint.remove_completed(&invalidated);
            self.save_checkpoint(&checkpoint)?;
        }
        Ok(invalidated.into_iter().collect())
    }

    pub fn invalidate_task_and_dependents(
        &self,
        executions: &[super::WorkflowV2CallExecution],
        requested_task_id: &str,
        affected_task_ids: &BTreeSet<String>,
        reason: &str,
    ) -> WorkflowResult<WorkflowV2TaskInvalidation> {
        let records = self.load_call_records()?;
        let seed_call_ids = records
            .iter()
            .filter(|record| record_intersects_tasks(record, affected_task_ids))
            .map(|record| record.call.id.clone())
            .collect::<BTreeSet<_>>();
        let mut invalidated = BTreeSet::new();
        for call_id in &seed_call_ids {
            if executions.is_empty() {
                invalidated.insert(call_id.clone());
            } else {
                invalidated.extend(downstream_call_ids(executions, call_id));
            }
        }
        for record in &records {
            if invalidated.contains(&record.call.id) {
                continue;
            }
            if record_intersects_tasks(record, affected_task_ids) {
                invalidated.insert(record.call.id.clone());
            }
        }
        for id in &invalidated {
            if let Some(mut record) = self.load_call_record(id)? {
                record.invalidated_by = Some(reason.to_string());
                self.save_call_record(&record)?;
            }
        }
        if let Some(mut checkpoint) = self.load_checkpoint()? {
            checkpoint.remove_completed(&invalidated);
            self.save_checkpoint(&checkpoint)?;
        }
        let mut deleted_branch_outcomes = Vec::new();
        for outcome in self.load_branch_outcomes()? {
            let task_ids = branch_outcome_task_ids(&outcome);
            let affected = task_ids
                .iter()
                .any(|task_id| affected_task_ids.contains(task_id));
            if !affected {
                continue;
            }
            let call_id = outcome
                .completion_evidence
                .iter()
                .find(|evidence| !evidence.call_id.trim().is_empty())
                .map(|evidence| evidence.call_id.clone())
                .unwrap_or_default();
            if call_id.is_empty() {
                continue;
            }
            if self.delete_branch_outcome(&call_id, &outcome.item_id)? {
                deleted_branch_outcomes.push(WorkflowV2DeletedBranchOutcome {
                    call_id,
                    item_id: outcome.item_id,
                    task_ids: task_ids.into_iter().collect(),
                });
            }
        }
        Ok(WorkflowV2TaskInvalidation {
            requested_task_id: requested_task_id.to_string(),
            affected_task_ids: affected_task_ids.iter().cloned().collect(),
            invalidated_call_ids: invalidated.into_iter().collect(),
            deleted_branch_outcomes,
        })
    }
}

fn load_outcomes_from_dir(
    dir: &Path,
    outcomes: &mut Vec<WorkflowV2BranchOutcome>,
) -> WorkflowResult<()> {
    for entry in fs::read_dir(dir).map_err(|err| WorkflowError::io(dir, err))? {
        let entry = entry.map_err(|err| WorkflowError::io(dir, err))?;
        if !entry
            .file_type()
            .map_err(|err| WorkflowError::io(entry.path(), err))?
            .is_file()
        {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && let Some(outcome) = read_store_record(&path)?
        {
            outcomes.push(outcome);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowV2RejectedOutput {
    pub attempt: String,
    pub error: String,
    pub raw_body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkflowV2RejectedOutputLog {
    branch_id: String,
    rejections: Vec<WorkflowV2RejectedOutput>,
}

fn load_rejected_output_log(path: &Path) -> WorkflowResult<WorkflowV2RejectedOutputLog> {
    if !path.exists() {
        return Ok(WorkflowV2RejectedOutputLog {
            branch_id: String::new(),
            rejections: Vec::new(),
        });
    }
    let raw = fs::read_to_string(path).map_err(|err| WorkflowError::io(path, err))?;
    serde_json::from_str(&raw).map_err(Into::into)
}

/// D79: a call id re-executed by a later cycle (e.g. a terminal-gate reroute)
/// must never silently destroy the prior record — post-run adjudication is
/// built on this history. When a NEW execution claims an occupied slot, the
/// existing file moves into a `superseded/` sibling directory first; an
/// unreadable existing file is archived rather than clobbered.
fn archive_superseded_json<T: DeserializeOwned>(
    path: &Path,
    same_execution: impl FnOnce(&T) -> bool,
) -> WorkflowResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if let Ok(raw) = fs::read_to_string(path)
        && let Ok(existing) = serde_json::from_str::<T>(&raw)
        && same_execution(&existing)
    {
        return Ok(());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let dir = parent.join("superseded");
    fs::create_dir_all(&dir).map_err(|err| WorkflowError::io(&dir, err))?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("record");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let sequence = SUPERSEDED_ARCHIVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let target = dir.join(format!(
        "{stem}-{stamp}-{}-{sequence}.json",
        std::process::id()
    ));
    fs::rename(path, &target).map_err(|err| WorkflowError::io(&target, err))?;
    Ok(())
}

include!("result_store_records.rs");

include!("result_store_scan.rs");

include!("result_store_invalidation.rs");

include!("result_store_io.rs");

#[cfg(test)]
#[path = "result_store_tests.rs"]
mod tests;

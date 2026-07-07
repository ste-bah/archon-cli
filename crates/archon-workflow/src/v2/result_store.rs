use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::events::sanitize_value;
use crate::{WorkflowError, WorkflowResult};

use super::{WorkflowV2BranchOutcome, WorkflowV2HostCall, WorkflowV2Result, WorkflowV2Status};

const RESULT_SCHEMA_VERSION: &str = "workflow-result-v2";

#[derive(Debug, Clone)]
pub struct WorkflowV2ResultStore {
    root: PathBuf,
}

impl WorkflowV2ResultStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
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

    pub fn save_call_record(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<()> {
        let path = self.result_path(&record.call.id);
        let mut clean = sanitize_for_persistence(record)?;
        clean.output_hash = stable_result_hash(&clean.result);
        write_json(&path, &clean)
    }

    pub fn save_branch_outcome(
        &self,
        call_id: &str,
        outcome: &WorkflowV2BranchOutcome,
    ) -> WorkflowResult<PathBuf> {
        let path = self.branch_outcome_path(call_id, &outcome.item_id);
        let clean = sanitize_for_persistence(outcome)?;
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
        let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
        serde_json::from_str(&raw).map(Some).map_err(Into::into)
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
        let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
        serde_json::from_str(&raw).map(Some).map_err(Into::into)
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
            let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
            records.push(serde_json::from_str(&raw)?);
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
}

include!("result_store_records.rs");

include!("result_store_invalidation.rs");

include!("result_store_io.rs");

#[cfg(test)]
#[path = "result_store_tests.rs"]
mod tests;

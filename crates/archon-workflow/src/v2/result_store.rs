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
                .map_err(|err| WorkflowError::io(&entry.path(), err))?
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2CallRecord {
    #[serde(default)]
    pub run_id: String,
    pub call: WorkflowV2HostCall,
    pub attempt: u32,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub finished_at: String,
    pub input_hash: String,
    #[serde(default)]
    pub output_hash: String,
    pub status: WorkflowV2Status,
    pub result: WorkflowV2Result,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}

impl WorkflowV2CallRecord {
    pub fn new(
        run_id: impl Into<String>,
        call: WorkflowV2HostCall,
        attempt: u32,
        input_hash: String,
        result: WorkflowV2Result,
        depends_on: Vec<String>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            run_id: run_id.into(),
            call,
            attempt,
            schema_version: RESULT_SCHEMA_VERSION.to_string(),
            started_at: now.clone(),
            finished_at: now,
            input_hash,
            output_hash: stable_result_hash(&result),
            status: result.status,
            result,
            depends_on,
            invalidated_by: None,
            agent_session_id: None,
        }
    }

    pub fn is_reusable_for(&self, input_hash: &str) -> bool {
        self.input_hash == input_hash
            && self.invalidated_by.is_none()
            && matches!(
                self.status,
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop
            )
            && self.result.validate().is_ok()
    }
}

fn default_schema_version() -> String {
    RESULT_SCHEMA_VERSION.to_string()
}

fn stable_result_hash(result: &WorkflowV2Result) -> String {
    let bytes = serde_json::to_vec(result).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkflowV2Checkpoint {
    #[serde(default)]
    pub completed_call_ids: Vec<String>,
}

impl WorkflowV2Checkpoint {
    pub fn mark_completed(&mut self, call_id: &str) {
        if !self.completed_call_ids.iter().any(|id| id == call_id) {
            self.completed_call_ids.push(call_id.to_string());
        }
    }

    pub fn remove_completed_call(&mut self, call_id: &str) {
        self.completed_call_ids.retain(|id| id != call_id);
    }

    pub fn remove_completed(&mut self, call_ids: &BTreeSet<String>) {
        self.completed_call_ids
            .retain(|call_id| !call_ids.contains(call_id));
    }
}

fn downstream_call_ids(
    executions: &[super::WorkflowV2CallExecution],
    call_id: &str,
) -> BTreeSet<String> {
    let mut by_dependency: HashMap<&str, Vec<&str>> = HashMap::new();
    for execution in executions {
        for dependency in &execution.depends_on {
            by_dependency
                .entry(dependency.as_str())
                .or_default()
                .push(execution.call.id.as_str());
        }
    }

    let mut invalidated = BTreeSet::new();
    let mut queue = vec![call_id];
    while let Some(current) = queue.pop() {
        if !invalidated.insert(current.to_string()) {
            continue;
        }
        if let Some(children) = by_dependency.get(current) {
            queue.extend(children.iter().copied());
        }
    }
    invalidated
}

fn sanitize_call_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
    fs::rename(&tmp, path).map_err(|err| WorkflowError::io(path, err))
}

fn sanitize_for_persistence<T>(value: &T) -> WorkflowResult<T>
where
    T: Serialize + DeserializeOwned,
{
    let value = serde_json::to_value(value)?;
    serde_json::from_value(sanitize_value(value)).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::super::{
        WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostMethod, WorkflowV2HostOptions,
    };
    use super::*;

    fn call(id: &str) -> WorkflowV2HostCall {
        WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        }
    }

    #[test]
    fn call_records_are_sanitized_before_persistence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path());
        let mut result = WorkflowV2Result::accepted("done with token=supersecret");
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Other,
            "authorization: bearer-secret should not persist",
        ));
        result.data = serde_json::json!({
            "raw_text": "private provider payload",
            "nested": { "api_key": "secret", "safe": "visible" }
        });
        let record = WorkflowV2CallRecord::new(
            "wf-test",
            call("discover"),
            0,
            "input".to_string(),
            result,
            Vec::new(),
        );

        store.save_call_record(&record).expect("save record");
        let raw = std::fs::read_to_string(store.result_path("discover")).expect("persisted record");

        assert!(!raw.contains("supersecret"));
        assert!(!raw.contains("authorization"));
        assert!(!raw.contains("raw_text"));
        assert!(!raw.contains("api_key"));
        assert!(raw.contains("visible"));
    }

    #[test]
    fn branch_outcomes_are_sanitized_before_persistence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path());
        let mut result = WorkflowV2Result::accepted("done");
        result.data = serde_json::json!({ "access_token": "do-not-store", "safe": "ok" });
        let outcome = WorkflowV2BranchOutcome {
            item_id: "item-1".to_string(),
            role: "coder".to_string(),
            status: WorkflowV2Status::Accepted,
            result: Some(result),
            error: Some("token=branchsecret".to_string()),
        };

        let path = store
            .save_branch_outcome("implementation", &outcome)
            .expect("save branch");
        let raw = std::fs::read_to_string(path).expect("persisted branch");

        assert!(!raw.contains("do-not-store"));
        assert!(!raw.contains("access_token"));
        assert!(!raw.contains("branchsecret"));
        assert!(raw.contains("ok"));
    }
}

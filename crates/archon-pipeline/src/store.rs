//! Persistent state store for pipeline runs.
//!
//! Stores pipeline state, audit logs, and step checkpoints on disk with
//! integrity verification (SHA-256 checksums) and atomic writes.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::PipelineError;
use crate::id::PipelineId;
use crate::run::PipelineRun;

/// Audit trail events emitted during pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEvent {
    Started { spec_hash: String },
    StepStarted { step: String, attempt: u32 },
    StepFinished { step: String, output_len: usize },
    StepFailed { step: String, msg: String },
    StepSkipped { step: String },
    RetryScheduled { step: String, delay_ms: u64 },
    RolledBack { step: String },
    Cancelled,
    Resumed,
    Finished,
}

/// On-disk wrapper for audit log lines — adds a wall-clock timestamp.
#[derive(Debug, Serialize, Deserialize)]
struct AuditLine {
    ts: String,
    #[serde(flatten)]
    event: AuditEvent,
}

/// File-system backed state store for pipeline runs.
///
/// Directory layout per run:
/// ```text
/// {root}/{pipeline_id}/
///   state.json          — integrity-checked pipeline state
///   audit.log           — append-only JSONL audit trail
///   checkpoints/
///     {step_id}.json    — per-step checkpoint data
/// ```
pub struct PipelineStateStore {
    root: PathBuf,
}

impl PipelineStateStore {
    /// Create a new store rooted at the given directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Return the per-run directory path.
    fn run_dir(&self, id: PipelineId) -> PathBuf {
        self.root.join(id.to_string())
    }

    // ---- internal path helpers ----

    fn state_path(&self, id: PipelineId) -> PathBuf {
        self.run_dir(id).join("state.json")
    }

    fn state_tmp_path(&self, id: PipelineId) -> PathBuf {
        self.run_dir(id).join("state.json.tmp")
    }

    fn audit_path(&self, id: PipelineId) -> PathBuf {
        self.run_dir(id).join("audit.log")
    }

    fn checkpoints_dir(&self, id: PipelineId) -> PathBuf {
        self.run_dir(id).join("checkpoints")
    }

    fn checkpoint_path(&self, id: PipelineId, step_id: &str) -> PathBuf {
        self.checkpoints_dir(id).join(format!("{step_id}.json"))
    }

    // ---- public API ----

    /// Create on-disk scaffolding for a new pipeline run.
    ///
    /// Creates the run directory, an initial `state.json`, an empty `audit.log`,
    /// and the `checkpoints/` subdirectory.  Returns an error if the directory
    /// already exists.
    pub fn create(&self, run: &PipelineRun) -> Result<(), PipelineError> {
        let dir = self.run_dir(run.id);
        if dir.exists() {
            return Err(PipelineError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("run directory already exists: {}", dir.display()),
            )));
        }
        fs::create_dir_all(&dir)?;
        fs::create_dir_all(self.checkpoints_dir(run.id))?;

        // Initial state
        self.save_state(run)?;

        // Empty audit log
        File::create(self.audit_path(run.id))?;

        Ok(())
    }

    /// Atomically persist the pipeline run state to `state.json`.
    ///
    /// Writes to a `.tmp` file first, fsyncs, then renames — ensuring that a
    /// crash mid-write cannot corrupt the canonical state file.  The file is
    /// prefixed with a SHA-256 checksum line so that [`load_state`] can detect
    /// corruption.
    pub fn save_state(&self, run: &PipelineRun) -> Result<(), PipelineError> {
        let json = serde_json::to_string_pretty(run)
            .map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;

        let hash = hex::encode(Sha256::digest(json.as_bytes()));
        let content = format!("// sha256: {hash}\n{json}");

        let tmp = self.state_tmp_path(run.id);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(content.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, self.state_path(run.id))?;
        Ok(())
    }

    /// Load and verify a pipeline run from `state.json`.
    ///
    /// Returns [`PipelineError::StateCorrupted`] if the SHA-256 header does not
    /// match the JSON body.
    pub fn load_state(&self, id: PipelineId) -> Result<PipelineRun, PipelineError> {
        let raw = fs::read_to_string(self.state_path(id))?;

        let (header, body) = raw
            .split_once('\n')
            .ok_or_else(|| PipelineError::StateCorrupted("missing header line".into()))?;

        let expected_hash = header
            .strip_prefix("// sha256: ")
            .ok_or_else(|| PipelineError::StateCorrupted("invalid header format".into()))?;

        let actual_hash = hex::encode(Sha256::digest(body.as_bytes()));

        if actual_hash != expected_hash {
            return Err(PipelineError::StateCorrupted(format!(
                "checksum mismatch: expected {expected_hash}, got {actual_hash}"
            )));
        }

        let run: PipelineRun =
            serde_json::from_str(body).map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;
        Ok(run)
    }

    /// Append an audit event as a single JSONL line to `audit.log`.
    pub fn append_audit(&self, id: PipelineId, event: &AuditEvent) -> Result<(), PipelineError> {
        let line = AuditLine {
            ts: Utc::now().to_rfc3339(),
            event: event.clone(),
        };
        let mut json = serde_json::to_string(&line)
            .map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;
        json.push('\n');

        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.audit_path(id))?;
        f.write_all(json.as_bytes())?;
        Ok(())
    }

    /// Atomically write a checkpoint for a single step.
    pub fn write_checkpoint(
        &self,
        id: PipelineId,
        step_id: &str,
        output: &serde_json::Value,
    ) -> Result<(), PipelineError> {
        let dir = self.checkpoints_dir(id);
        fs::create_dir_all(&dir)?;

        let target = self.checkpoint_path(id, step_id);
        let tmp = target.with_extension("json.tmp");

        let json = serde_json::to_string_pretty(output)
            .map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;
        {
            let mut f = File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &target)?;
        Ok(())
    }

    /// Load a checkpoint for a step, returning `None` if it does not exist.
    pub fn load_checkpoint(
        &self,
        id: PipelineId,
        step_id: &str,
    ) -> Result<Option<serde_json::Value>, PipelineError> {
        let path = self.checkpoint_path(id, step_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;
        Ok(Some(value))
    }

    /// Delete a single step checkpoint, ignoring `NotFound`.
    pub fn delete_checkpoint(&self, id: PipelineId, step_id: &str) -> Result<(), PipelineError> {
        let path = self.checkpoint_path(id, step_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete mutable run artefacts but **preserve the audit log**.
    ///
    /// Removes `state.json`, `state.json.tmp`, and the `checkpoints/` directory,
    /// but leaves `audit.log` intact for post-mortem analysis.
    pub fn delete_run(&self, id: PipelineId) -> Result<(), PipelineError> {
        let dir = self.run_dir(id);
        if !dir.exists() {
            return Ok(());
        }

        // Remove state files (ignore NotFound)
        remove_if_exists(&self.state_path(id))?;
        remove_if_exists(&self.state_tmp_path(id))?;

        // Remove checkpoints directory recursively
        let cp_dir = self.checkpoints_dir(id);
        if cp_dir.exists() {
            fs::remove_dir_all(&cp_dir)?;
        }

        Ok(())
    }

    // ---- spec persistence ----

    /// Path to the serialized spec file.
    fn spec_path(&self, id: PipelineId) -> PathBuf {
        self.run_dir(id).join("spec.json")
    }

    fn spec_tmp_path(&self, id: PipelineId) -> PathBuf {
        self.run_dir(id).join("spec.json.tmp")
    }

    /// Atomically persist the pipeline spec to `spec.json`.
    pub fn save_spec(
        &self,
        id: PipelineId,
        spec: &crate::spec::PipelineSpec,
    ) -> Result<(), PipelineError> {
        let json = serde_json::to_string_pretty(spec)
            .map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;
        let tmp = self.spec_tmp_path(id);
        let target = self.spec_path(id);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &target)?;
        Ok(())
    }

    /// Load a previously saved pipeline spec from `spec.json`.
    pub fn load_spec(&self, id: PipelineId) -> Result<crate::spec::PipelineSpec, PipelineError> {
        let path = self.spec_path(id);
        let raw = fs::read_to_string(&path)?;
        let spec: crate::spec::PipelineSpec =
            serde_json::from_str(&raw).map_err(|e| PipelineError::StateCorrupted(e.to_string()))?;
        Ok(spec)
    }

    /// List all pipeline run IDs stored under the root directory.
    ///
    /// Only returns entries whose directory name is a valid UUID.
    pub fn list_runs(&self) -> Result<Vec<PipelineId>, PipelineError> {
        let mut ids = Vec::new();

        if !self.root.exists() {
            return Ok(ids);
        }

        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Ok(id) = name_str.parse::<PipelineId>() {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

/// Helper: remove a file, ignoring `NotFound`.
fn remove_if_exists(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{PipelineState, StepRun, StepRunState};
    use std::collections::HashMap;
    use tempfile::TempDir;

    /// Build a minimal `PipelineRun` for testing.
    fn sample_run() -> PipelineRun {
        PipelineRun {
            id: PipelineId::new(),
            spec_hash: "deadbeef".to_string(),
            state: PipelineState::Pending,
            steps: HashMap::new(),
            started_at: Utc::now(),
            finished_at: None,
        }
    }

    #[test]
    fn create_then_save_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let mut run = sample_run();
        store.create(&run).unwrap();

        // Mutate and save
        run.state = PipelineState::Running;
        run.steps.insert(
            "s1".into(),
            StepRun {
                task_id: None,
                state: StepRunState::Running,
                output: None,
                attempts: 1,
                last_error: None,
            },
        );
        store.save_state(&run).unwrap();

        let loaded = store.load_state(run.id).unwrap();
        assert_eq!(loaded.state, PipelineState::Running);
        assert!(loaded.steps.contains_key("s1"));
    }

    #[test]
    fn corrupted_state_detected() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let run = sample_run();
        store.create(&run).unwrap();

        // Tamper: change a character in the JSON body while keeping valid UTF-8
        let state_path = store.state_path(run.id);
        let raw = fs::read_to_string(&state_path).unwrap();
        let newline_pos = raw.find('\n').unwrap();
        let mut body = raw[newline_pos + 1..].to_string();
        // Replace the first '{' with '[' — still valid UTF-8, but hash will differ
        if let Some(pos) = body.find('{') {
            body.replace_range(pos..pos + 1, "[");
        }
        let tampered = format!("{}\n{}", &raw[..newline_pos], body);
        fs::write(&state_path, tampered.as_bytes()).unwrap();

        let err = store.load_state(run.id).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("corrupted") || msg.contains("checksum") || msg.contains("mismatch"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn audit_log_append_only() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let run = sample_run();
        store.create(&run).unwrap();

        let events = [
            AuditEvent::Started {
                spec_hash: "abc".into(),
            },
            AuditEvent::StepStarted {
                step: "s1".into(),
                attempt: 1,
            },
            AuditEvent::Finished,
        ];

        for e in &events {
            store.append_audit(run.id, e).unwrap();
        }

        let raw = fs::read_to_string(store.audit_path(run.id)).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 3);

        // Each line should be valid JSON with a "ts" field
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(v.get("ts").is_some());
        }
    }

    #[test]
    fn delete_run_preserves_audit_log() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let run = sample_run();
        store.create(&run).unwrap();
        store
            .append_audit(
                run.id,
                &AuditEvent::Started {
                    spec_hash: "x".into(),
                },
            )
            .unwrap();
        store
            .write_checkpoint(run.id, "s1", &serde_json::json!({"ok": true}))
            .unwrap();

        store.delete_run(run.id).unwrap();

        // audit.log still exists
        assert!(store.audit_path(run.id).exists());
        // state.json is gone
        assert!(!store.state_path(run.id).exists());
        // checkpoints/ dir is gone
        assert!(!store.checkpoints_dir(run.id).exists());
    }

    #[test]
    fn checkpoint_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let run = sample_run();
        store.create(&run).unwrap();

        let data = serde_json::json!({"tokens": 42, "summary": "hello"});
        store.write_checkpoint(run.id, "analysis", &data).unwrap();

        let loaded = store.load_checkpoint(run.id, "analysis").unwrap();
        assert_eq!(loaded, Some(data));

        // Non-existent checkpoint returns None
        let missing = store.load_checkpoint(run.id, "no-such-step").unwrap();
        assert!(missing.is_none());

        // Delete and verify
        store.delete_checkpoint(run.id, "analysis").unwrap();
        let after_delete = store.load_checkpoint(run.id, "analysis").unwrap();
        assert!(after_delete.is_none());

        // Delete again (idempotent)
        store.delete_checkpoint(run.id, "analysis").unwrap();
    }

    #[test]
    fn atomic_save_uses_rename() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let run = sample_run();
        store.create(&run).unwrap();

        // After a successful save, no .tmp file should remain
        store.save_state(&run).unwrap();
        assert!(!store.state_tmp_path(run.id).exists());
        assert!(store.state_path(run.id).exists());
    }

    #[test]
    fn list_runs_filters_to_uuid_dirs() {
        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        // Create two real runs
        let run1 = sample_run();
        let run2 = sample_run();
        store.create(&run1).unwrap();
        store.create(&run2).unwrap();

        // Create a stray non-UUID directory
        fs::create_dir_all(tmp.path().join("not-a-uuid")).unwrap();
        // Create a stray file (not a directory)
        fs::write(tmp.path().join("random-file.txt"), "hello").unwrap();

        let ids = store.list_runs().unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&run1.id));
        assert!(ids.contains(&run2.id));
    }

    #[test]
    fn spec_save_load_roundtrip() {
        use crate::spec::{BackoffKind, OnFailurePolicy, PipelineSpec, RetrySpec, StepSpec};

        let tmp = TempDir::new().unwrap();
        let store = PipelineStateStore::new(tmp.path());

        let run = sample_run();
        store.create(&run).unwrap();

        let spec = PipelineSpec {
            name: "roundtrip-test".to_string(),
            version: "2.0".to_string(),
            global_timeout_secs: 7200,
            max_parallelism: 4,
            steps: vec![StepSpec {
                id: "s1".to_string(),
                agent: "analyzer".to_string(),
                input: serde_json::json!({"key": "value"}),
                depends_on: vec![],
                retry: RetrySpec {
                    max_attempts: 3,
                    backoff: BackoffKind::Linear,
                    base_delay_ms: 500,
                },
                timeout_secs: 900,
                condition: Some("a.output.ok == true".to_string()),
                on_failure: OnFailurePolicy::Fail,
            }],
        };

        store.save_spec(run.id, &spec).unwrap();
        let loaded = store.load_spec(run.id).unwrap();
        assert_eq!(spec, loaded);
    }
}

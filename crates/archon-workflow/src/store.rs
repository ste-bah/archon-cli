use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{ArtifactRef, WorkflowRun};
use crate::spec::WorkflowSpec;

const RUN_SUBDIRS: &[&str] = &[
    "artifacts",
    "agent-outputs",
    "prompts",
    "reducers",
    "quality",
    "learning",
];

#[derive(Debug, Serialize, Deserialize)]
struct RunManifest {
    id: String,
    name: String,
    created_at: String,
    schema: String,
}

#[derive(Debug, Clone)]
pub struct WorkflowStore {
    root: PathBuf,
}

impl WorkflowStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".archon").join("workflows"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.root.join(run_id)
    }

    pub fn state_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("state.json")
    }

    pub fn events_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("events.jsonl")
    }

    pub fn create_run(&self, spec: WorkflowSpec) -> WorkflowResult<WorkflowRun> {
        let run = WorkflowRun::new(spec, &self.root);
        let dir = self.run_dir(&run.id);
        if dir.exists() {
            return Err(WorkflowError::RunAlreadyExists(run.id));
        }
        fs::create_dir_all(&dir).map_err(|e| WorkflowError::io(&dir, e))?;
        for subdir in RUN_SUBDIRS {
            let path = dir.join(subdir);
            fs::create_dir_all(&path).map_err(|e| WorkflowError::io(path, e))?;
        }
        File::create(self.events_path(&run.id))
            .map_err(|e| WorkflowError::io(self.events_path(&run.id), e))?;
        self.write_manifest(&run)?;
        self.write_spec(&run)?;
        self.save_state(&run)?;
        Ok(run)
    }

    pub fn save_state(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let target = self.state_path(&run.id);
        let tmp = target.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(run)?;
        write_atomic(&tmp, &target, &json)
    }

    pub fn load_state(&self, run_id: &str) -> WorkflowResult<WorkflowRun> {
        let path = self.state_path(run_id);
        let raw = fs::read(&path).map_err(|e| WorkflowError::io(&path, e))?;
        serde_json::from_slice(&raw).map_err(|e| WorkflowError::StateCorrupt(e.to_string()))
    }

    pub fn list_runs(&self) -> WorkflowResult<Vec<WorkflowRun>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut runs = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|e| WorkflowError::io(&self.root, e))? {
            let entry = entry.map_err(|e| WorkflowError::io(&self.root, e))?;
            if !entry.path().is_dir() {
                continue;
            }
            if let Some(id) = entry.file_name().to_str() {
                if let Ok(run) = self.load_state(id) {
                    runs.push(run);
                }
            }
        }
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs)
    }

    pub fn append_event_line(&self, run_id: &str, json_line: &str) -> WorkflowResult<()> {
        let path = self.events_path(run_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| WorkflowError::io(&path, e))?;
        file.write_all(json_line.as_bytes())
            .map_err(|e| WorkflowError::io(&path, e))?;
        file.write_all(b"\n")
            .map_err(|e| WorkflowError::io(&path, e))?;
        Ok(())
    }

    pub fn next_event_seq(&self, run_id: &str) -> WorkflowResult<u64> {
        let path = self.events_path(run_id);
        if !path.exists() {
            return Ok(1);
        }
        let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
        Ok(raw.lines().filter(|line| !line.trim().is_empty()).count() as u64 + 1)
    }

    pub fn write_artifact(
        &self,
        run_id: &str,
        producing_stage: &str,
        source_input_hash: &str,
        extension: &str,
        bytes: &[u8],
    ) -> WorkflowResult<ArtifactRef> {
        let content_hash = blake3::hash(bytes).to_hex().to_string();
        let safe_ext = extension.trim_start_matches('.').trim();
        let suffix = if safe_ext.is_empty() { "bin" } else { safe_ext };
        let id = format!("artifact-{content_hash}");
        let rel = PathBuf::from("artifacts").join(format!("{id}.{suffix}"));
        let target = self.run_dir(run_id).join(&rel);
        let tmp = target.with_extension(format!("{suffix}.tmp"));
        write_atomic(&tmp, &target, bytes)?;
        Ok(ArtifactRef {
            id,
            path: rel,
            content_hash,
            producing_stage: producing_stage.to_string(),
            source_input_hash: source_input_hash.to_string(),
            accepted: false,
        })
    }

    pub fn validate_for_reuse(
        &self,
        run: &WorkflowRun,
        artifact: &ArtifactRef,
        expected_source_input_hash: &str,
    ) -> WorkflowResult<()> {
        if artifact.source_input_hash != expected_source_input_hash {
            return Err(WorkflowError::ArtifactInvalid(
                "source input hash changed".into(),
            ));
        }
        if !run.accepted_stage(&artifact.producing_stage) || !artifact.accepted {
            return Err(WorkflowError::ArtifactInvalid(
                "producer stage is not accepted".into(),
            ));
        }
        let path = self.run_dir(&run.id).join(&artifact.path);
        let mut file = File::open(&path).map_err(|e| WorkflowError::io(&path, e))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| WorkflowError::io(&path, e))?;
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != artifact.content_hash {
            return Err(WorkflowError::ArtifactInvalid(
                "content hash mismatch".into(),
            ));
        }
        Ok(())
    }

    fn write_manifest(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let manifest = RunManifest {
            id: run.id.clone(),
            name: run.spec.name.clone(),
            created_at: Utc::now().to_rfc3339(),
            schema: run.spec.schema.clone(),
        };
        let path = self.run_dir(&run.id).join("manifest.toml");
        let tmp = path.with_extension("toml.tmp");
        let body = toml::to_string_pretty(&manifest)?;
        write_atomic(&tmp, &path, body.as_bytes())
    }

    fn write_spec(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let path = self.run_dir(&run.id).join("spec.yaml");
        let tmp = path.with_extension("yaml.tmp");
        let body = run.spec.to_yaml()?;
        write_atomic(&tmp, &path, body.as_bytes())
    }
}

fn write_atomic(tmp: &Path, target: &Path, bytes: &[u8]) -> WorkflowResult<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| WorkflowError::io(parent, e))?;
    }
    {
        let mut file = File::create(tmp).map_err(|e| WorkflowError::io(tmp, e))?;
        file.write_all(bytes)
            .map_err(|e| WorkflowError::io(tmp, e))?;
        file.sync_all().map_err(|e| WorkflowError::io(tmp, e))?;
    }
    fs::rename(tmp, target).map_err(|e| WorkflowError::io(target, e))?;
    Ok(())
}

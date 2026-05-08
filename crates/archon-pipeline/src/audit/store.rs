use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::audit::types::{
    AgentAuditRecord, BundleManifest, BundleState, BundleStatus, PipelineEvent, PipelineEventLine,
    PromptAuditRecord,
};
use crate::runner::PipelineType;

#[derive(Clone, Debug)]
pub struct PipelineBundleStore {
    root: PathBuf,
}

impl PipelineBundleStore {
    pub fn new(worktree: impl AsRef<Path>) -> Self {
        Self {
            root: worktree.as_ref().join(".archon").join("pipelines"),
        }
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn bundle_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(session_id)
    }

    pub fn create(
        &self,
        session_id: &str,
        pipeline_type: PipelineType,
        task: &str,
    ) -> Result<BundleState> {
        let dir = self.bundle_dir(session_id);
        fs::create_dir_all(dir.join("agents"))?;
        fs::create_dir_all(dir.join("prompts"))?;
        fs::create_dir_all(dir.join("outputs"))?;
        fs::create_dir_all(dir.join("verification"))?;
        fs::create_dir_all(dir.join("exports"))?;

        let manifest = BundleManifest {
            schema_version: 1,
            session_id: session_id.to_string(),
            pipeline_type: pipeline_type.clone(),
            archon_version: env!("CARGO_PKG_VERSION").to_string(),
            worktree_path: self
                .root
                .parent()
                .and_then(|archon_dir| archon_dir.parent())
                .unwrap_or_else(|| Path::new("."))
                .display()
                .to_string(),
            initial_git_head: git_head().ok(),
            initial_worktree_dirty: git_dirty().ok(),
            task: task.to_string(),
            created_at: Utc::now(),
        };
        write_json_atomic(&dir.join("manifest.json"), &manifest)?;

        let state = BundleState {
            session_id: session_id.to_string(),
            pipeline_type,
            task: task.to_string(),
            status: BundleStatus::Running,
            current_agent_key: None,
            completed_agent_count: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost_usd: 0.0,
            started_at: Utc::now(),
            updated_at: Utc::now(),
            completed_at: None,
            final_output_hash: None,
            completion_integrity_summary: None,
            completion_report_id: None,
            last_error: None,
        };
        self.save_state(&state)?;
        File::create(dir.join("audit.log"))?;
        self.append_event(
            session_id,
            PipelineEvent::RunCreated {
                session_id: session_id.to_string(),
                pipeline_type: state.pipeline_type.clone(),
            },
        )?;
        Ok(state)
    }

    pub fn load_manifest(&self, session_id: &str) -> Result<BundleManifest> {
        let raw = fs::read_to_string(self.bundle_dir(session_id).join("manifest.json"))?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn load_state(&self, session_id: &str) -> Result<BundleState> {
        let raw = fs::read_to_string(self.bundle_dir(session_id).join("state.json"))?;
        let (_, body) = split_hash_body(&raw)?;
        Ok(serde_json::from_str(body)?)
    }

    pub fn save_state(&self, state: &BundleState) -> Result<()> {
        let path = self.bundle_dir(&state.session_id).join("state.json");
        write_json_with_hash_atomic(&path, state)
    }

    pub fn append_event(&self, session_id: &str, event: PipelineEvent) -> Result<()> {
        let line = PipelineEventLine {
            ts: Utc::now(),
            event,
        };
        let mut json = serde_json::to_string(&line)?;
        json.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.bundle_dir(session_id).join("audit.log"))?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn write_prompt(&self, session_id: &str, record: &PromptAuditRecord) -> Result<PathBuf> {
        let path = self
            .bundle_dir(session_id)
            .join("prompts")
            .join(record_file_name(record.ordinal, &record.agent_key, "json"));
        write_json_atomic(&path, record)?;
        Ok(path)
    }

    pub fn write_output(
        &self,
        session_id: &str,
        ordinal: usize,
        agent_key: &str,
        output: &str,
    ) -> Result<PathBuf> {
        let path = self
            .bundle_dir(session_id)
            .join("outputs")
            .join(record_file_name(ordinal, agent_key, "txt"));
        write_text_atomic(&path, output)?;
        Ok(path)
    }

    pub fn write_attempt_output(
        &self,
        session_id: &str,
        ordinal: usize,
        agent_key: &str,
        attempt: usize,
        output: &str,
    ) -> Result<PathBuf> {
        let path = self
            .bundle_dir(session_id)
            .join("outputs")
            .join("attempts")
            .join(record_file_name(
                ordinal,
                &format!("{agent_key}-attempt-{attempt}"),
                "txt",
            ));
        write_text_atomic(&path, output)?;
        Ok(path)
    }

    pub fn write_agent(&self, session_id: &str, record: &AgentAuditRecord) -> Result<PathBuf> {
        let path = self
            .bundle_dir(session_id)
            .join("agents")
            .join(record_file_name(record.ordinal, &record.agent_key, "json"));
        write_json_atomic(&path, record)?;
        Ok(path)
    }

    pub fn list_agent_records(&self, session_id: &str) -> Result<Vec<AgentAuditRecord>> {
        let dir = self.bundle_dir(session_id).join("agents");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut records: Vec<AgentAuditRecord> = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path)?;
            records.push(serde_json::from_str(&raw).with_context(|| path.display().to_string())?);
        }
        records.sort_by_key(|record| record.ordinal);
        Ok(records)
    }

    pub fn list_states(&self) -> Result<Vec<BundleState>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut states = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Ok(state) = self.load_state(id) {
                states.push(state);
            }
        }
        states.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(states)
    }
}

pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}

pub fn json_hash(value: &impl serde::Serialize) -> Result<String> {
    Ok(sha256_hex(serde_json::to_vec(value)?))
}

pub fn record_file_name(ordinal: usize, agent_key: &str, ext: &str) -> String {
    let safe: String = agent_key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{ordinal:03}-{safe}.{ext}")
}

pub fn split_hash_body(raw: &str) -> Result<(&str, &str)> {
    let (header, body) = raw
        .split_once('\n')
        .context("state file missing checksum header")?;
    let expected = header
        .strip_prefix("// sha256: ")
        .context("state file has invalid checksum header")?;
    let actual = sha256_hex(body.as_bytes());
    if expected != actual {
        anyhow::bail!("state checksum mismatch: expected {expected}, got {actual}");
    }
    Ok((header, body))
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    write_text_atomic(path, &json)
}

fn write_json_with_hash_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    let hash = sha256_hex(json.as_bytes());
    write_text_atomic(path, &format!("// sha256: {hash}\n{json}"))
}

fn write_text_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn git_head() -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("git rev-parse failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_dirty() -> Result<bool> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("git status failed");
    }
    Ok(!out.stdout.is_empty())
}

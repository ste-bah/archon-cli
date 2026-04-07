//! Session management, checkpointing, and crash-recovery.
//!
//! All functions are synchronous and operate directly on the filesystem.
//! Checkpoints are written atomically (temp file + fsync + rename) so that
//! a crash mid-write never corrupts the session state.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::runner::PipelineType;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Current status of a pipeline session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Paused,
    Completed,
    Failed,
    Interrupted,
}

/// Record of a single agent that has finished execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletedAgent {
    pub agent_key: String,
    /// SHA-256 hex digest of the agent's output text.
    pub output_hash: String,
    pub quality_score: f64,
    pub cost_usd: f64,
    pub completed_at: DateTime<Utc>,
}

/// Full checkpoint state for a pipeline session, persisted to disk as JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineCheckpoint {
    pub session_id: String,
    pub pipeline_type: PipelineType,
    pub task: String,
    pub current_agent_key: Option<String>,
    pub completed_agents: Vec<CompletedAgent>,
    pub rlm_snapshot_path: Option<PathBuf>,
    pub total_cost_usd: f64,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
}

/// Lightweight summary returned by [`list_sessions`].
pub struct SessionSummary {
    pub session_id: String,
    pub pipeline_type: PipelineType,
    pub task: String,
    pub status: SessionStatus,
    pub completed_count: usize,
    pub total_cost_usd: f64,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a new session with a fresh UUID and `Running` status.
pub fn new_session(pipeline_type: PipelineType, task: &str) -> PipelineCheckpoint {
    PipelineCheckpoint {
        session_id: uuid::Uuid::new_v4().to_string(),
        pipeline_type,
        task: task.to_string(),
        current_agent_key: None,
        completed_agents: Vec::new(),
        rlm_snapshot_path: None,
        total_cost_usd: 0.0,
        started_at: Utc::now(),
        updated_at: Utc::now(),
        status: SessionStatus::Running,
    }
}

/// Atomically write a checkpoint to disk.
///
/// The write is performed via a temporary file, flushed with `fsync`, then
/// atomically renamed into place so that a crash at any point leaves either
/// the old checkpoint or the new one -- never a half-written file.
pub fn checkpoint(session: &PipelineCheckpoint, state_dir: &Path) -> Result<()> {
    let dir = state_dir
        .join(".pipeline-state")
        .join(&session.session_id);
    fs::create_dir_all(&dir)?;

    let tmp_path = dir.join("checkpoint.tmp");
    let final_path = dir.join("checkpoint.json");

    let json = serde_json::to_string_pretty(session)?;

    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }

    fs::rename(&tmp_path, &final_path)?;

    Ok(())
}

/// Detect sessions that were interrupted (status is `Running` or `Paused`).
///
/// Results are sorted by `updated_at` descending (most recent first).
pub fn detect_interrupted(state_dir: &Path) -> Result<Vec<PipelineCheckpoint>> {
    let pipeline_dir = state_dir.join(".pipeline-state");
    if !pipeline_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(&pipeline_dir)? {
        let entry = entry?;
        let cp_path = entry.path().join("checkpoint.json");
        if cp_path.exists() {
            let data = fs::read_to_string(&cp_path)?;
            let session: PipelineCheckpoint = serde_json::from_str(&data)?;
            if session.status == SessionStatus::Running
                || session.status == SessionStatus::Paused
            {
                sessions.push(session);
            }
        }
    }

    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// Resume an interrupted session by reloading it from disk and setting its
/// status back to `Running`.
pub fn resume(session_id: &str, state_dir: &Path) -> Result<PipelineCheckpoint> {
    let cp_path = state_dir
        .join(".pipeline-state")
        .join(session_id)
        .join("checkpoint.json");
    let data = fs::read_to_string(&cp_path)?;
    let mut session: PipelineCheckpoint = serde_json::from_str(&data)?;
    session.status = SessionStatus::Running;
    session.updated_at = Utc::now();
    checkpoint(&session, state_dir)?;
    Ok(session)
}

/// Abort a session by setting its status to `Failed` and writing a final
/// checkpoint.
pub fn abort(session_id: &str, state_dir: &Path) -> Result<()> {
    let cp_path = state_dir
        .join(".pipeline-state")
        .join(session_id)
        .join("checkpoint.json");
    let data = fs::read_to_string(&cp_path)?;
    let mut session: PipelineCheckpoint = serde_json::from_str(&data)?;
    session.status = SessionStatus::Failed;
    session.updated_at = Utc::now();
    checkpoint(&session, state_dir)?;
    Ok(())
}

/// List all sessions found in the state directory.
pub fn list_sessions(state_dir: &Path) -> Result<Vec<SessionSummary>> {
    let pipeline_dir = state_dir.join(".pipeline-state");
    if !pipeline_dir.exists() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::new();
    for entry in fs::read_dir(&pipeline_dir)? {
        let entry = entry?;
        let cp_path = entry.path().join("checkpoint.json");
        if cp_path.exists() {
            let data = fs::read_to_string(&cp_path)?;
            let session: PipelineCheckpoint = serde_json::from_str(&data)?;
            summaries.push(SessionSummary {
                session_id: session.session_id,
                pipeline_type: session.pipeline_type,
                task: session.task,
                status: session.status,
                completed_count: session.completed_agents.len(),
                total_cost_usd: session.total_cost_usd,
                started_at: session.started_at,
                updated_at: session.updated_at,
            });
        }
    }
    Ok(summaries)
}

/// Mark a session as completed and write the final checkpoint.
pub fn mark_completed(
    session: &mut PipelineCheckpoint,
    state_dir: &Path,
) -> Result<()> {
    session.status = SessionStatus::Completed;
    session.updated_at = Utc::now();
    checkpoint(session, state_dir)
}

/// Record that an agent has completed and return the [`CompletedAgent`] entry.
///
/// The output text is hashed with SHA-256 and stored as a hex string. The
/// agent's cost is accumulated into `session.total_cost_usd`.
pub fn record_agent_completion(
    session: &mut PipelineCheckpoint,
    agent_key: &str,
    output: &str,
    quality_score: f64,
    cost_usd: f64,
) -> CompletedAgent {
    let hash = hex::encode(Sha256::digest(output.as_bytes()));
    let completed = CompletedAgent {
        agent_key: agent_key.to_string(),
        output_hash: hash,
        quality_score,
        cost_usd,
        completed_at: Utc::now(),
    };
    session.completed_agents.push(completed.clone());
    session.total_cost_usd += cost_usd;
    session.updated_at = Utc::now();
    completed
}

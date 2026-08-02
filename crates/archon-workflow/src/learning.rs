//! The workflow side of the learning bridge: **one** record stream per run.
//!
//! # Why a file handoff at all
//!
//! `archon-pipeline` already integrates SONA, ReasoningBank, Reflexion, RLM,
//! JEPA and the world model in-process and closed-loop through
//! `LearningIntegration`. This crate cannot call it: `archon-workflow` depends
//! on exactly one Archon crate (`archon-llm`) and that thinness is deliberate —
//! it is why persistence here is file-based in the first place. So the workflow
//! side writes records and something above both crates reads them. That
//! consumer is the topology fold in `src/command/topology_fold/`, which is the
//! only layer that can see a workflow run *and* the learning stack.
//!
//! # Why one stream and not ten
//!
//! An earlier shape demultiplexed every stage outcome into `records.jsonl`,
//! `durable-memory.jsonl`, `world-traces.jsonl`, `governed-proposals.jsonl`,
//! six `adapter-*.jsonl` files and `adapter-records.jsonl` — roughly a dozen
//! copies of one outcome, split by consumer before any consumer existed. The
//! split is the reader's job: [`WorkflowLearningRecord::hooks`] carries the
//! spec's `learning_hooks`, so the fold routes from the record itself. One
//! append-only stream, one routing selector, no demultiplexing here.
//!
//! [`record_write_coordination_outcome`] is a separate, independently wired
//! metadata stream and is deliberately untouched by any of the above.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{ArtifactRef, RunStatus, StageStatus, WorkflowRun};
use crate::spec::{StageKind, StageSpec};
use crate::store::WorkflowStore;

/// File name of the single record stream, under `<run>/learning/`.
pub const LEARNING_RECORDS_FILE: &str = "records.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Accepted,
    Forced,
    Failed,
    Unverified,
}

impl Verification {
    /// Quality in `[0, 1]` for a stage that carried no explicit score.
    ///
    /// The consumer needs a number and the run only records one when a quality
    /// gate produced it, so this is the fallback: acceptance is full credit, a
    /// forced acceptance is partial (a human overrode a check that did not
    /// pass), and anything else is none.
    pub fn quality_fallback(&self) -> f64 {
        match self {
            Self::Accepted => 1.0,
            Self::Forced => 0.5,
            Self::Failed | Self::Unverified => 0.0,
        }
    }

    /// Whether this outcome describes a node that ran to completion.
    ///
    /// `Failed` counts: a failure is an outcome worth learning from. `Unverified`
    /// does not — the stage never reached a terminal state, so there is nothing
    /// to attribute.
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Accepted | Self::Forced | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageTelemetry {
    pub attempt: u32,
    pub error_class: Option<String>,
    pub artifact_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowLearningRecord {
    pub run_id: String,
    pub name: String,
    pub stage_id: String,
    /// Stage kind, snake_case. The consumer uses it as the learning phase, so
    /// episodes group by the kind of work rather than by workflow name.
    #[serde(default)]
    pub phase: String,
    /// Agent the spec named for this stage, when it named one.
    #[serde(default)]
    pub agent: Option<String>,
    pub status: StageStatus,
    pub verification: Verification,
    pub durable: bool,
    /// Score recorded by a quality gate, when one ran.
    #[serde(default)]
    pub quality_score: Option<f64>,
    pub artifact_refs: Vec<String>,
    pub telemetry: StageTelemetry,
    pub trace_ref: Option<String>,
    /// The spec's `learning_hooks`, verbatim. **The routing selector**: the fold
    /// dispatches on this and an empty list dispatches nothing. Carried per
    /// record so the stream is self-describing and the reader needs no second
    /// file to route.
    #[serde(default)]
    pub hooks: Vec<String>,
    pub ts: DateTime<Utc>,
}

impl WorkflowLearningRecord {
    /// The name to attribute this outcome to.
    ///
    /// Falls back to the phase, then the stage id: the generated V2 path builds
    /// its approval-metadata spec with `agent: None` on every stage, so a
    /// declared agent is the exception rather than the rule.
    pub fn agent_key(&self) -> &str {
        self.agent
            .as_deref()
            .filter(|agent| !agent.trim().is_empty())
            .or(Some(self.phase.as_str()))
            .filter(|phase| !phase.trim().is_empty())
            .unwrap_or(self.stage_id.as_str())
    }

    /// Quality for this outcome: the recorded score when there is one.
    pub fn quality(&self) -> f64 {
        self.quality_score
            .filter(|score| score.is_finite())
            .unwrap_or_else(|| self.verification.quality_fallback())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunLearningSummary {
    pub run_id: String,
    pub status: RunStatus,
    pub records: usize,
    pub durable_records: usize,
}

/// Writes the one record stream a run produces.
#[derive(Debug, Clone)]
pub struct WorkflowLearningSink {
    store: WorkflowStore,
}

impl WorkflowLearningSink {
    pub fn new(store: WorkflowStore) -> Self {
        Self { store }
    }

    /// Write `<run>/learning/records.jsonl` — one line per stage outcome.
    ///
    /// Rewritten in full rather than appended: a run's stage set is fixed and a
    /// resume re-derives every stage's current state, so a truncating write is
    /// what keeps a resumed run from double-counting its own stages.
    pub fn record(&self, run: &WorkflowRun) -> WorkflowResult<WorkflowRunLearningSummary> {
        let records = learning_records(run);
        let learning_dir = learning_dir(&self.store, &run.id);
        std::fs::create_dir_all(&learning_dir).map_err(|e| WorkflowError::io(&learning_dir, e))?;
        write_jsonl(&learning_dir.join(LEARNING_RECORDS_FILE), &records)?;

        Ok(WorkflowRunLearningSummary {
            run_id: run.id.clone(),
            status: run.status.clone(),
            durable_records: records.iter().filter(|record| record.durable).count(),
            records: records.len(),
        })
    }
}

fn learning_dir(store: &WorkflowStore, run_id: &str) -> PathBuf {
    store.run_dir(run_id).join("learning")
}

/// Path of the record stream for a run. The consumer reads it from here rather
/// than rebuilding the layout.
pub fn learning_records_path(store: &WorkflowStore, run_id: &str) -> PathBuf {
    learning_dir(store, run_id).join(LEARNING_RECORDS_FILE)
}

/// Read back the record stream a run produced.
///
/// A missing file is not an error — it means the run wrote nothing worth
/// consuming. Unparseable lines are skipped rather than failing the read, so a
/// truncated tail from a crash mid-write costs one record and not the stream.
pub fn read_learning_records(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<Vec<WorkflowLearningRecord>> {
    let path = learning_records_path(store, run_id);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(WorkflowError::io(path, error)),
    };
    Ok(contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<WorkflowLearningRecord>(line).ok())
        .collect())
}

pub fn learning_records(run: &WorkflowRun) -> Vec<WorkflowLearningRecord> {
    run.stages
        .values()
        .map(|stage| {
            let spec = run
                .spec
                .stages
                .iter()
                .find(|candidate| candidate.id == stage.id);
            let verification = verification_for(stage.status);
            let durable = verification == Verification::Accepted && !stage.artifacts.is_empty();
            WorkflowLearningRecord {
                run_id: run.id.clone(),
                name: run.spec.name.clone(),
                stage_id: stage.id.clone(),
                phase: spec.map(stage_phase).unwrap_or_default(),
                agent: spec.and_then(|spec| spec.agent.clone()),
                status: stage.status,
                verification,
                durable,
                quality_score: stage.quality_score,
                artifact_refs: artifact_ids(&stage.artifacts),
                telemetry: StageTelemetry {
                    attempt: stage.attempt,
                    error_class: stage.error.as_ref().map(|_| "stage_failed".to_string()),
                    artifact_count: stage.artifacts.len(),
                },
                trace_ref: stage.artifacts.first().map(|artifact| artifact.id.clone()),
                hooks: run.spec.learning_hooks.clone(),
                ts: Utc::now(),
            }
        })
        .collect()
}

fn stage_phase(stage: &StageSpec) -> String {
    match stage.kind {
        StageKind::Agent => "agent",
        StageKind::Fanout => "fanout",
        StageKind::Reduce => "reduce",
        StageKind::Tool => "tool",
        StageKind::Checkpoint => "checkpoint",
        StageKind::QualityGate => "quality_gate",
        StageKind::HumanGate => "human_gate",
        StageKind::Implementation => "implementation",
    }
    .to_string()
}

fn verification_for(status: StageStatus) -> Verification {
    match status {
        StageStatus::Accepted => Verification::Accepted,
        StageStatus::ForcedAccepted => Verification::Forced,
        StageStatus::NeedsReview => Verification::Failed,
        StageStatus::Blocked => Verification::Failed,
        StageStatus::Failed => Verification::Failed,
        StageStatus::Cancelled => Verification::Unverified,
        StageStatus::Pending
        | StageStatus::Running
        | StageStatus::Paused
        | StageStatus::Skipped => Verification::Unverified,
    }
}

fn artifact_ids(artifacts: &[ArtifactRef]) -> Vec<String> {
    artifacts
        .iter()
        .map(|artifact| artifact.id.clone())
        .collect()
}

fn write_jsonl<T: Serialize>(path: &Path, values: &[T]) -> WorkflowResult<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| WorkflowError::io(PathBuf::from(path), e))?;
    for value in values {
        let line = serde_json::to_string(value)?;
        file.write_all(line.as_bytes())
            .map_err(|e| WorkflowError::io(PathBuf::from(path), e))?;
        file.write_all(b"\n")
            .map_err(|e| WorkflowError::io(PathBuf::from(path), e))?;
    }
    file.sync_all()
        .map_err(|e| WorkflowError::io(PathBuf::from(path), e))?;
    Ok(())
}

/// TASK-WC-008 — write METADATA-ONLY learning rows for a coordinated outcome.
/// Never embeds patch bytes or diff lines; blake3 hashes + path names + sizes
/// are allowed (§18). One row per item (carrying its wave id).
pub fn record_write_coordination_outcome(
    store: &WorkflowStore,
    outcome: &crate::write_coordinator::coordinator::CoordinatedOutcome,
) -> WorkflowResult<()> {
    let dir = store
        .run_dir(&outcome.run_id)
        .join("learning")
        .join("write-coordination");
    std::fs::create_dir_all(&dir).map_err(|e| WorkflowError::io(&dir, e))?;
    let rows: Vec<serde_json::Value> = outcome
        .plans
        .iter()
        .map(|plan| {
            let status = outcome
                .item_status
                .get(&plan.item_id)
                .map(|s| format!("{s:?}"))
                .unwrap_or_else(|| "Unknown".into());
            serde_json::json!({
                "run_id": outcome.run_id,
                "stage_id": outcome.stage_id,
                "item_id": plan.item_id,
                "wave_id": plan.wave_id,
                "status": status,
                "changed_files": plan.changed_files,
                "patch_byte_size": plan.patch_bytes_len,
                "blake3_hashes": plan.post_hashes,
            })
        })
        .collect();
    write_jsonl(&dir.join("outcomes.jsonl"), &rows)
}

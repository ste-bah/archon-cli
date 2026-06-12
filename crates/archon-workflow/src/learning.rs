use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::run::{ArtifactRef, RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Accepted,
    Forced,
    Failed,
    Unverified,
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
    pub status: StageStatus,
    pub verification: Verification,
    pub durable: bool,
    pub artifact_refs: Vec<String>,
    pub telemetry: StageTelemetry,
    pub trace_ref: Option<String>,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunLearningSummary {
    pub run_id: String,
    pub status: RunStatus,
    pub records: usize,
    pub durable_records: usize,
    pub adapter_records: usize,
    pub proposal_records: usize,
}

#[derive(Debug, Clone)]
pub struct WorkflowLearningSink {
    store: WorkflowStore,
}

impl WorkflowLearningSink {
    pub fn new(store: WorkflowStore) -> Self {
        Self { store }
    }

    pub fn record(&self, run: &WorkflowRun) -> WorkflowResult<WorkflowRunLearningSummary> {
        let records = learning_records(run);
        let learning_dir = self.store.run_dir(&run.id).join("learning");
        std::fs::create_dir_all(&learning_dir).map_err(|e| WorkflowError::io(&learning_dir, e))?;
        write_jsonl(&learning_dir.join("records.jsonl"), &records)?;

        let durable = records
            .iter()
            .filter(|record| record.durable)
            .cloned()
            .collect::<Vec<_>>();
        let proposals = proposal_records(&durable);
        let adapters = adapter_records(&durable);
        write_jsonl(&learning_dir.join("durable-memory.jsonl"), &durable)?;
        write_jsonl(
            &learning_dir.join("world-traces.jsonl"),
            &trace_records(&durable),
        )?;
        write_jsonl(&learning_dir.join("governed-proposals.jsonl"), &proposals)?;
        write_adapter_files(&learning_dir, &adapters)?;

        Ok(WorkflowRunLearningSummary {
            run_id: run.id.clone(),
            status: run.status.clone(),
            records: records.len(),
            durable_records: durable.len(),
            adapter_records: adapters.len(),
            proposal_records: proposals.len(),
        })
    }
}

pub fn learning_records(run: &WorkflowRun) -> Vec<WorkflowLearningRecord> {
    run.stages
        .values()
        .map(|stage| {
            let verification = verification_for(stage.status);
            let durable = verification == Verification::Accepted && !stage.artifacts.is_empty();
            WorkflowLearningRecord {
                run_id: run.id.clone(),
                name: run.spec.name.clone(),
                stage_id: stage.id.clone(),
                status: stage.status,
                verification,
                durable,
                artifact_refs: artifact_ids(&stage.artifacts),
                telemetry: StageTelemetry {
                    attempt: stage.attempt,
                    error_class: stage.error.as_ref().map(|_| "stage_failed".to_string()),
                    artifact_count: stage.artifacts.len(),
                },
                trace_ref: stage.artifacts.first().map(|artifact| artifact.id.clone()),
                ts: Utc::now(),
            }
        })
        .collect()
}

fn verification_for(status: StageStatus) -> Verification {
    match status {
        StageStatus::Accepted => Verification::Accepted,
        StageStatus::ForcedAccepted => Verification::Forced,
        StageStatus::Failed => Verification::Failed,
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

fn trace_records(records: &[WorkflowLearningRecord]) -> Vec<serde_json::Value> {
    records
        .iter()
        .filter_map(|record| {
            Some(serde_json::json!({
                "run_id": record.run_id,
                "stage_id": record.stage_id,
                "trace_ref": record.trace_ref.as_ref()?,
                "surface": "dynamic_workflow",
                "durable": record.durable,
            }))
        })
        .collect()
}

fn proposal_records(records: &[WorkflowLearningRecord]) -> Vec<serde_json::Value> {
    records
        .iter()
        .filter(|record| record.telemetry.error_class.is_some())
        .map(|record| {
            serde_json::json!({
                "run_id": record.run_id,
                "stage_id": record.stage_id,
                "kind": "workflow_pattern",
                "applied": false,
            })
        })
        .collect()
}

fn adapter_records(records: &[WorkflowLearningRecord]) -> Vec<serde_json::Value> {
    records
        .iter()
        .flat_map(|record| {
            adapter_targets().into_iter().map(move |target| {
                serde_json::json!({
                    "target": target.name,
                    "adapter": target.file_stem,
                    "run_id": record.run_id,
                    "stage_id": record.stage_id,
                    "workflow": record.name,
                    "trace_ref": &record.trace_ref,
                    "artifact_refs": &record.artifact_refs,
                    "surface": "dynamic_workflow",
                    "verification": &record.verification,
                    "ts": record.ts,
                })
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct AdapterTarget {
    name: &'static str,
    file_stem: &'static str,
}

fn adapter_targets() -> Vec<AdapterTarget> {
    vec![
        AdapterTarget {
            name: "SONA",
            file_stem: "sona",
        },
        AdapterTarget {
            name: "RLM",
            file_stem: "rlm",
        },
        AdapterTarget {
            name: "Reflexion",
            file_stem: "reflexion",
        },
        AdapterTarget {
            name: "ReasoningBank",
            file_stem: "reasoning-bank",
        },
        AdapterTarget {
            name: "JEPA",
            file_stem: "jepa",
        },
        AdapterTarget {
            name: "WorldModel",
            file_stem: "world-model",
        },
    ]
}

fn write_adapter_files(dir: &Path, records: &[serde_json::Value]) -> WorkflowResult<()> {
    for target in adapter_targets() {
        let values = records
            .iter()
            .filter(|record| {
                record.get("adapter").and_then(|value| value.as_str()) == Some(target.file_stem)
            })
            .cloned()
            .collect::<Vec<_>>();
        write_jsonl(
            &dir.join(format!("adapter-{}.jsonl", target.file_stem)),
            &values,
        )?;
    }
    write_jsonl(&dir.join("adapter-records.jsonl"), records)
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

pub(crate) fn record_workflow_learning(
    store: &WorkflowStore,
    run: &WorkflowRun,
    seq: &mut u64,
) -> WorkflowResult<()> {
    if !matches!(run.status, RunStatus::Completed | RunStatus::Failed) {
        return Ok(());
    }
    let log = WorkflowEventLog::new(store.clone());
    match WorkflowLearningSink::new(store.clone()).record(run) {
        Ok(summary) => {
            log.emit(
                &run.id,
                *seq,
                WorkflowEventKind::LearningRecorded,
                json!({
                    "records": summary.records,
                    "durable_records": summary.durable_records,
                    "adapter_records": summary.adapter_records,
                    "proposal_records": summary.proposal_records,
                }),
            )?;
        }
        Err(err) => {
            log.emit(
                &run.id,
                *seq,
                WorkflowEventKind::LearningRecorded,
                json!({"status": "degraded", "error_class": err.to_string()}),
            )?;
        }
    }
    *seq += 1;
    Ok(())
}

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_workflow::{ExecutionReport, RunStatus, StageStatus, WorkflowRun, WorkflowStore};
use archon_world_model::labels::DeterministicLabelBuilder;
use archon_world_model::schema::{
    EvidenceRef, ScalarFeatures, WorldActionKind, WorldTraceRow, WorldTraceSource,
};
use archon_world_model::storage::{RetentionPolicy, WorldModelStore};

pub(crate) fn record_report(store: &WorkflowStore, report: &ExecutionReport) -> String {
    match record_report_inner(store, report) {
        Ok(summary) => summary.note(),
        Err(error) => format!("World-model workflow learning: degraded ({error})"),
    }
}

fn record_report_inner(
    store: &WorkflowStore,
    report: &ExecutionReport,
) -> Result<WorkflowWorldLearningSummary> {
    let config = archon_core::config::load_config().unwrap_or_default();
    if !config.learning.world_model.enabled {
        return Ok(WorkflowWorldLearningSummary::disabled(
            "world model disabled",
        ));
    }
    if !config.learning.world_model.include_agent_outputs {
        return Ok(WorkflowWorldLearningSummary::disabled(
            "world model agent-output ingest disabled",
        ));
    }
    let run = store.load_state(&report.run_id)?;
    persist_run_to_world_model(&run, &world_model_root()?, retention_policy(&config))
}

fn persist_run_to_world_model(
    run: &WorkflowRun,
    root: &Path,
    retention: RetentionPolicy,
) -> Result<WorkflowWorldLearningSummary> {
    if !matches!(run.status, RunStatus::Completed | RunStatus::Failed) {
        return Ok(WorkflowWorldLearningSummary::skipped(
            "workflow not terminal",
        ));
    }
    let mut rows = workflow_rows(run);
    let rows_seen = rows.len();
    let store = WorldModelStore::open(root)?;
    let known = store.row_ids()?;
    rows.retain(|row| !known.contains(&row.row_id));
    if rows.is_empty() {
        return Ok(WorkflowWorldLearningSummary {
            rows_seen,
            ..WorkflowWorldLearningSummary::skipped("already up to date")
        });
    }
    let persisted = store.persist_rows_with_retention(&rows, retention)?;
    Ok(WorkflowWorldLearningSummary {
        rows_seen,
        rows_persisted: persisted.jsonl_rows,
        cozo_rows: persisted.cozo_rows,
        db_path: Some(persisted.db_path),
        disabled_reason: None,
        skipped_reason: None,
    })
}

fn workflow_rows(run: &WorkflowRun) -> Vec<WorldTraceRow> {
    let mut rows = run
        .stages
        .values()
        .filter(|stage| terminal_status(stage.status))
        .map(|stage| stage_row(run, stage))
        .collect::<Vec<_>>();
    rows.extend(
        run.items
            .values()
            .filter(|item| terminal_status(item.status))
            .map(|item| item_row(run, item)),
    );
    rows
}

fn stage_row(run: &WorkflowRun, stage: &archon_workflow::run::StageState) -> WorldTraceRow {
    let mut row = base_row(
        run,
        format!(
            "world-row-workflow-stage-{}-{}-{}",
            run.id, stage.id, stage.attempt
        ),
        stage.status,
        stage.attempt,
        stage.quality_score,
    );
    row.agent = stage_agent(run, &stage.id);
    row.created_at = stage.completed_at.unwrap_or(run.updated_at);
    row.redacted_excerpt = Some(format!(
        "workflow={} stage={} status={} attempt={} artifacts={} error={}",
        run.spec.name,
        stage.id,
        status_name(stage.status),
        stage.attempt,
        stage.artifacts.len(),
        stage.error.as_deref().unwrap_or("")
    ));
    row.evidence_refs
        .push(EvidenceRef::new("workflow_stage", stage.id.clone()));
    row.evidence_refs
        .extend(stage.artifacts.iter().map(|artifact| {
            let mut evidence = EvidenceRef::new("workflow_artifact", artifact.id.clone());
            evidence.path = Some(run.root.join(&artifact.path).display().to_string());
            evidence.hash = Some(artifact.content_hash.clone());
            evidence
        }));
    label_row(row, stage.status)
}

fn item_row(run: &WorkflowRun, item: &archon_workflow::run::ItemState) -> WorldTraceRow {
    let mut row = base_row(
        run,
        format!("world-row-workflow-item-{}-{}", run.id, item.id),
        item.status,
        1,
        None,
    );
    row.agent = stage_agent(run, &item.stage_id);
    row.redacted_excerpt = Some(format!(
        "workflow={} fanout_item={} stage={} status={} error={}",
        run.spec.name,
        item.id,
        item.stage_id,
        status_name(item.status),
        item.error.as_deref().unwrap_or("")
    ));
    row.evidence_refs
        .push(EvidenceRef::new("workflow_item", item.id.clone()));
    if let Some(artifact) = &item.artifact {
        let mut evidence = EvidenceRef::new("workflow_artifact", artifact.id.clone());
        evidence.path = Some(run.root.join(&artifact.path).display().to_string());
        evidence.hash = Some(artifact.content_hash.clone());
        row.evidence_refs.push(evidence);
    }
    label_row(row, item.status)
}

fn base_row(
    run: &WorkflowRun,
    row_id: String,
    status: StageStatus,
    attempt: u32,
    quality_score: Option<f64>,
) -> WorldTraceRow {
    let action_kind = if attempt > 1 {
        WorldActionKind::Retry
    } else {
        WorldActionKind::AgentAttempt
    };
    let mut row = WorldTraceRow::new(run.id.clone(), action_kind).with_row_id(row_id);
    row.run_id = Some(run.id.clone());
    row.source = WorldTraceSource::Workflow;
    row.created_at = run.updated_at;
    row.scalar_features = ScalarFeatures {
        attempt_index: Some(attempt),
        quality_overall: quality_score,
        ..ScalarFeatures::default()
    };
    row.labels.success = success_label(status);
    row.labels.failure = matches!(status, StageStatus::Failed);
    row.labels.retry = attempt > 1;
    row.evidence_refs
        .push(EvidenceRef::new("workflow_run", run.id.clone()));
    row
}

fn label_row(mut row: WorldTraceRow, status: StageStatus) -> WorldTraceRow {
    let mut labels = DeterministicLabelBuilder.label_row(&row);
    labels.success = success_label(status);
    labels.failure |= matches!(status, StageStatus::Failed);
    labels.retry |= row.scalar_features.attempt_index.unwrap_or_default() > 1;
    row.labels = labels;
    row
}

fn stage_agent(run: &WorkflowRun, stage_id: &str) -> Option<String> {
    run.spec
        .stages
        .iter()
        .find(|stage| stage.id == stage_id)
        .and_then(|stage| stage.agent.clone())
        .or_else(|| Some(stage_id.to_string()))
}

fn terminal_status(status: StageStatus) -> bool {
    matches!(
        status,
        StageStatus::Accepted | StageStatus::Failed | StageStatus::ForcedAccepted
    )
}

fn success_label(status: StageStatus) -> Option<bool> {
    match status {
        StageStatus::Accepted | StageStatus::ForcedAccepted => Some(true),
        StageStatus::Failed => Some(false),
        _ => None,
    }
}

fn status_name(status: StageStatus) -> &'static str {
    match status {
        StageStatus::Pending => "pending",
        StageStatus::Running => "running",
        StageStatus::Paused => "paused",
        StageStatus::Accepted => "accepted",
        StageStatus::Failed => "failed",
        StageStatus::Skipped => "skipped",
        StageStatus::ForcedAccepted => "forced_accepted",
    }
}

fn retention_policy(config: &archon_core::config::ArchonConfig) -> RetentionPolicy {
    let retention = &config.learning.world_model.retention;
    RetentionPolicy {
        jsonl_rotate_bytes: retention.jsonl_rotate_mb.saturating_mul(1024 * 1024),
        raw_retention_days: retention.raw_retention_days,
    }
}

fn world_model_root() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("home directory unavailable"))?;
    Ok(home.join(".archon").join("world-model"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowWorldLearningSummary {
    rows_seen: usize,
    rows_persisted: usize,
    cozo_rows: usize,
    db_path: Option<PathBuf>,
    disabled_reason: Option<&'static str>,
    skipped_reason: Option<&'static str>,
}

impl WorkflowWorldLearningSummary {
    fn disabled(reason: &'static str) -> Self {
        Self {
            rows_seen: 0,
            rows_persisted: 0,
            cozo_rows: 0,
            db_path: None,
            disabled_reason: Some(reason),
            skipped_reason: None,
        }
    }

    fn skipped(reason: &'static str) -> Self {
        Self {
            rows_seen: 0,
            rows_persisted: 0,
            cozo_rows: 0,
            db_path: None,
            disabled_reason: None,
            skipped_reason: Some(reason),
        }
    }

    fn note(&self) -> String {
        if let Some(reason) = self.disabled_reason {
            return format!("World-model workflow learning: disabled ({reason})");
        }
        if let Some(reason) = self.skipped_reason {
            return format!(
                "World-model workflow learning: skipped ({reason}; rows_seen={})",
                self.rows_seen
            );
        }
        format!(
            "World-model workflow learning: persisted {} row(s), cozo_upserts={}, store={}",
            self.rows_persisted,
            self.cozo_rows,
            self.db_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "unknown".into())
        )
    }
}

#[cfg(test)]
mod tests {
    use archon_workflow::{HeuristicWorkflowPlanner, WorkflowExecutor, WorkflowPlanner};

    use super::*;

    #[test]
    fn terminal_workflow_rows_feed_world_model_store_once() {
        let temp = tempfile::tempdir().unwrap();
        let workflow_root = temp.path().join("workflows");
        let world_root = temp.path().join("world-model");
        let store = WorkflowStore::new(&workflow_root);
        let spec = HeuristicWorkflowPlanner
            .plan("Research a topic with final report")
            .unwrap();
        let executor = WorkflowExecutor::new(store.clone(), Default::default());
        let run = executor.start(spec).unwrap();
        executor.execute(run.clone()).unwrap();
        let finished = store.load_state(&run.id).unwrap();

        let first =
            persist_run_to_world_model(&finished, &world_root, RetentionPolicy::default()).unwrap();
        let second =
            persist_run_to_world_model(&finished, &world_root, RetentionPolicy::default()).unwrap();
        let rows = WorldModelStore::open(&world_root)
            .unwrap()
            .load_rows()
            .unwrap();

        assert!(first.rows_persisted > 0);
        assert_eq!(second.rows_persisted, 0);
        assert_eq!(rows.len(), first.rows_persisted);
        assert!(
            rows.iter()
                .all(|row| row.source == WorldTraceSource::Workflow)
        );
        assert!(rows.iter().any(|row| row.labels.success == Some(true)));
    }

    #[test]
    fn failed_workflow_stage_becomes_negative_world_model_example() {
        let spec = HeuristicWorkflowPlanner.plan("Research a topic").unwrap();
        let mut run = WorkflowRun::new(spec, "/tmp/workflows");
        run.status = RunStatus::Failed;
        run.stage_mut("discover").unwrap().status = StageStatus::Failed;
        run.stage_mut("discover").unwrap().attempt = 1;
        run.stage_mut("discover").unwrap().error = Some("quality gate failed".into());

        let rows = workflow_rows(&run);

        let failed = rows.iter().find(|row| row.labels.failure).unwrap();
        assert_eq!(failed.labels.success, Some(false));
        assert_eq!(failed.run_id.as_deref(), Some(run.id.as_str()));
        assert_eq!(failed.source, WorldTraceSource::Workflow);
    }
}

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    WorkflowV2Artifact, WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2EvidenceKind,
    WorkflowV2FileRecord, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2ReportPaths {
    pub harness_path: String,
    pub run_state_path: String,
    pub event_log_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2FinalReport {
    pub status: WorkflowV2Status,
    pub paths: WorkflowV2ReportPaths,
    pub task_coverage: Vec<WorkflowV2TaskCoverage>,
    pub files_read: Vec<WorkflowV2FileRecord>,
    pub files_changed: Vec<WorkflowV2FileRecord>,
    pub commands_run: Vec<WorkflowV2CommandRecord>,
    pub tests_run: Vec<WorkflowV2CommandRecord>,
    pub review_findings: Vec<String>,
    pub remediation_actions: Vec<String>,
    pub artifacts: Vec<WorkflowV2Artifact>,
    pub accepted_tasks: Vec<String>,
    pub noop_tasks: Vec<String>,
    pub failed_tasks: Vec<String>,
    pub blocked_tasks: Vec<String>,
    pub missing_tasks: Vec<String>,
    pub residual_gaps: Vec<WorkflowV2ResidualGap>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowV2FinalReportBuilder;

impl WorkflowV2FinalReportBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn build(
        &self,
        paths: WorkflowV2ReportPaths,
        required_task_ids: &[String],
        results: &[WorkflowV2Result],
    ) -> Result<WorkflowV2FinalReport, WorkflowV2FinalReportError> {
        validate_paths(&paths)?;
        let mut report = WorkflowV2FinalReport {
            status: WorkflowV2Status::Accepted,
            paths,
            task_coverage: Vec::new(),
            files_read: Vec::new(),
            files_changed: Vec::new(),
            commands_run: Vec::new(),
            tests_run: Vec::new(),
            review_findings: Vec::new(),
            remediation_actions: Vec::new(),
            artifacts: Vec::new(),
            accepted_tasks: Vec::new(),
            noop_tasks: Vec::new(),
            failed_tasks: Vec::new(),
            blocked_tasks: Vec::new(),
            missing_tasks: Vec::new(),
            residual_gaps: Vec::new(),
        };
        let mut coverage_by_task = BTreeMap::<String, Vec<WorkflowV2TaskCoverage>>::new();
        for (idx, result) in results.iter().enumerate() {
            result
                .validate()
                .map_err(|err| WorkflowV2FinalReportError::InvalidResult {
                    index: idx,
                    message: err.to_string(),
                })?;
            collect_result(idx, result, &mut report, &mut coverage_by_task);
        }

        classify_required_tasks(required_task_ids, &coverage_by_task, &mut report);
        if report.commands_run.is_empty() {
            report.status = WorkflowV2Status::NeedsReview;
        }
        if !report.failed_tasks.is_empty()
            || !report.blocked_tasks.is_empty()
            || !report.missing_tasks.is_empty()
            || !report.residual_gaps.is_empty()
        {
            report.status = WorkflowV2Status::NeedsReview;
        }
        Ok(report)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2FinalReportError {
    #[error("final report path '{field}' is required")]
    MissingPath { field: &'static str },
    #[error("workflow result at index {index} is invalid: {message}")]
    InvalidResult { index: usize, message: String },
}

fn validate_paths(paths: &WorkflowV2ReportPaths) -> Result<(), WorkflowV2FinalReportError> {
    if paths.harness_path.trim().is_empty() {
        return Err(WorkflowV2FinalReportError::MissingPath {
            field: "harness_path",
        });
    }
    if paths.run_state_path.trim().is_empty() {
        return Err(WorkflowV2FinalReportError::MissingPath {
            field: "run_state_path",
        });
    }
    if paths.event_log_path.trim().is_empty() {
        return Err(WorkflowV2FinalReportError::MissingPath {
            field: "event_log_path",
        });
    }
    Ok(())
}

fn collect_result(
    idx: usize,
    result: &WorkflowV2Result,
    report: &mut WorkflowV2FinalReport,
    coverage_by_task: &mut BTreeMap<String, Vec<WorkflowV2TaskCoverage>>,
) {
    report.files_read.extend(result.files_read.clone());
    report.files_changed.extend(result.files_changed.clone());
    report.commands_run.extend(result.commands_run.clone());
    report.tests_run.extend(
        result
            .commands_run
            .iter()
            .filter(|command| command.kind == WorkflowV2CommandKind::Test)
            .cloned(),
    );
    report.artifacts.extend(result.artifacts.clone());
    report.residual_gaps.extend(result.residual_gaps.clone());
    for evidence in &result.evidence {
        match evidence.kind {
            WorkflowV2EvidenceKind::Review => report.review_findings.push(evidence.summary.clone()),
            WorkflowV2EvidenceKind::Remediation => {
                report.remediation_actions.push(evidence.summary.clone())
            }
            _ => {}
        }
    }
    for coverage in &result.task_coverage {
        report.task_coverage.push(coverage.clone());
        coverage_by_task
            .entry(coverage.task_id.clone())
            .or_default()
            .push(coverage.clone());
    }
    if matches!(
        result.status,
        WorkflowV2Status::Pending
            | WorkflowV2Status::Running
            | WorkflowV2Status::Failed
            | WorkflowV2Status::Blocked
            | WorkflowV2Status::NeedsReview
            | WorkflowV2Status::Cancelled
    ) {
        let task_ids = if result.task_coverage.is_empty() {
            vec![format!("result:{idx}")]
        } else {
            result
                .task_coverage
                .iter()
                .map(|coverage| coverage.task_id.clone())
                .collect()
        };
        if result.status == WorkflowV2Status::Blocked {
            report.blocked_tasks.extend(task_ids);
        } else {
            report.failed_tasks.extend(task_ids);
        }
    }
}

fn classify_required_tasks(
    required_task_ids: &[String],
    coverage_by_task: &BTreeMap<String, Vec<WorkflowV2TaskCoverage>>,
    report: &mut WorkflowV2FinalReport,
) {
    let mut accepted = BTreeSet::new();
    let mut noop = BTreeSet::new();
    let mut failed = BTreeSet::new();
    let mut blocked = BTreeSet::new();
    let mut missing = BTreeSet::new();

    for task_id in required_task_ids {
        let Some(records) = coverage_by_task.get(task_id) else {
            missing.insert(task_id.clone());
            continue;
        };
        if records
            .iter()
            .any(|coverage| coverage.status == WorkflowV2TaskCoverageStatus::Blocked)
        {
            blocked.insert(task_id.clone());
        } else if records.iter().any(|coverage| {
            matches!(
                coverage.status,
                WorkflowV2TaskCoverageStatus::Partial
                    | WorkflowV2TaskCoverageStatus::Missing
                    | WorkflowV2TaskCoverageStatus::Unknown
            )
        }) {
            failed.insert(task_id.clone());
        } else if records
            .iter()
            .all(|coverage| coverage.status == WorkflowV2TaskCoverageStatus::Noop)
        {
            noop.insert(task_id.clone());
        } else {
            accepted.insert(task_id.clone());
        }
    }
    report.accepted_tasks = accepted.into_iter().collect();
    report.noop_tasks = noop.into_iter().collect();
    report.failed_tasks = merge_sorted(report.failed_tasks.clone(), failed);
    report.blocked_tasks = merge_sorted(report.blocked_tasks.clone(), blocked);
    report.missing_tasks = missing.into_iter().collect();
}

fn merge_sorted(mut existing: Vec<String>, extra: BTreeSet<String>) -> Vec<String> {
    existing.extend(extra);
    existing.sort();
    existing.dedup();
    existing
}

use thiserror::Error;

use super::{
    WorkflowV2CommandKind, WorkflowV2EvidenceKind, WorkflowV2ImplementationStatus,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2TaskRecord,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2InspectionDecision {
    pub task_id: String,
    pub implementation_status: WorkflowV2ImplementationStatus,
    pub noop_result: Option<WorkflowV2Result>,
    pub work_item: Option<WorkflowV2WorkItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WorkItem {
    pub kind: WorkflowV2WorkItemKind,
    pub task_id: String,
    pub description: String,
    pub expected_target_files: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub verification_commands: Vec<String>,
    pub depends_on: Vec<String>,
    pub write_isolation_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowV2WorkItemKind {
    Implementation,
    Investigation,
    Blocked,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowV2ImplementationInspector;

impl WorkflowV2ImplementationInspector {
    pub fn new() -> Self {
        Self
    }

    pub fn inspect_task(
        &self,
        task: &WorkflowV2TaskRecord,
        result: WorkflowV2Result,
    ) -> Result<WorkflowV2InspectionDecision, WorkflowV2InspectionError> {
        result
            .validate()
            .map_err(|err| WorkflowV2InspectionError::InvalidInspectionResult {
                task_id: task.task_id.clone(),
                message: err.to_string(),
            })?;
        let coverage = coverage_for_task(task, &result)?;
        match coverage.status {
            WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop => {
                validate_noop_proof(task, &result, coverage)?;
                Ok(WorkflowV2InspectionDecision {
                    task_id: task.task_id.clone(),
                    implementation_status: WorkflowV2ImplementationStatus::Complete,
                    noop_result: Some(result),
                    work_item: None,
                })
            }
            WorkflowV2TaskCoverageStatus::Partial | WorkflowV2TaskCoverageStatus::Missing => {
                let work_item = implementation_work_item(task, &result, coverage)?;
                Ok(WorkflowV2InspectionDecision {
                    task_id: task.task_id.clone(),
                    implementation_status: match coverage.status {
                        WorkflowV2TaskCoverageStatus::Partial => {
                            WorkflowV2ImplementationStatus::Partial
                        }
                        _ => WorkflowV2ImplementationStatus::Missing,
                    },
                    noop_result: None,
                    work_item: Some(work_item),
                })
            }
            WorkflowV2TaskCoverageStatus::Blocked => Ok(WorkflowV2InspectionDecision {
                task_id: task.task_id.clone(),
                implementation_status: WorkflowV2ImplementationStatus::Blocked,
                noop_result: None,
                work_item: Some(blocked_work_item(task, &result, coverage)?),
            }),
            WorkflowV2TaskCoverageStatus::Unknown => Ok(WorkflowV2InspectionDecision {
                task_id: task.task_id.clone(),
                implementation_status: WorkflowV2ImplementationStatus::Unknown,
                noop_result: None,
                work_item: Some(investigation_work_item(task, &result, coverage)?),
            }),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2InspectionError {
    #[error("inspection result for task '{task_id}' is invalid: {message}")]
    InvalidInspectionResult { task_id: String, message: String },
    #[error("inspection result does not include coverage for required task '{0}'")]
    MissingTaskCoverage(String),
    #[error("task '{0}' no-op proof requires files_read evidence")]
    NoopWithoutFilesRead(String),
    #[error("task '{0}' complete inspection must return noop status")]
    NoopWrongStatus(String),
    #[error("task '{0}' no-op proof requires command evidence")]
    NoopWithoutCommandEvidence(String),
    #[error("task '{0}' no-op proof requires inspection evidence")]
    NoopWithoutInspectionEvidence(String),
    #[error("task '{task_id}' no-op proof does not cover acceptance criterion: {criterion}")]
    NoopMissingAcceptanceCriterion { task_id: String, criterion: String },
    #[error("task '{0}' implementation work item requires target files")]
    WorkItemMissingTargetFiles(String),
    #[error("task '{0}' implementation work item requires verification commands")]
    WorkItemMissingVerificationCommands(String),
    #[error("task '{0}' implementation work item requires a concrete description")]
    WorkItemMissingDescription(String),
    #[error("task '{0}' blocked status requires blocker evidence or residual gap")]
    BlockedWithoutEvidence(String),
    #[error("task '{0}' unknown status requires inspection evidence")]
    UnknownWithoutEvidence(String),
}

fn coverage_for_task<'a>(
    task: &WorkflowV2TaskRecord,
    result: &'a WorkflowV2Result,
) -> Result<&'a WorkflowV2TaskCoverage, WorkflowV2InspectionError> {
    result
        .task_coverage
        .iter()
        .find(|coverage| coverage.task_id.eq_ignore_ascii_case(&task.task_id))
        .ok_or_else(|| WorkflowV2InspectionError::MissingTaskCoverage(task.task_id.clone()))
}

fn validate_noop_proof(
    task: &WorkflowV2TaskRecord,
    result: &WorkflowV2Result,
    coverage: &WorkflowV2TaskCoverage,
) -> Result<(), WorkflowV2InspectionError> {
    if result.status != WorkflowV2Status::Noop {
        return Err(WorkflowV2InspectionError::NoopWrongStatus(
            task.task_id.clone(),
        ));
    }
    if result
        .files_read
        .iter()
        .all(|file| file.path.trim().is_empty())
    {
        return Err(WorkflowV2InspectionError::NoopWithoutFilesRead(
            task.task_id.clone(),
        ));
    }
    if !has_command_evidence(result) {
        return Err(WorkflowV2InspectionError::NoopWithoutCommandEvidence(
            task.task_id.clone(),
        ));
    }
    if !has_inspection_evidence(result, coverage) {
        return Err(WorkflowV2InspectionError::NoopWithoutInspectionEvidence(
            task.task_id.clone(),
        ));
    }
    for criterion in &task.acceptance_criteria {
        if !acceptance_criterion_is_covered(criterion, coverage) {
            return Err(WorkflowV2InspectionError::NoopMissingAcceptanceCriterion {
                task_id: task.task_id.clone(),
                criterion: criterion.clone(),
            });
        }
    }
    Ok(())
}

fn has_inspection_evidence(result: &WorkflowV2Result, coverage: &WorkflowV2TaskCoverage) -> bool {
    result
        .evidence
        .iter()
        .chain(coverage.evidence.iter())
        .any(|evidence| {
            evidence.kind == WorkflowV2EvidenceKind::Inspection
                && !evidence.summary.trim().is_empty()
        })
}

fn has_command_evidence(result: &WorkflowV2Result) -> bool {
    result
        .commands_run
        .iter()
        .any(|command| !command.command.trim().is_empty())
}

fn acceptance_criterion_is_covered(criterion: &str, coverage: &WorkflowV2TaskCoverage) -> bool {
    let normalized = normalize_text(criterion);
    coverage.evidence.iter().any(|evidence| {
        evidence
            .source
            .as_deref()
            .map(normalize_text)
            .is_some_and(|source| {
                source
                    .strip_prefix("acceptance:")
                    .is_some_and(|value| value.trim().contains(&normalized))
            })
            || normalize_text(&evidence.summary).contains(&normalized)
    }) || normalize_text(&coverage.summary).contains(&normalized)
}

fn implementation_work_item(
    task: &WorkflowV2TaskRecord,
    result: &WorkflowV2Result,
    coverage: &WorkflowV2TaskCoverage,
) -> Result<WorkflowV2WorkItem, WorkflowV2InspectionError> {
    let description = coverage.summary.trim().to_string();
    if description.is_empty() {
        return Err(WorkflowV2InspectionError::WorkItemMissingDescription(
            task.task_id.clone(),
        ));
    }
    if task.candidate_target_files.is_empty() {
        return Err(WorkflowV2InspectionError::WorkItemMissingTargetFiles(
            task.task_id.clone(),
        ));
    }
    let verification_commands = verification_commands(result);
    if verification_commands.is_empty() {
        return Err(
            WorkflowV2InspectionError::WorkItemMissingVerificationCommands(task.task_id.clone()),
        );
    }
    Ok(WorkflowV2WorkItem {
        kind: WorkflowV2WorkItemKind::Implementation,
        task_id: task.task_id.clone(),
        description,
        expected_target_files: task.candidate_target_files.clone(),
        acceptance_criteria: task.acceptance_criteria.clone(),
        verification_commands,
        depends_on: task.depends_on.clone(),
        write_isolation_required: true,
    })
}

fn investigation_work_item(
    task: &WorkflowV2TaskRecord,
    result: &WorkflowV2Result,
    coverage: &WorkflowV2TaskCoverage,
) -> Result<WorkflowV2WorkItem, WorkflowV2InspectionError> {
    if !has_inspection_evidence(result, coverage) {
        return Err(WorkflowV2InspectionError::UnknownWithoutEvidence(
            task.task_id.clone(),
        ));
    }
    Ok(WorkflowV2WorkItem {
        kind: WorkflowV2WorkItemKind::Investigation,
        task_id: task.task_id.clone(),
        description: coverage.summary.trim().to_string(),
        expected_target_files: task.candidate_target_files.clone(),
        acceptance_criteria: task.acceptance_criteria.clone(),
        verification_commands: Vec::new(),
        depends_on: task.depends_on.clone(),
        write_isolation_required: false,
    })
}

fn blocked_work_item(
    task: &WorkflowV2TaskRecord,
    result: &WorkflowV2Result,
    coverage: &WorkflowV2TaskCoverage,
) -> Result<WorkflowV2WorkItem, WorkflowV2InspectionError> {
    if !has_blocker_evidence(result, coverage) {
        return Err(WorkflowV2InspectionError::BlockedWithoutEvidence(
            task.task_id.clone(),
        ));
    }
    Ok(WorkflowV2WorkItem {
        kind: WorkflowV2WorkItemKind::Blocked,
        task_id: task.task_id.clone(),
        description: coverage.summary.trim().to_string(),
        expected_target_files: task.candidate_target_files.clone(),
        acceptance_criteria: task.acceptance_criteria.clone(),
        verification_commands: Vec::new(),
        depends_on: task.depends_on.clone(),
        write_isolation_required: false,
    })
}

fn has_blocker_evidence(result: &WorkflowV2Result, coverage: &WorkflowV2TaskCoverage) -> bool {
    coverage
        .evidence
        .iter()
        .any(|evidence| !evidence.summary.trim().is_empty())
        || result.evidence.iter().any(|evidence| {
            evidence.kind == WorkflowV2EvidenceKind::Blocker && !evidence.summary.trim().is_empty()
        })
        || result
            .residual_gaps
            .iter()
            .any(|gap| !gap.id.trim().is_empty() && !gap.description.trim().is_empty())
}

fn verification_commands(result: &WorkflowV2Result) -> Vec<String> {
    result
        .commands_run
        .iter()
        .filter(|command| {
            matches!(
                command.kind,
                WorkflowV2CommandKind::Test
                    | WorkflowV2CommandKind::Build
                    | WorkflowV2CommandKind::Format
                    | WorkflowV2CommandKind::Other
            ) && !command.command.trim().is_empty()
        })
        .map(|command| command.command.clone())
        .collect()
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

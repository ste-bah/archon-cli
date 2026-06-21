use thiserror::Error;

use super::result::{
    WorkflowV2CommandKind, WorkflowV2CommandStatus, WorkflowV2EvidenceKind, WorkflowV2Result,
    WorkflowV2Status, WorkflowV2TaskCoverageStatus,
};

pub type WorkflowV2ValidationResult<T> = Result<T, WorkflowV2ValidationError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2ValidationError {
    #[error("workflow result summary is required")]
    MissingSummary,
    #[error("workflow result status {0:?} requires concrete evidence")]
    MissingEvidence(WorkflowV2Status),
    #[error("blocked workflow result requires a concrete blocker or residual gap")]
    MissingBlocker,
    #[error("residual gap at index {0} requires id and description")]
    EmptyResidualGap(usize),
    #[error("workflow result claims test evidence without a successful test command")]
    TestEvidenceWithoutCommand,
    #[error("artifact record at index {0} has an empty path")]
    EmptyArtifactPath(usize),
    #[error("changed file record at index {0} has an empty path")]
    EmptyChangedFilePath(usize),
    #[error("task coverage at index {0} has an empty task_id")]
    EmptyTaskId(usize),
    #[error("task coverage at index {0} requires a summary")]
    EmptyTaskCoverageSummary(usize),
    #[error("task coverage for '{0}' with status {1:?} requires evidence")]
    TaskCoverageMissingEvidence(String, WorkflowV2TaskCoverageStatus),
}

impl WorkflowV2Result {
    pub fn validate(&self) -> WorkflowV2ValidationResult<()> {
        validate_result(self)
    }
}

pub fn validate_result(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    if result.summary.trim().is_empty() {
        return Err(WorkflowV2ValidationError::MissingSummary);
    }
    validate_status_evidence(result)?;
    validate_artifacts(result)?;
    validate_residual_gaps(result)?;
    validate_test_evidence(result)?;
    validate_changed_files(result)?;
    validate_task_coverage(result)?;
    Ok(())
}

fn validate_status_evidence(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    match result.status {
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => {
            if !has_concrete_evidence(result) {
                return Err(WorkflowV2ValidationError::MissingEvidence(result.status));
            }
        }
        WorkflowV2Status::Blocked => {
            let has_blocker_evidence = result.evidence.iter().any(|evidence| {
                evidence.kind == WorkflowV2EvidenceKind::Blocker
                    && !evidence.summary.trim().is_empty()
            });
            let has_residual_gap = result
                .residual_gaps
                .iter()
                .any(|gap| !gap.id.trim().is_empty() && !gap.description.trim().is_empty());
            if !has_blocker_evidence && !has_residual_gap {
                return Err(WorkflowV2ValidationError::MissingBlocker);
            }
        }
        WorkflowV2Status::Pending
        | WorkflowV2Status::Running
        | WorkflowV2Status::Failed
        | WorkflowV2Status::NeedsReview
        | WorkflowV2Status::Cancelled => {}
    }
    Ok(())
}

fn has_concrete_evidence(result: &WorkflowV2Result) -> bool {
    result
        .evidence
        .iter()
        .any(|evidence| !evidence.summary.trim().is_empty())
        || result.task_coverage.iter().any(|coverage| {
            coverage
                .evidence
                .iter()
                .any(|evidence| !evidence.summary.trim().is_empty())
        })
        || result
            .commands_run
            .iter()
            .any(|command| !command.command.trim().is_empty())
}

fn validate_artifacts(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    if let Some((idx, _)) = result
        .artifacts
        .iter()
        .enumerate()
        .find(|(_, artifact)| artifact.path.trim().is_empty())
    {
        return Err(WorkflowV2ValidationError::EmptyArtifactPath(idx));
    }
    Ok(())
}

fn validate_residual_gaps(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    if let Some((idx, _)) = result
        .residual_gaps
        .iter()
        .enumerate()
        .find(|(_, gap)| gap.id.trim().is_empty() || gap.description.trim().is_empty())
    {
        return Err(WorkflowV2ValidationError::EmptyResidualGap(idx));
    }
    Ok(())
}

fn validate_test_evidence(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    let claims_test_success = result
        .evidence
        .iter()
        .any(|evidence| evidence.kind == WorkflowV2EvidenceKind::Test)
        || result.task_coverage.iter().any(|coverage| {
            coverage
                .evidence
                .iter()
                .any(|evidence| evidence.kind == WorkflowV2EvidenceKind::Test)
        });
    let has_test_command = result.commands_run.iter().any(|command| {
        command.kind == WorkflowV2CommandKind::Test && !command.command.trim().is_empty()
    });
    let has_successful_test_command = result.commands_run.iter().any(|command| {
        command.kind == WorkflowV2CommandKind::Test
            && command.status == WorkflowV2CommandStatus::Succeeded
            && !command.command.trim().is_empty()
    });
    let requires_successful_command = matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    );
    if claims_test_success
        && ((requires_successful_command && !has_successful_test_command)
            || (!requires_successful_command && !has_test_command))
    {
        return Err(WorkflowV2ValidationError::TestEvidenceWithoutCommand);
    }
    Ok(())
}

fn validate_changed_files(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    if let Some((idx, _)) = result
        .files_changed
        .iter()
        .enumerate()
        .find(|(_, file)| file.path.trim().is_empty())
    {
        return Err(WorkflowV2ValidationError::EmptyChangedFilePath(idx));
    }
    Ok(())
}

fn validate_task_coverage(result: &WorkflowV2Result) -> WorkflowV2ValidationResult<()> {
    for (idx, coverage) in result.task_coverage.iter().enumerate() {
        if coverage.task_id.trim().is_empty() {
            return Err(WorkflowV2ValidationError::EmptyTaskId(idx));
        }
        if coverage.summary.trim().is_empty() {
            return Err(WorkflowV2ValidationError::EmptyTaskCoverageSummary(idx));
        }
        if matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop
        ) && coverage
            .evidence
            .iter()
            .all(|evidence| evidence.summary.trim().is_empty())
        {
            return Err(WorkflowV2ValidationError::TaskCoverageMissingEvidence(
                coverage.task_id.clone(),
                coverage.status,
            ));
        }
    }
    Ok(())
}

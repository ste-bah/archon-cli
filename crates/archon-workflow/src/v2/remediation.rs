use thiserror::Error;

use super::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowV2VerificationKind {
    FocusedTest,
    AdversarialReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowV2VerificationStatus {
    Passed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2VerificationOutcome {
    pub kind: WorkflowV2VerificationKind,
    pub task_id: String,
    pub status: WorkflowV2VerificationStatus,
    pub summary: String,
    pub command: Option<WorkflowV2CommandRecord>,
    pub evidence: Vec<WorkflowV2Evidence>,
    pub external_blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2RemediationItem {
    pub id: String,
    pub task_id: String,
    pub source_kind: WorkflowV2VerificationKind,
    pub description: String,
    pub commands_to_rerun: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowV2ConvergenceStatus {
    Accepted,
    Remediate,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2ConvergenceDecision {
    pub status: WorkflowV2ConvergenceStatus,
    pub iteration: usize,
    pub remediation_items: Vec<WorkflowV2RemediationItem>,
    pub requires_reverification: bool,
    pub result: WorkflowV2Result,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2ConvergenceController {
    max_iterations: usize,
}

impl WorkflowV2ConvergenceController {
    pub fn new(max_iterations: usize) -> Self {
        Self {
            max_iterations: max_iterations.max(1),
        }
    }

    pub fn evaluate(
        &self,
        iteration: usize,
        outcomes: &[WorkflowV2VerificationOutcome],
    ) -> Result<WorkflowV2ConvergenceDecision, WorkflowV2ConvergenceError> {
        if outcomes.is_empty() {
            return Err(WorkflowV2ConvergenceError::MissingOutcomes);
        }
        for outcome in outcomes {
            validate_outcome(outcome)?;
        }

        if let Some(blocked) = outcomes
            .iter()
            .find(|outcome| outcome.status == WorkflowV2VerificationStatus::Blocked)
        {
            return Ok(blocked_decision(iteration, outcomes, blocked));
        }

        let failed = outcomes
            .iter()
            .filter(|outcome| outcome.status == WorkflowV2VerificationStatus::Failed)
            .collect::<Vec<_>>();
        if failed.is_empty() {
            validate_acceptance_outcomes(outcomes)?;
            return Ok(accepted_decision(iteration, outcomes));
        }
        if iteration >= self.max_iterations {
            return Ok(max_iterations_blocked_decision(iteration, outcomes));
        }
        Ok(remediation_decision(iteration, &failed, outcomes))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2ConvergenceError {
    #[error("verification/review loop requires at least one outcome")]
    MissingOutcomes,
    #[error("outcome for task '{0}' requires a summary")]
    MissingSummary(String),
    #[error("focused test outcome for task '{0}' requires a command")]
    MissingFocusedTestCommand(String),
    #[error("focused test command for task '{task_id}' only lists tests: {command}")]
    ListingCommandIsNotExecution { task_id: String, command: String },
    #[error("adversarial review outcome for task '{0}' requires review evidence")]
    MissingReviewEvidence(String),
    #[error("blocked outcome for task '{0}' requires concrete external blocker evidence")]
    MissingExternalBlocker(String),
    #[error("accepted convergence requires focused test outcome")]
    MissingFocusedTestOutcomeForAcceptance,
    #[error("accepted convergence requires adversarial review outcome")]
    MissingReviewOutcomeForAcceptance,
}

fn validate_outcome(
    outcome: &WorkflowV2VerificationOutcome,
) -> Result<(), WorkflowV2ConvergenceError> {
    if outcome.summary.trim().is_empty() {
        return Err(WorkflowV2ConvergenceError::MissingSummary(
            outcome.task_id.clone(),
        ));
    }
    if outcome.status == WorkflowV2VerificationStatus::Blocked
        && outcome
            .external_blocker
            .as_deref()
            .is_none_or(|blocker| blocker.trim().is_empty())
    {
        return Err(WorkflowV2ConvergenceError::MissingExternalBlocker(
            outcome.task_id.clone(),
        ));
    }
    match outcome.kind {
        WorkflowV2VerificationKind::FocusedTest => validate_test_outcome(outcome),
        WorkflowV2VerificationKind::AdversarialReview => validate_review_outcome(outcome),
    }
}

fn validate_test_outcome(
    outcome: &WorkflowV2VerificationOutcome,
) -> Result<(), WorkflowV2ConvergenceError> {
    let Some(command) = &outcome.command else {
        return Err(WorkflowV2ConvergenceError::MissingFocusedTestCommand(
            outcome.task_id.clone(),
        ));
    };
    if command.command.trim().is_empty() {
        return Err(WorkflowV2ConvergenceError::MissingFocusedTestCommand(
            outcome.task_id.clone(),
        ));
    }
    if looks_like_listing_command(&command.command) {
        return Err(WorkflowV2ConvergenceError::ListingCommandIsNotExecution {
            task_id: outcome.task_id.clone(),
            command: command.command.clone(),
        });
    }
    Ok(())
}

fn validate_review_outcome(
    outcome: &WorkflowV2VerificationOutcome,
) -> Result<(), WorkflowV2ConvergenceError> {
    if outcome.evidence.iter().all(|evidence| {
        evidence.kind != WorkflowV2EvidenceKind::Review || evidence.summary.trim().is_empty()
    }) {
        return Err(WorkflowV2ConvergenceError::MissingReviewEvidence(
            outcome.task_id.clone(),
        ));
    }
    Ok(())
}

fn validate_acceptance_outcomes(
    outcomes: &[WorkflowV2VerificationOutcome],
) -> Result<(), WorkflowV2ConvergenceError> {
    if !outcomes
        .iter()
        .any(|outcome| outcome.kind == WorkflowV2VerificationKind::FocusedTest)
    {
        return Err(WorkflowV2ConvergenceError::MissingFocusedTestOutcomeForAcceptance);
    }
    if !outcomes
        .iter()
        .any(|outcome| outcome.kind == WorkflowV2VerificationKind::AdversarialReview)
    {
        return Err(WorkflowV2ConvergenceError::MissingReviewOutcomeForAcceptance);
    }
    Ok(())
}

fn accepted_decision(
    iteration: usize,
    outcomes: &[WorkflowV2VerificationOutcome],
) -> WorkflowV2ConvergenceDecision {
    WorkflowV2ConvergenceDecision {
        status: WorkflowV2ConvergenceStatus::Accepted,
        iteration,
        remediation_items: Vec::new(),
        requires_reverification: false,
        result: result_from_outcomes(
            WorkflowV2Status::Accepted,
            "verification and adversarial review passed",
            outcomes,
        ),
    }
}

fn remediation_decision(
    iteration: usize,
    failed: &[&WorkflowV2VerificationOutcome],
    outcomes: &[WorkflowV2VerificationOutcome],
) -> WorkflowV2ConvergenceDecision {
    let remediation_items = failed
        .iter()
        .enumerate()
        .map(|(idx, outcome)| remediation_item(idx, outcome))
        .collect::<Vec<_>>();
    WorkflowV2ConvergenceDecision {
        status: WorkflowV2ConvergenceStatus::Remediate,
        iteration,
        remediation_items,
        requires_reverification: true,
        result: result_from_outcomes(
            WorkflowV2Status::NeedsReview,
            "verification or review failed; remediation required",
            outcomes,
        ),
    }
}

fn blocked_decision(
    iteration: usize,
    outcomes: &[WorkflowV2VerificationOutcome],
    blocked: &WorkflowV2VerificationOutcome,
) -> WorkflowV2ConvergenceDecision {
    let mut result = result_from_outcomes(
        WorkflowV2Status::Blocked,
        "verification blocked by external requirement",
        outcomes,
    );
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        blocked.external_blocker.clone().unwrap_or_default(),
    ));
    WorkflowV2ConvergenceDecision {
        status: WorkflowV2ConvergenceStatus::Blocked,
        iteration,
        remediation_items: Vec::new(),
        requires_reverification: false,
        result,
    }
}

fn max_iterations_blocked_decision(
    iteration: usize,
    outcomes: &[WorkflowV2VerificationOutcome],
) -> WorkflowV2ConvergenceDecision {
    let mut result = result_from_outcomes(
        WorkflowV2Status::Blocked,
        "maximum remediation iterations reached",
        outcomes,
    );
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "max_iterations".to_string(),
        description: "verification or review still failed after the configured iteration limit"
            .to_string(),
        severity: Some("blocking".to_string()),
    });
    WorkflowV2ConvergenceDecision {
        status: WorkflowV2ConvergenceStatus::Blocked,
        iteration,
        remediation_items: Vec::new(),
        requires_reverification: false,
        result,
    }
}

fn remediation_item(
    idx: usize,
    outcome: &WorkflowV2VerificationOutcome,
) -> WorkflowV2RemediationItem {
    let mut commands_to_rerun = Vec::new();
    if let Some(command) = &outcome.command {
        commands_to_rerun.push(command.command.clone());
    }
    WorkflowV2RemediationItem {
        id: format!("remediate-{}-{idx}", outcome.task_id),
        task_id: outcome.task_id.clone(),
        source_kind: outcome.kind,
        description: outcome.summary.clone(),
        commands_to_rerun,
    }
}

fn result_from_outcomes(
    status: WorkflowV2Status,
    summary: &str,
    outcomes: &[WorkflowV2VerificationOutcome],
) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status,
        summary: summary.to_string(),
        ..WorkflowV2Result::default()
    };
    for outcome in outcomes {
        if let Some(command) = &outcome.command {
            result.commands_run.push(command.clone());
        }
        result.evidence.extend(outcome.evidence.clone());
        if outcome.kind == WorkflowV2VerificationKind::FocusedTest {
            result.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Test,
                outcome.summary.clone(),
            ));
        }
    }
    result
}

fn looks_like_listing_command(command: &str) -> bool {
    command.split_whitespace().any(|word| {
        word == "list-tests"
            || word.starts_with("--list")
            || word.starts_with("--dry-run")
            || word.starts_with("--collect-only")
    })
}

pub fn test_command(
    command: impl Into<String>,
    status: WorkflowV2CommandStatus,
) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: command.into(),
        status,
        exit_code: None,
        output_summary: String::new(),
    }
}

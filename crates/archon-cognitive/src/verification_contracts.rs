use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CandidateActionKind, CognitiveError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    CodeEdit,
    Commit,
    Push,
    CiDebug,
    CompletionClaim,
    DocsUpdate,
    WorldModelPromotion,
    PipelineForceContinue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractInput {
    pub verification_kind: VerificationKind,
    pub action_kind: CandidateActionKind,
    pub files_touched: Vec<PathBuf>,
    pub commands_planned: Vec<String>,
    pub working_directory: PathBuf,
    pub situation_id: String,
    pub override_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRequirement {
    pub evidence_type: String,
    pub target: String,
    pub acceptance_criteria: String,
    pub required: bool,
    pub fallback_if_unavailable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationContract {
    pub contract_id: String,
    pub situation_id: String,
    pub verification_kind: VerificationKind,
    pub requirements: Vec<VerificationRequirement>,
    pub required_for_completion: bool,
    pub not_run_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    Passed,
    Failed { reason: String },
    Skipped { reason: String },
    NotRun,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationEvidence {
    pub evidence_type: String,
    pub target: String,
    pub passed: Option<bool>,
    pub details: String,
}

#[derive(Debug, Default, Clone)]
pub struct VerificationEngine;

impl VerificationEngine {
    pub fn require(&self, input: &ContractInput) -> Result<VerificationContract, CognitiveError> {
        let requirements = match input.verification_kind {
            VerificationKind::CodeEdit => code_edit_requirements(input)?,
            VerificationKind::Commit => commit_requirements(),
            VerificationKind::Push => vec![req(
                "ref_verify",
                "remote branch/ref",
                "Pushed ref and remote response must be captured.",
                true,
                None,
            )],
            VerificationKind::CiDebug => vec![req(
                "log_evidence",
                "current CI logs",
                "Fetch current CI logs with line references; guessing from memory is prohibited.",
                true,
                Some("CI logs unavailable; diagnosis must be marked provisional."),
            )],
            VerificationKind::CompletionClaim => vec![req(
                "build_pass",
                "workspace build",
                "Successful build/test evidence, or explicit not_run reason, is required.",
                true,
                Some("Record why build/test verification was not run."),
            )],
            VerificationKind::DocsUpdate => docs_requirements(input)?,
            VerificationKind::WorldModelPromotion => world_model_requirements(),
            VerificationKind::PipelineForceContinue => pipeline_override_requirements(input)?,
        };
        Ok(contract(input, requirements))
    }

    pub fn verify(
        &self,
        contract: &VerificationContract,
        working_dir: &Path,
    ) -> VerificationVerdict {
        if contract.requirements.is_empty() {
            return VerificationVerdict::NotRun;
        }
        let evidence = contract
            .requirements
            .iter()
            .filter(|requirement| requirement.evidence_type == "path_exists")
            .map(|requirement| VerificationEvidence {
                evidence_type: requirement.evidence_type.clone(),
                target: requirement.target.clone(),
                passed: Some(working_dir.join(&requirement.target).exists()),
                details: "filesystem path check".into(),
            })
            .collect::<Vec<_>>();
        self.verify_evidence(contract, &evidence)
    }

    pub fn verify_evidence(
        &self,
        contract: &VerificationContract,
        evidence: &[VerificationEvidence],
    ) -> VerificationVerdict {
        if contract.requirements.is_empty() {
            return VerificationVerdict::NotRun;
        }
        let mut skipped = Vec::new();
        for requirement in &contract.requirements {
            match evidence_for(requirement, evidence) {
                Some(found) if found.passed == Some(true) => {}
                Some(found) if found.passed == Some(false) => {
                    return VerificationVerdict::Failed {
                        reason: format!("{} failed: {}", requirement.evidence_type, found.details),
                    };
                }
                Some(_) => return VerificationVerdict::Inconclusive,
                None if requirement.fallback_if_unavailable.is_some() => {
                    skipped.push(requirement.evidence_type.clone());
                }
                None if requirement.required => {
                    return VerificationVerdict::Failed {
                        reason: format!("{} missing", requirement.evidence_type),
                    };
                }
                None => {}
            }
        }
        if skipped.is_empty() {
            VerificationVerdict::Passed
        } else {
            VerificationVerdict::Skipped {
                reason: format!("missing optional fallback evidence: {}", skipped.join(", ")),
            }
        }
    }
}

fn code_edit_requirements(
    input: &ContractInput,
) -> Result<Vec<VerificationRequirement>, CognitiveError> {
    if input.files_touched.is_empty() {
        return Err(CognitiveError::Store(
            "verification contract violated: no files touched".into(),
        ));
    }
    Ok(input
        .files_touched
        .iter()
        .map(|file| {
            req(
                "test_run",
                file.to_string_lossy().as_ref(),
                "Relevant tests pass, or an explicit not_run reason is recorded.",
                true,
                Some("Record not_run reason and proceed with elevated risk."),
            )
        })
        .collect())
}

fn commit_requirements() -> Vec<VerificationRequirement> {
    vec![
        req(
            "git_status",
            "repository working tree",
            "git status output must be captured and reviewed.",
            true,
            None,
        ),
        req(
            "git_diff",
            "staged or unstaged changes",
            "git diff output must be captured and match intended changes.",
            true,
            None,
        ),
    ]
}

fn docs_requirements(
    input: &ContractInput,
) -> Result<Vec<VerificationRequirement>, CognitiveError> {
    if input.files_touched.is_empty() {
        return Err(CognitiveError::Store(
            "verification contract violated: docs path missing".into(),
        ));
    }
    Ok(input
        .files_touched
        .iter()
        .map(|file| {
            req(
                "path_exists",
                file.to_string_lossy().as_ref(),
                "Expected documentation path must exist after the action.",
                true,
                None,
            )
        })
        .collect())
}

fn world_model_requirements() -> Vec<VerificationRequirement> {
    vec![
        req(
            "eval_record",
            "candidate eval",
            "Full eval record must exist.",
            true,
            None,
        ),
        req(
            "gate_verdict",
            "promotion gates",
            "Promotion gates must pass.",
            true,
            None,
        ),
    ]
}

fn pipeline_override_requirements(
    input: &ContractInput,
) -> Result<Vec<VerificationRequirement>, CognitiveError> {
    if input
        .override_reason
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err(CognitiveError::Store(
            "verification contract violated: override reason missing".into(),
        ));
    }
    Ok(vec![req(
        "gate_override",
        "failed quality gate",
        "Failed gate and explicit human override reason must be captured.",
        true,
        None,
    )])
}

fn contract(
    input: &ContractInput,
    requirements: Vec<VerificationRequirement>,
) -> VerificationContract {
    VerificationContract {
        contract_id: Uuid::new_v4().to_string(),
        situation_id: input.situation_id.clone(),
        verification_kind: input.verification_kind,
        requirements,
        required_for_completion: true,
        not_run_reason: None,
    }
}

fn evidence_for<'a>(
    requirement: &VerificationRequirement,
    evidence: &'a [VerificationEvidence],
) -> Option<&'a VerificationEvidence> {
    evidence.iter().find(|item| {
        item.evidence_type == requirement.evidence_type
            && (item.target == requirement.target || item.target == "*")
    })
}

fn req(
    evidence_type: &str,
    target: &str,
    acceptance_criteria: &str,
    required: bool,
    fallback_if_unavailable: Option<&str>,
) -> VerificationRequirement {
    VerificationRequirement {
        evidence_type: evidence_type.into(),
        target: target.into(),
        acceptance_criteria: acceptance_criteria.into(),
        required,
        fallback_if_unavailable: fallback_if_unavailable.map(str::to_string),
    }
}

//! Policy findings separated from portable freeze integrity.

use crate::v2::deliverable_contract::contract_defect;
use crate::verifier_strength::verifier_strength_defect;

use super::{AcceptanceCheck, AcceptanceContract, JudgeDecision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptancePolicyFinding {
    pub field: String,
    pub message: String,
}

pub fn acceptance_policy_findings(contract: &AcceptanceContract) -> Vec<AcceptancePolicyFinding> {
    let mut findings = Vec::new();
    for criterion in contract.acceptance.iter().chain(&contract.supplementary) {
        let before_check = findings.len();
        match &criterion.check {
            AcceptanceCheck::Command { command, .. } => {
                if let Some(defect) = verifier_strength_defect(Some(command), None, None) {
                    findings.push(AcceptancePolicyFinding {
                        field: format!("{}.check", criterion.id),
                        message: format!("check '{}': {defect}", criterion.id),
                    });
                }
            }
            // Structured as exclusive branches rather than early `continue`s:
            // a `continue` here skipped the judgment checks below, so a verdict
            // of `accepted` on a floor that is outright invalid - the most
            // blatant contradiction available - produced no finding at all.
            AcceptanceCheck::Floor { contract } => match serde_json::to_value(contract) {
                Err(error) => findings.push(AcceptancePolicyFinding {
                    field: format!("{}.check", criterion.id),
                    message: format!(
                        "check '{}' floor could not be serialized: {error}",
                        criterion.id
                    ),
                }),
                Ok(value) => {
                    if let Some(defect) = contract_defect(&value) {
                        findings.push(AcceptancePolicyFinding {
                            field: format!("{}.check", criterion.id),
                            message: format!(
                                "check '{}' floor is invalid: {defect}",
                                criterion.id
                            ),
                        });
                    } else if let Some(defect) = verifier_strength_defect(
                        contract.typed_verifier_command.as_deref(),
                        Some(&contract.artifact_path),
                        Some(contract),
                    ) {
                        findings.push(AcceptancePolicyFinding {
                            field: format!("{}.check", criterion.id),
                            message: format!(
                                "check '{}' floor is not falsifiable: {defect}",
                                criterion.id
                            ),
                        });
                    }
                }
            },
        }
        // A verdict cannot outrank a defect the host can check for itself. The
        // judge is prose validated only for non-emptiness, so an `accepted`
        // standing on a check the policy layer independently reports is a
        // contradiction, not evidence - and it is exactly the disagreement that
        // makes a rubber-stamp judge look like a passing gate.
        if findings.len() > before_check && criterion.judgment.verdict == JudgeDecision::Accepted {
            findings.push(AcceptancePolicyFinding {
                field: format!("{}.judgment", criterion.id),
                message: format!(
                    "check '{}' was accepted by the judge while the same check carries a machine-checkable defect reported above; the verdict contradicts a finding the host verified",
                    criterion.id
                ),
            });
        }
        if criterion.judgment.verdict != JudgeDecision::Accepted {
            findings.push(AcceptancePolicyFinding {
                field: format!("{}.judgment", criterion.id),
                message: format!(
                    "check '{}' was refuted by the host judge; replace the check with one the judge cannot falsify",
                    criterion.id
                ),
            });
        }
    }
    findings
}

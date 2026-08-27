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
        match &criterion.check {
            AcceptanceCheck::Command { command, .. } => {
                if let Some(defect) = verifier_strength_defect(Some(command), None, None) {
                    findings.push(AcceptancePolicyFinding {
                        field: format!("{}.check", criterion.id),
                        message: format!("check '{}': {defect}", criterion.id),
                    });
                }
            }
            AcceptanceCheck::Floor { contract } => {
                let value = match serde_json::to_value(contract) {
                    Ok(value) => value,
                    Err(error) => {
                        findings.push(AcceptancePolicyFinding {
                            field: format!("{}.check", criterion.id),
                            message: format!(
                                "check '{}' floor could not be serialized: {error}",
                                criterion.id
                            ),
                        });
                        continue;
                    }
                };
                if let Some(defect) = contract_defect(&value) {
                    findings.push(AcceptancePolicyFinding {
                        field: format!("{}.check", criterion.id),
                        message: format!("check '{}' floor is invalid: {defect}", criterion.id),
                    });
                    continue;
                }
                if let Some(defect) = verifier_strength_defect(
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

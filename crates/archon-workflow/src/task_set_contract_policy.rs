//! Policy findings separated from portable freeze integrity.

use crate::verifier_strength::{acceptance_verifier_strength_defect, verifier_strength_defect};

use super::{AcceptanceCheck, AcceptanceContract, JudgeDecision};

/// Identifiers a PRD uses when it prescribes an acceptance check's SHAPE
/// rather than its outcome: the deliverable-contract fields that decide what a
/// floor targets and whether it can fail, plus the phrase for a floor with no
/// command. These are the engine's own vocabulary, not any PRD's.
///
/// A criterion written in outcome language ("`status` shows the data root")
/// leaves the shape to the author, so a policy finding against that shape is
/// the author's to repair. A criterion that names these fields has fixed the
/// shape itself, so the same finding is an observation about the input and
/// re-authoring would only make the author violate the PRD.
pub const CHECK_SHAPE_VOCABULARY: [&str; 6] = [
    "typed_verifier_command",
    "artifact_path",
    "artifact_format",
    "required_true_fields",
    "min_instances",
    "commandless floor",
];

/// Whether a PRD criterion prescribes its check's shape in the engine's own
/// contract vocabulary.
pub fn criterion_prescribes_check_shape(criterion: &str) -> bool {
    CHECK_SHAPE_VOCABULARY
        .iter()
        .any(|token| criterion.contains(token))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptancePolicyFinding {
    pub identity: Option<crate::defect::DeterministicDefect>,
    pub field: String,
    pub message: String,
}

pub fn acceptance_policy_findings(contract: &AcceptanceContract) -> Vec<AcceptancePolicyFinding> {
    let mut findings = Vec::new();
    for (index, criterion) in contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .enumerate()
    {
        let slot = format!("checks/{index}");
        let before_check = findings.len();
        let mut check_defects = Vec::new();
        match &criterion.check {
            AcceptanceCheck::Command { command, .. } => {
                if let Some(defect) = verifier_strength_defect(Some(command), None, None) {
                    check_defects.push(crate::defect::ValidationDefect::new(
                        strength_code(&defect),
                        &slot,
                        "check",
                        defect.to_string(),
                    ));
                }
            }
            AcceptanceCheck::Floor { contract } => {
                match serde_json::to_value(contract) {
                    Err(error) => check_defects.push(crate::defect::ValidationDefect::new(
                        "invalid_floor_serialization",
                        &slot,
                        "check",
                        error.to_string(),
                    )),
                    Ok(value) => {
                        check_defects.extend(
                            crate::v2::deliverable_contract::contract_defects(&value)
                                .into_iter()
                                .map(|mut defect| {
                                    defect.identity.subject.clone_from(&slot);
                                    defect
                                }),
                        );
                    }
                }
                // Independent from path shape: reporting both lets the author
                // repair every outstanding defect in this attempt.
                if let Some(defect) = acceptance_verifier_strength_defect(
                    contract.typed_verifier_command.as_deref(),
                    Some(&contract.artifact_path),
                ) {
                    check_defects.push(crate::defect::ValidationDefect::new(
                        strength_code(&defect), &slot, "check/typed_verifier_command", format!(
                            "floor is not falsifiable: {defect}; acceptance floors require typed_verifier_command that exercises the deliverable (instance counts and asserted fields alone are insufficient)")));
                }
            }
        }
        findings.extend(
            check_defects
                .into_iter()
                .map(|defect| AcceptancePolicyFinding {
                    identity: Some(defect.identity),
                    field: format!("{}.check", criterion.id),
                    message: format!("check '{}': {}", criterion.id, defect.message),
                }),
        );
        if findings.len() > before_check && criterion.judgment.verdict == JudgeDecision::Accepted {
            findings.push(AcceptancePolicyFinding {
                identity: Some(crate::defect::DeterministicDefect::new("contradictory_judgment", &slot, "judgment")),
                field: format!("{}.judgment", criterion.id),
                message: format!("check '{}' was accepted by the judge while the same check carries a machine-checkable defect reported above; the verdict contradicts a finding the host verified", criterion.id),
            });
        }
        if criterion.judgment.verdict != JudgeDecision::Accepted {
            findings.push(AcceptancePolicyFinding {
                identity: None,
                field: format!("{}.judgment", criterion.id),
                message: refuted_check_message(
                    &criterion.id,
                    &criterion.judgment.reason,
                    &criterion.judgment.counterexample,
                ),
            });
        }
    }
    findings
}

fn strength_code(defect: &crate::verifier_strength::VerifierStrengthDefect) -> &'static str {
    use crate::verifier_strength::VerifierStrengthDefect::*;
    match defect {
        MissingExecutionObligation => "missing_execution_obligation",
        OwnArtifactExistenceOnly { .. } => "existence_only_verifier",
        FixedSuccessProgram { .. } => "fixed_success_program",
        FixedSuccessFallback { .. } => "fixed_success_fallback",
    }
}

/// The finding an author repairs against when the judge refuted a check. The
/// judge's reason and counterexample travel inside the finding text because the
/// finding text is the only thing the repair prompt carries: an author told just
/// "refuted" can only guess at what to change, and a live acceptance phase spent
/// its whole attempt budget guessing. Both fields are flattened to one line so a
/// judge that ignored the single-line rule cannot break the finding list layout.
pub fn refuted_check_message(id: &str, reason: &str, counterexample: &str) -> String {
    format!(
        "check '{id}' was refuted by the host judge; reason: \"{}\"; counterexample: \"{}\"; replace the check with one that fails in that state",
        judge_prose(reason, "(judge gave no reason)"),
        judge_prose(counterexample, "(judge gave no counterexample)"),
    )
}

/// Judge prose is model output: flattened to one line so it cannot break the
/// finding list, quoted so the author reads it as evidence rather than as a
/// host instruction, and capped because the repair prompt repeats every earlier
/// attempt's findings verbatim and would otherwise grow without bound.
const JUDGE_PROSE_CAP: usize = 400;

fn judge_prose(text: &str, fallback: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return fallback.to_string();
    }
    match flat.char_indices().nth(JUDGE_PROSE_CAP) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}

#[cfg(test)]
#[path = "task_set_contract_policy_tests.rs"]
mod tests;

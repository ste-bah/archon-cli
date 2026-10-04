//! Complete deterministic acceptance structure diagnostics with stable identities.
use super::*;
use crate::defect::{ValidationDefect, defect_message};

pub fn acceptance_structure_defects(
    contract: &AcceptanceContract,
    expected: &BTreeSet<String>,
    require_judgments: bool,
) -> Vec<ValidationDefect> {
    let mut defects = Vec::new();
    if contract.schema_version != 1 {
        defects.push(ValidationDefect::new(
            "invalid_schema_version",
            "acceptance",
            "schema_version",
            format!(
                "acceptance contract schema_version must be 1, found {}",
                contract.schema_version
            ),
        ));
    }
    if contract.acceptance.is_empty() {
        defects.push(ValidationDefect::new(
            "empty_acceptance",
            "acceptance",
            "acceptance",
            "acceptance contract examined zero acceptance checks; this is not a pass",
        ));
    }
    for (index, id) in contract
        .gap_policy
        .permitted_acceptance_ids
        .difference(expected)
        .enumerate()
    {
        defects.push(ValidationDefect::new("unknown_gap_permission", "acceptance", &format!("gap_policy/{index}"),
            format!("gap_policy.permitted_acceptance_ids contains unknown ids {id}; remove each unknown id or correct it to one defined by the PRD")));
    }
    let mut seen = BTreeSet::new();
    for (index, entry) in contract.acceptance.iter().enumerate() {
        let subject = if expected.contains(&entry.id) {
            entry.id.clone()
        } else {
            format!("acceptance/{index}")
        };
        if !expected.contains(&entry.id) {
            defects.push(ValidationDefect::new(
                "unknown_acceptance_id",
                &subject,
                "id",
                format!(
                    "acceptance id '{}' is not defined by the PRD; remove it or correct the id",
                    entry.id
                ),
            ));
        }
        if !seen.insert(entry.id.clone()) {
            defects.push(ValidationDefect::new(
                "duplicate_acceptance_id",
                &subject,
                &format!("acceptance/{index}/id"),
                format!(
                    "acceptance id '{}' appears more than once; keep exactly one check for this id",
                    entry.id
                ),
            ));
        }
        criterion_defects(entry, &subject, require_judgments, &mut defects);
        if entry.gap_permitted
            != contract
                .gap_policy
                .permitted_acceptance_ids
                .contains(&entry.id)
        {
            defects.push(ValidationDefect::new("gap_permission_mismatch", &subject, "gap_permitted", format!(
                "acceptance id '{}' gap_permitted disagrees with gap_policy; make both declarations match", entry.id)));
        }
    }
    for id in expected.difference(&seen) {
        defects.push(ValidationDefect::new("missing_acceptance_check", id, "acceptance", format!(
            "acceptance contract is missing checks for {id}; keep every check already present and add one check for each id listed, so every PRD acceptance id has its own check")));
    }
    let mut supplementary = BTreeSet::new();
    for (index, entry) in contract.supplementary.iter().enumerate() {
        let subject = format!("supplementary/{index}");
        if !entry.id.starts_with("SUP-") {
            defects.push(ValidationDefect::new(
                "invalid_supplementary_id",
                &subject,
                "id",
                format!(
                    "supplementary check '{}' must use a distinct SUP-* id",
                    entry.id
                ),
            ));
        }
        if expected.contains(&entry.id) || !supplementary.insert(entry.id.clone()) {
            defects.push(ValidationDefect::new("duplicate_supplementary_id", &subject, "id", format!(
                "supplementary check '{}' collides with an acceptance or supplementary id; choose a distinct SUP-* id", entry.id)));
        }
        if entry.gap_permitted {
            defects.push(ValidationDefect::new("supplementary_gap_permission", &subject, "gap_permitted", format!(
                "supplementary check '{}' cannot be covered by a residual gap; set gap_permitted to false", entry.id)));
        }
        criterion_defects(entry, &subject, require_judgments, &mut defects);
    }
    defects
}

fn criterion_defects(
    entry: &AcceptanceCriterion,
    subject: &str,
    judgments: bool,
    defects: &mut Vec<ValidationDefect>,
) {
    if entry.criterion.trim().is_empty() {
        defects.push(ValidationDefect::new(
            "empty_criterion",
            subject,
            "criterion",
            format!(
                "check '{}' has empty criterion text; copy the exact criterion text",
                entry.id
            ),
        ));
    }
    if let AcceptanceCheck::Floor { contract } = &entry.check
        && let Err(error) = serde_json::to_value(contract)
    {
        defects.push(ValidationDefect::new(
            "invalid_floor_serialization",
            subject,
            "check",
            error.to_string(),
        ));
    }
    if judgments {
        for (field, value) in [
            ("counterexample", &entry.judgment.counterexample),
            ("reason", &entry.judgment.reason),
            ("host_call_id", &entry.judgment.host_call_id),
        ] {
            if value.trim().is_empty() {
                defects.push(ValidationDefect::new("empty_judgment_field", subject, &format!("judgment/{field}"), format!(
                    "check '{}' judgment field '{field}' is empty; re-run freeze-acceptance so the host judge records it", entry.id)));
            }
        }
    }
}

pub fn validate_acceptance_structure(
    contract: &AcceptanceContract,
    expected: &BTreeSet<String>,
    require_judgments: bool,
) -> ContractResult<()> {
    let defects = acceptance_structure_defects(contract, expected, require_judgments);
    if defects.is_empty() {
        return Ok(());
    }
    Err(TaskSetContractError {
        message: defect_message(&defects),
        defects,
    })
}

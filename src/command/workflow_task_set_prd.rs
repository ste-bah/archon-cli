use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::obligation_ids::{
    acceptance_criteria, duplicate_obligation_finding, duplicate_obligation_ids,
    malformed_obligation_finding, malformed_obligation_ids,
};
use archon_workflow::task_set_contract::content_digest;

/// Add current PRD requirement text to the in-memory script arguments. This
/// field is runtime input only; callers keep the launch-bound arguments intact.
pub(crate) fn script_arguments_with_prd_requirement_texts(
    arguments: &serde_json::Value,
    prd_text: &str,
) -> serde_json::Value {
    let mut script_arguments = arguments.clone();
    script_arguments["deliverableContractRule"] =
        serde_json::json!(archon_workflow::task_universe_contract_audit::DELIVERABLE_CONTRACT_RULE);
    script_arguments["prdRequirementTexts"] = serde_json::json!(
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text)
    );
    script_arguments["prdObligationIds"] =
        serde_json::json!(archon_workflow::obligation_ids::obligation_ids(prd_text));
    script_arguments["prdConstraintTexts"] =
        serde_json::json!(archon_workflow::obligation_ids::constraint_texts(prd_text));
    script_arguments["prdSections"] =
        serde_json::json!(archon_workflow::obligation_ids::numbered_sections(prd_text));
    script_arguments
}

pub(crate) fn validate_prd_input(
    prd_path: &Path,
) -> Result<(Vec<u8>, String, BTreeMap<String, String>)> {
    let bytes = std::fs::read(prd_path)
        .with_context(|| format!("reading PRD at {}", prd_path.display()))?;
    let text = String::from_utf8(bytes.clone()).context("PRD is not UTF-8")?;
    let malformed = malformed_obligation_ids(&text);
    if !malformed.is_empty() {
        return Err(anyhow!(
            "{}",
            malformed
                .iter()
                .map(|id| malformed_obligation_finding(id))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    let duplicates = duplicate_obligation_ids(&text);
    if !duplicates.is_empty() {
        return Err(anyhow!(
            "{}",
            duplicates
                .iter()
                .map(|id| duplicate_obligation_finding(id))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    if archon_workflow::obligation_ids::obligation_ids(&text).is_empty() {
        return Err(anyhow!(
            "PRD {} defines zero obligations; add canonical REQ/AC obligation rows before decomposition",
            prd_path.display()
        ));
    }
    let criteria = acceptance_criteria(&text);
    if criteria.is_empty() {
        return Err(anyhow!(
            "PRD {} defines no acceptance IDs; add an acceptance/obligation table before freezing",
            prd_path.display()
        ));
    }
    Ok((bytes.clone(), content_digest(&bytes), criteria))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_arguments_include_the_exact_obligation_union_without_changing_launch_arguments() {
        let prd = "- REQ-X-001: work\n- QX-001: constraint\n\n| ID | Goal |\n| --- | --- |\n| G-X-001 | goal |\n\n## Done Definition\n1. Complete.\n";
        let arguments = serde_json::json!({"prdPath": "prd.md"});
        let enriched = script_arguments_with_prd_requirement_texts(&arguments, prd);
        assert_eq!(
            enriched["prdObligationIds"],
            serde_json::json!(["DONE-1", "G-X-001", "REQ-X-001"])
        );
        assert_eq!(
            enriched["prdRequirementTexts"],
            serde_json::json!({"REQ-X-001": "work", "QX-001": "constraint", "G-X-001": "goal", "DONE-1": "Complete."})
        );
        assert_eq!(
            enriched["prdConstraintTexts"],
            serde_json::json!({"QX-001": "constraint"})
        );
        assert_eq!(
            enriched["deliverableContractRule"],
            archon_workflow::task_universe_contract_audit::DELIVERABLE_CONTRACT_RULE
        );
        assert_eq!(arguments, serde_json::json!({"prdPath": "prd.md"}));
    }
    #[test]
    fn section_host_map_keeps_own_text_and_parent_citation_status() {
        let prd = "## 7. Widgets\n### 7.1 Small\nRequired shapes.\n";
        let args = script_arguments_with_prd_requirement_texts(&serde_json::json!({}), prd);
        assert_eq!(args["prdSections"]["7"]["requires_citation"], false);
        assert_eq!(
            args["prdSections"]["7.1"]["text"],
            "### 7.1 Small\nRequired shapes."
        );
        assert_eq!(args["prdConstraintTexts"], serde_json::json!({}));
    }
}

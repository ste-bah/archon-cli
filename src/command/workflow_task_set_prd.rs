use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::obligation_ids::{
    acceptance_criteria, duplicate_obligation_finding, duplicate_obligation_ids,
    malformed_obligation_finding, malformed_obligation_ids,
};
use archon_workflow::task_set_contract::content_digest;

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

//! Candidate-only validation before any acceptance judge request.
use super::*;

pub(super) fn prepare(
    project_root: &Path,
    prd_path: &Path,
    prd_digest: &str,
    prd_text: &str,
    exact_criteria: &std::collections::BTreeMap<String, String>,
    expected: &BTreeSet<String>,
    original: &[u8],
) -> Result<AcceptanceContract> {
    // The candidate is the author's artifact from stdin, not the contract on
    // disk. Everything up to the judge inspects that artifact, so a failure
    // here is tagged as the artifact's and fed back to the author.
    let mut contract: AcceptanceContract = CandidateRejected::tag(
        serde_json::from_slice(original).context("parsing the candidate acceptance contract"),
    )?;
    contract.prd.path = project_relative(project_root, prd_path);
    contract.prd.digest = prd_digest.to_string();
    contract.gap_policy.forbidden_phrases = residual_gap_forbidden_phrases(&prd_text);
    contract.gap_policy.required_fields = REQUIRED_RESIDUAL_GAP_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    for criterion in &mut contract.acceptance {
        criterion.criterion =
            CandidateRejected::tag(exact_criteria.get(&criterion.id).cloned().ok_or_else(|| {
                anyhow!(
                    "acceptance id '{}' is not defined by the PRD; remove it or correct the id",
                    criterion.id
                )
            }))?;
    }
    CandidateRejected::tag(
        validate_acceptance_structure(&contract, &expected, false).map_err(anyhow::Error::new),
    )?;

    // Authored verdicts are placeholders. Only check defects, never those
    // placeholder judgments, decide whether the candidate reaches the judge.
    let defects = acceptance_policy_findings(&contract)
        .into_iter()
        .filter(|finding| {
            finding.field.ends_with(".check")
                && !contract
                    .acceptance
                    .iter()
                    .chain(&contract.supplementary)
                    .any(|entry| {
                        finding.field == format!("{}.check", entry.id)
                            && criterion_prescribes_check_shape(&entry.criterion)
                    })
        })
        .map(|finding| finding.message)
        .collect::<Vec<_>>();
    if !defects.is_empty() {
        return CandidateRejected::tag(Err(anyhow!("{}", defects.join("; "))));
    }
    Ok(contract)
}

//! Candidate-only validation before any acceptance judge request.
use super::*;
use crate::command::workflow_task_set_candidate::CandidateDefects;
use archon_workflow::defect::ValidationDefect;

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
        crate::command::workflow_freeze_candidate::acceptance_candidate_for_validation(original)
            .and_then(|bytes| {
                serde_json::from_slice(&bytes).context("parsing the candidate acceptance contract")
            }),
    )?;
    contract.prd.path = project_relative(project_root, prd_path);
    contract.prd.digest = prd_digest.to_string();
    contract.gap_policy.forbidden_phrases = residual_gap_forbidden_phrases(prd_text);
    contract.gap_policy.required_fields = REQUIRED_RESIDUAL_GAP_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    let marker_value: serde_json::Value = serde_json::from_slice(
        &crate::command::workflow_freeze_candidate::candidate_document(original),
    )
    .context("acceptance marker inspection")?;
    let mut defects =
        crate::command::workflow_freeze_candidate::marker_defects(&marker_value, false);
    for (index, criterion) in contract.acceptance.iter_mut().enumerate() {
        if let Some(text) = exact_criteria.get(&criterion.id) {
            criterion.criterion.clone_from(text);
        } else {
            // Structure validation below reports every unknown id. Continue
            // stamping the known siblings so their independent defects are visible.
            criterion.criterion = format!("unknown acceptance entry {index}");
        }
    }
    // H4: a supplementary check minted for a requirement no check covered
    // is judged against that requirement's exact text and always covers it;
    // both are the host's, like an acceptance entry's criterion.
    let requirements =
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    for (index, criterion) in contract.supplementary.iter_mut().enumerate() {
        let Some(requirement) =
            archon_workflow::v2::acceptance_stage::coverage::supplementary_requirement(
                &criterion.id,
            )
        else {
            continue;
        };
        let Some(text) = requirements.get(requirement).cloned() else {
            defects.push(ValidationDefect::new("unknown_supplementary_requirement", "acceptance",
                &format!("supplementary/{index}"), format!(
                    "supplementary check '{}' names requirement {requirement}, which the PRD does not define; remove it or correct the id", criterion.id)));
            continue;
        };
        criterion.criterion = text;
        if !criterion.covers.iter().any(|id| id == requirement) {
            criterion.covers.insert(0, requirement.to_string());
        }
    }
    defects.extend(
        archon_workflow::task_set_contract::acceptance_structure_defects(
            &contract, expected, false,
        ),
    );

    // Authored verdicts are placeholders. Only check defects, never those
    // placeholder judgments, decide whether the candidate reaches the judge.
    let policy_defects = acceptance_policy_findings(&contract)
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
        .filter_map(|finding| {
            finding.identity.map(|identity| ValidationDefect {
                identity,
                message: finding.message,
            })
        })
        .collect::<Vec<_>>();
    defects.extend(policy_defects);
    if !defects.is_empty() {
        return CandidateRejected::tag(Err(CandidateDefects(defects).into()));
    }
    Ok(contract)
}

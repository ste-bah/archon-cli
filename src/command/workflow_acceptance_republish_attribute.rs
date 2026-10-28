//! D4: a gate finding that one re-authored candidate causes goes back to
//! that candidate, pending, and its siblings are published.
//!
//! Under enforce, any finding the acceptance gate records refuses the whole
//! republish, and so does the skeleton gate under enforce: it records a
//! predecessor finding whenever the acceptance gate it stands on recorded
//! one (`predecessor_findings`); the skeleton itself is unchanged by a
//! per-check repair. So a candidate's effect on both gates is the
//! acceptance findings that name its entry, which only that candidate
//! changed. Each candidate with such a finding is held back -- its frozen
//! entry kept, its saved candidate forgotten, the finding its author's --
//! before anything is published. A finding no candidate causes (the PRD's
//! own) still refuses the republish, as before.

use archon_workflow::RemediationScope;

use super::*;

/// Hold back from `publish_ids` (restoring each held entry in
/// `progress.contract` from `verified`) every candidate an enforced gate
/// would refuse for a finding that names it.
pub(super) fn hold_back(
    request: &ReauthorRequest<'_>,
    verified: &verify::Verified,
    progress: &mut reauthor::ReauthorProgress,
    publish_ids: &mut BTreeSet<String>,
) -> Result<()> {
    let enforced = verified.acceptance_mode == FreezeGateMode::Enforce
        || (verified.skeleton.is_some() && verified.skeleton_mode == FreezeGateMode::Enforce);
    if !enforced {
        return Ok(());
    }
    let contract_path = request.tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let findings = findings::acceptance_findings(
        request.project_root,
        request.prd_path,
        &verified.prd_text,
        &contract_path,
        &progress.contract,
    )?;
    // The PRD's own finding refuses the republish whatever is held back.
    if (findings.iter()).any(|finding| finding.remediation_scope == RemediationScope::PrdInput) {
        return Ok(());
    }
    let held: Vec<(String, String)> = (publish_ids.iter())
        .filter_map(|id| {
            let texts: Vec<&str> = (findings.iter())
                .filter(|finding| &finding.subject == id)
                .map(|finding| finding.text.as_str())
                .collect();
            (!texts.is_empty()).then(|| {
                let finding = format!(
                    "check '{id}': the gates of this chain, frozen under enforce, refuse this re-authored check, so it was not published with the rest: {}; repair it so the gates record no finding for it",
                    texts.join("; ")
                );
                (id.clone(), finding)
            })
        })
        .collect();
    for (id, finding) in held {
        let frozen = (verified.contract.acceptance.iter())
            .chain(&verified.contract.supplementary)
            .find(|entry| entry.id == id)
            .expect("a candidate replaces a frozen entry")
            .clone();
        if let Some(entry) = (progress.contract.acceptance.iter_mut())
            .chain(&mut progress.contract.supplementary)
            .find(|entry| entry.id == id)
        {
            *entry = frozen;
        }
        reauthor::cache::forget(request.tasks_root, &id)
            .with_context(|| format!("forgetting the saved candidate of {id}"))?;
        publish_ids.remove(&id);
        progress.unproven.insert(id.clone(), finding.clone());
        progress.authored_pending.insert(id, finding);
    }
    Ok(())
}

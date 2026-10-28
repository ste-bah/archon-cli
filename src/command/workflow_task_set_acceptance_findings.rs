use super::*;

/// Every finding the acceptance gate records for `contract` against the PRD.
pub(in crate::command::workflow_task_set) fn acceptance_findings(
    project_root: &Path,
    prd_path: &Path,
    prd_text: &str,
    contract_path: &Path,
    contract: &AcceptanceContract,
) -> Result<Vec<GateFinding>> {
    let mut findings = malformed_obligation_ids(prd_text)
        .into_iter()
        .map(|id| {
            GateFinding::new(
                GateId::FreezeAcceptance,
                malformed_obligation_finding(&id),
                id,
                Some(prd_path.to_path_buf()),
                archon_workflow::RemediationScope::PrdInput,
            )
        })
        .collect::<Vec<_>>();
    findings.extend(duplicate_obligation_ids(prd_text).into_iter().map(|id| {
        GateFinding::new(
            GateId::FreezeAcceptance,
            duplicate_obligation_finding(&id),
            id,
            Some(prd_path.to_path_buf()),
            archon_workflow::RemediationScope::PrdInput,
        )
    }));
    findings.extend(
        preflight::rooted_policy_findings(
            contract,
            project_root,
            contract_path.parent().expect("contract directory"),
        )?
        .into_iter()
        .map(|finding| {
            let subject = finding
                .field
                .split('.')
                .next()
                .unwrap_or("acceptance-contract");
            let entry = contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .find(|criterion| criterion.id == subject);
            // Repairable unless the PRD itself prescribed the check's shape
            // (TD-015): outcome language leaves the shape to the author, so a
            // floor that cannot fail goes back with the finding; a criterion
            // naming the contract fields has fixed the shape, so that
            // finding is recorded, never retried, never blocking in observe.
            // Every check still needs its paths in the site.
            let prescribed = entry
                .is_some_and(|criterion| criterion_prescribes_check_shape(&criterion.criterion));
            let unavailable_site_path = finding
                .identity
                .as_ref()
                .is_some_and(|identity| identity.code == "unavailable_site_path");
            // A judge verdict other than `accepted` is never an observation:
            // such a check can never run, so it is never published, and the
            // only way forward is the author's. It goes back to the author
            // whatever the criterion prescribes.
            let refuted = finding.field.ends_with(".judgment")
                && entry
                    .is_some_and(|criterion| criterion.judgment.verdict != JudgeDecision::Accepted);
            let scope = if prescribed && !refuted && !unavailable_site_path {
                archon_workflow::RemediationScope::InheritedPredecessor
            } else {
                archon_workflow::RemediationScope::CandidateArtifact
            };
            let mut gate_finding = GateFinding::new(
                GateId::FreezeAcceptance,
                finding.message,
                subject,
                Some(contract_path.to_path_buf()),
                scope,
            );
            gate_finding.deterministic_defect = finding.identity;
            gate_finding
        }),
    );
    Ok(findings)
}

//! Findings recorded by the skeleton gate.

use super::*;

/// Every finding the skeleton gate records for `skeleton` under `pin`.
pub(crate) fn skeleton_findings(
    tasks_root: &Path,
    canonical_prd: &Path,
    prd_text: &str,
    pin: &AcceptancePin,
    pin_path: &Path,
    skeleton: &TaskSkeleton,
) -> Result<Vec<GateFinding>> {
    let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
    let mut findings = malformed_obligation_ids(prd_text)
        .into_iter()
        .map(|id| {
            GateFinding::new(
                GateId::FreezeSkeleton,
                malformed_obligation_finding(&id),
                id,
                Some(canonical_prd.to_path_buf()),
                archon_workflow::RemediationScope::PrdInput,
            )
        })
        .collect::<Vec<_>>();
    findings.extend(duplicate_obligation_ids(prd_text).into_iter().map(|id| {
        GateFinding::new(
            GateId::FreezeSkeleton,
            duplicate_obligation_finding(&id),
            id,
            Some(canonical_prd.to_path_buf()),
            archon_workflow::RemediationScope::PrdInput,
        )
    }));
    predecessor_findings(pin, pin_path, &mut findings);
    let expected_obligations = archon_workflow::obligation_ids::obligation_ids(prd_text);
    let constraints = archon_workflow::obligation_ids::constraint_ids(prd_text);
    let allowed = expected_obligations.union(&constraints).cloned().collect();
    findings.extend(
        validate_skeleton_set(skeleton, &allowed)
            .into_iter()
            .filter(|finding| {
                !(finding.identity.code == "unowned_obligation"
                    && constraints.contains(&finding.identity.subject))
            })
            .map(|finding| {
                // This rule's message is shared verbatim with body and set gates;
                // its structured field still identifies the offending claim.
                let text = if finding.identity.code == "deliverable_implements_unclaimed" {
                    finding.message
                } else {
                    format!("{}: {}", finding.field, finding.message)
                };
                GateFinding::new(
                    GateId::FreezeSkeleton,
                    text,
                    finding.field,
                    Some(skeleton_path.clone()),
                    archon_workflow::RemediationScope::Skeleton,
                )
                .with_defect(finding.identity)
            }),
    );
    findings.extend(
        archon_workflow::obligation_ids::section_citation_findings(
            prd_text,
            skeleton
                .tasks
                .iter()
                .map(|task| (task.task_id.as_str(), task.source_sections.as_slice())),
        )
        .into_iter()
        .map(|finding| {
            GateFinding::new(
                GateId::FreezeSkeleton,
                finding.message,
                finding.identity.subject.clone(),
                Some(skeleton_path.clone()),
                archon_workflow::RemediationScope::Skeleton,
            )
            .with_defect(finding.identity)
        }),
    );
    findings.extend(
        archon_workflow::task_skeleton::constraint_owner_findings(
            skeleton,
            &archon_workflow::obligation_ids::constraint_texts(prd_text),
        )
        .into_iter()
        .map(|finding| {
            GateFinding::new(
                GateId::FreezeSkeleton,
                finding.message,
                finding.identity.subject.clone(),
                Some(skeleton_path.clone()),
                archon_workflow::RemediationScope::Skeleton,
            )
            .with_defect(finding.identity)
        }),
    );
    let edge_analysis = analyze_task_set_edges(skeleton);
    findings.extend(edge_analysis.blockers.into_iter().map(|finding| {
        GateFinding::new(
            GateId::FreezeSkeleton,
            format!("{}: {}", finding.field, finding.message),
            finding.field,
            Some(skeleton_path.clone()),
            archon_workflow::RemediationScope::Skeleton,
        )
        .with_defect(finding.identity)
    }));
    // Issue-55: every repository file the PRD names must have an owning
    // task; the skeleton author assigns it on the retry this finding drives.
    findings.extend(
        crate::command::topology_lint::skeleton_owner_defects(tasks_root, prd_text, skeleton)?
            .into_iter()
            .map(|defect| {
                GateFinding::new(
                    GateId::FreezeSkeleton,
                    defect.message,
                    "deliverable_contracts",
                    Some(skeleton_path.clone()),
                    archon_workflow::RemediationScope::Skeleton,
                )
                .with_defect(defect.identity)
            }),
    );
    Ok(findings)
}

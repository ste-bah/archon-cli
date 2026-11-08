use super::*;

/// Requirements the PRD defines that no task claims, for a caller that blocks.
///
/// # Why this blocks rather than warns
///
/// A requirement nobody claims is work nobody does, and nothing downstream ever
/// notices: the run finishes, every task it knows about passes, and the feature
/// simply is not there. Observed live — a decomposition numbered its tasks
/// `010, 020, 040, 050`, left the `030` slot empty, and never wrote the Pine
/// artifacts task at all. Four requirements were orphaned by that one omission
/// and the lint reported them into a summary that exited zero.
///
/// It is a fact, not a judgement: the ids come from the PRD's own bullets and
/// the claims from the tasks' own `implements:`. Nothing here knows what a
/// requirement means, so it holds for any PRD in any domain.
pub(in super::super) fn policy_findings(tasks_root: Option<&Path>) -> Vec<CoveragePolicyFinding> {
    let Some(root) = tasks_root else {
        return Vec::new();
    };
    let Ok((mut claims, skipped)) = task_requirement_claims_tolerant(root) else {
        return Vec::new();
    };
    let prd_path = match resolve_prd(root, &claims) {
        Ok(Some(prd_path)) => prd_path,
        Ok(None) => return Vec::new(),
        // The frozen contract names a PRD that no longer digests as frozen.
        // That is a fact about the inputs, not a lint judgement, and it must
        // block: auditing against a drifted PRD would look like a pass.
        Err(error) => {
            return vec![CoveragePolicyFinding {
                text: format!("{error:#}"),
                subject: ACCEPTANCE_CONTRACT_FILE.to_string(),
                source_path: root.join(ACCEPTANCE_CONTRACT_FILE),
                identity: None,
                remediation_scope: archon_workflow::RemediationScope::PrdInput,
            }];
        }
    };
    let Ok(prd) = std::fs::read_to_string(&prd_path) else {
        return Vec::new();
    };
    let constraints = archon_workflow::obligation_ids::constraint_ids(&prd);
    let mut defined = obligation_ids(&prd);
    defined.extend(constraints.iter().cloned());
    let skeleton = std::fs::read(root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE))
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<archon_workflow::task_skeleton::TaskSkeleton>(&bytes).ok()
        });
    if skeleton.is_none()
        && root
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE)
            .exists()
    {
        // An unreadable skeleton has its own finding, not body-derived coverage.
        return Vec::new();
    }
    if let Some(skeleton) = &skeleton {
        claims = skeleton
            .tasks
            .iter()
            .map(|task| TaskRequirementClaims {
                task_id: task.task_id.clone(),
                source_path: root.join(&task.file_name).display().to_string(),
                implements: task.implements.clone(),
                source_sections: task.source_sections.clone(),
            })
            .collect();
    }
    // Missing parsed claims cannot prove a set-level coverage gap; defects
    // directly evidenced by a parsed body or the PRD still apply.
    let partial = skeleton.is_none() && !skipped.is_empty();
    let claimed = claimed_by_task(&claims);
    let mut findings = archon_workflow::obligation_ids::malformed_obligation_ids(&prd)
        .into_iter()
        .map(|id| CoveragePolicyFinding {
            text: archon_workflow::obligation_ids::malformed_obligation_finding(&id),
            subject: id,
            source_path: prd_path.clone(),
            identity: None,
            remediation_scope: archon_workflow::RemediationScope::PrdInput,
        })
        .collect::<Vec<_>>();
    findings.extend(
        archon_workflow::obligation_ids::duplicate_obligation_ids(&prd)
            .into_iter()
            .map(|id| CoveragePolicyFinding {
                text: archon_workflow::obligation_ids::duplicate_obligation_finding(&id),
                subject: id,
                source_path: prd_path.clone(),
                identity: None,
                remediation_scope: archon_workflow::RemediationScope::PrdInput,
            }),
    );
    findings.extend(
        defined
            .iter()
            .filter(|id| !partial && !claimed.contains_key(*id) && !constraints.contains(*id))
            .map(|id| CoveragePolicyFinding {
                text: format!(
                    "{id}: defined in the PRD but claimed by no task — add it to the implements of at least one task in the skeleton"
                ),
                subject: id.clone(),
                source_path: prd_path.clone(),
                identity: None,
                remediation_scope: archon_workflow::RemediationScope::Skeleton,
            }),
    );
    let owners = skeleton
        .clone()
        .unwrap_or_else(|| archon_workflow::task_skeleton::TaskSkeleton {
            schema_version: 1,
            acceptance_digest: String::new(),
            tasks: claims
                .iter()
                .map(|claim| archon_workflow::task_skeleton::FrozenTask {
                    status: None,
                    task_id: claim.task_id.clone(),
                    file_name: String::new(),
                    depends_on: vec![],
                    blocks: vec![],
                    implements: claim.implements.clone(),
                    source_sections: claim.source_sections.clone(),
                    deliverable_contracts: vec![],
                })
                .collect(),
        });
    findings.extend(
        archon_workflow::task_skeleton::constraint_owner_findings(
            &owners,
            &archon_workflow::obligation_ids::constraint_texts(&prd),
        )
        .into_iter()
        .filter(|_| !partial)
        .map(|finding| CoveragePolicyFinding {
            text: finding.message,
            subject: finding.identity.subject.clone(),
            source_path: root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
            identity: Some(finding.identity),
            remediation_scope: archon_workflow::RemediationScope::Skeleton,
        }),
    );
    // A draft/pending skeleton is authoritative before TASK bodies exist.
    // Body equality is checked separately, so a stale body cannot erase a
    // citation the current skeleton already assigns.
    let citations: Vec<_> = match &skeleton {
        Some(skeleton) => skeleton
            .tasks
            .iter()
            .map(|task| (task.task_id.as_str(), task.source_sections.as_slice()))
            .collect(),
        None => claims
            .iter()
            .map(|task| (task.task_id.as_str(), task.source_sections.as_slice()))
            .collect(),
    };
    findings.extend(
        archon_workflow::obligation_ids::section_citation_findings(&prd, citations)
            .into_iter()
            .filter(|finding| !partial || finding.identity.code != "uncited_section")
            .map(|finding| CoveragePolicyFinding {
                text: finding.message,
                subject: finding.identity.subject.clone(),
                source_path: root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
                identity: Some(finding.identity),
                remediation_scope: archon_workflow::RemediationScope::Skeleton,
            }),
    );
    for claim in &claims {
        for cited in &claim.implements {
            if !defined.contains(cited) {
                findings.push(CoveragePolicyFinding {
                    text: format!(
                        "task '{}' cites unknown obligation '{}' in {}; remove it from that task's skeleton implements or correct it to a host-listed id",
                        claim.task_id, cited, claim.source_path
                    ),
                    subject: claim.task_id.clone(),
                    source_path: PathBuf::from(&claim.source_path),
                    identity: None,
                    remediation_scope: archon_workflow::RemediationScope::Skeleton,
                });
            }
        }
    }
    findings.sort_by(|left, right| {
        (&left.text, &left.source_path).cmp(&(&right.text, &right.source_path))
    });
    findings.dedup();
    findings
}

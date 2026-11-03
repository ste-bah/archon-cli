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
    let Ok((claims, _skipped)) = task_requirement_claims_tolerant(root) else {
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
            .filter(|id| !claimed.contains_key(*id) && !constraints.contains(*id))
            .map(|id| CoveragePolicyFinding {
                text: format!(
                    "{id}: defined in the PRD but claimed by no task — add it to at least one TASK file's implements list or remove/correct the PRD obligation"
                ),
                subject: id.clone(),
                source_path: prd_path.clone(),
                identity: None,
                remediation_scope: archon_workflow::RemediationScope::Skeleton,
            }),
    );
    findings.extend(constraints.iter().filter(|id| !claimed.contains_key(*id)).map(|id| {
        CoveragePolicyFinding {
            text: format!("PRD constraint '{id}' is stated as a list item that restricts work, but no task names it; add it to the implements of every task whose work it restricts (all tasks if it restricts all work)"),
            subject: id.clone(),
            source_path: root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
            identity: Some(archon_workflow::defect::DeterministicDefect::new("unowned_constraint", id, "implements")),
            remediation_scope: archon_workflow::RemediationScope::Skeleton,
        }
    }));
    for claim in &claims {
        for cited in &claim.implements {
            if !defined.contains(cited) {
                findings.push(CoveragePolicyFinding {
                    text: format!(
                        "task '{}' cites unknown obligation '{}' in {}; remove it from that TASK file's implements list or correct it to an ID defined by the PRD",
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

//! Whole-set freeze findings for what the acceptance checks exercise (H4,
//! A12): a requirement no check covers, a covered id the PRD does not
//! define, and a task no check answers for.
//!
//! Each finding goes back to the author that can close it: an uncovered
//! requirement names the supplementary check it is owed (`SUP-<id>`, which
//! the acceptance author then writes, covering exactly that requirement,
//! judged by the same judge), a check claiming an unknown id names that
//! check, and a task no check answers for goes to the skeleton author. Only
//! the whole-set freezes raise them; a per-check repair of a contract frozen
//! before `covers` existed is not refused for the coverage it never had --
//! the acceptance round holds such a contract to the PRD instead
//! (`workflow_live_v3_acceptance_drift`).

use super::*;
use archon_workflow::v2::acceptance_stage::coverage::{
    prd_requirement_texts, supplementary_id, tasks_without_checks, uncovered_requirements,
    unknown_covers,
};

/// Findings for every PRD requirement no check of `contract` covers and
/// every covered id the PRD does not define.
pub(super) fn acceptance_coverage_findings(
    prd_text: &str,
    contract_path: &Path,
    contract: &AcceptanceContract,
) -> Vec<GateFinding> {
    let texts = prd_requirement_texts(prd_text);
    let ids: BTreeSet<String> = texts.keys().cloned().collect();
    let finding = |text: String, subject: String| {
        GateFinding::new(
            GateId::FreezeAcceptance,
            text,
            subject,
            Some(contract_path.to_path_buf()),
            archon_workflow::RemediationScope::CandidateArtifact,
        )
    };
    let mut findings: Vec<GateFinding> = uncovered_requirements(&ids, contract)
        .into_iter()
        .map(|requirement| {
            let id = supplementary_id(&requirement);
            finding(
                format!(
                    "check '{id}': PRD requirement {requirement} is covered by no acceptance check; author supplementary check {id} with covers [\"{requirement}\"] that fails whenever {requirement} is violated: {}",
                    texts.get(&requirement).map_or("", String::as_str)
                ),
                id,
            )
        })
        .collect();
    findings.extend(
        unknown_covers(&ids, contract)
            .into_iter()
            .map(|(check, covered)| {
                finding(
                    format!(
                        "check '{check}': covers names {covered}, which the PRD does not define as a requirement; list only PRD requirement ids the check fails on"
                    ),
                    check,
                )
            }),
    );
    findings
}

/// Findings for every skeleton task no frozen check answers for: neither a
/// check's id nor any requirement a check covers is in its `implements`.
pub(super) fn skeleton_check_findings(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    skeleton_path: &Path,
) -> Vec<GateFinding> {
    let tasks = skeleton
        .tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task.implements.as_slice()));
    tasks_without_checks(tasks, contract)
        .into_iter()
        .map(|task| {
            GateFinding::new(
                GateId::FreezeSkeleton,
                format!(
                    "tasks.{task}.implements: task {task} implements nothing a frozen acceptance check answers for (no check's id or covers is in its implements), so no check can show its work done; give it the acceptance or requirement id its work is checked by"
                ),
                "implements",
                Some(skeleton_path.to_path_buf()),
                archon_workflow::RemediationScope::Skeleton,
            )
        })
        .collect()
}

/// A4: findings for every judged-accepted check of `contract` that crashed
/// in its own code, passed on the tree before any implementation (the
/// repository's HEAD at the freeze), or could not be run there. The probe
/// runs only in a hermetic copy (`executability`); why it could not run at
/// all is printed as a diagnostic.
pub(super) async fn pre_implementation_findings(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    contract: &AcceptanceContract,
) -> Vec<GateFinding> {
    use super::executability::{Baseline, ExecutabilityProbe, HostProbe};
    use archon_workflow::task_set_contract::JudgeDecision;
    let scope = super::reauthor::AuthorScope::for_task_set(project_root, tasks_root, prd_path);
    let probe = HostProbe::for_task_set(project_root, tasks_root);
    let probe = match Baseline::head_of(&scope.repository_root) {
        Some(baseline) => probe.with_baseline(baseline),
        None => {
            eprintln!(
                "pre-implementation probe not run: {} is not a git checkout, so the checks are not proven able to fail",
                scope.repository_root.display()
            );
            probe
        }
    };
    let accepted: BTreeSet<String> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .map(|entry| entry.id.clone())
        .collect();
    let defects = probe.script_defects(contract, &accepted).await;
    for diagnostic in probe.take_diagnostics() {
        eprintln!("{diagnostic}");
    }
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    defects
        .into_iter()
        .map(|(id, text)| {
            GateFinding::new(
                GateId::FreezeAcceptance,
                text,
                id,
                Some(contract_path.clone()),
                archon_workflow::RemediationScope::CandidateArtifact,
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "workflow_acceptance_coverage_gate_tests.rs"]
mod tests;

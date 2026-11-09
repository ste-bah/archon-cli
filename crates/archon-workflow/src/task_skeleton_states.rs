use super::*;
use crate::requirement_state::{DoneKind, RequirementDecision, RequirementState, task_status};
pub(super) fn defects(skeleton: &TaskSkeleton) -> Vec<crate::defect::ValidationDefect> {
    let mut defects = Vec::new();
    for task in &skeleton.tasks {
        let mut add = |code, field: &str, text: String| {
            defects.push(crate::defect::ValidationDefect::new(
                code,
                &task.task_id,
                field,
                text,
            ))
        };
        let states = &task.requirement_states;
        if !states.is_empty()
            && states.keys().collect::<BTreeSet<_>>() != task.implements.iter().collect()
        {
            add(
                "requirement_states_mismatch",
                "requirement_states",
                format!("task {} states differ from its claimed ids", task.task_id),
            );
        }
        if (task
            .status
            .as_deref()
            .is_some_and(|s| matches!(s, "done" | "kept"))
            || !states.is_empty())
            && task.status != task_status(states)
        {
            add(
                "done_status_mismatch",
                "status",
                format!(
                    "task {} completion status differs from its states",
                    task.task_id
                ),
            );
        }
        for id in task
            .implements
            .iter()
            .chain(task.proven_done.iter().map(|p| &p.id))
            .collect::<BTreeSet<_>>()
        {
            let proofs: Vec<_> = task.proven_done.iter().filter(|p| &p.id == id).collect();
            let done = states
                .get(id)
                .is_some_and(|d| d.state == RequirementState::Done);
            if if done {
                proofs.len() != 1
                    || proofs.iter().any(|p| match p.kind {
                        DoneKind::Breakage => {
                            p.mutant_id.is_none()
                                || p.body_gut_mutant_id.is_none()
                                || p.patch_digest.is_none()
                        }
                        DoneKind::TextEvidence => p.literals.is_empty(),
                    })
            } else {
                !proofs.is_empty()
            } {
                add(
                    "done_proof_mismatch",
                    id,
                    format!(
                        "task {} id {id} evidence differs from its state",
                        task.task_id
                    ),
                );
            }
        }
    }
    defects
}
pub fn refresh(
    skeleton: &mut TaskSkeleton,
    contract: &crate::task_set_contract::AcceptanceContract,
) {
    for task in &mut skeleton.tasks {
        let stale: Vec<_> = task
            .proven_done
            .iter()
            .filter(|p| {
                !contract
                    .acceptance
                    .iter()
                    .chain(&contract.supplementary)
                    .any(|c| {
                        c.id == p.check_id
                            && crate::baseline_results::check_digest(&c.check) == p.check_digest
                    })
            })
            .map(|p| p.id.clone())
            .collect();
        for id in stale {
            task.requirement_states.insert(
                id.clone(),
                RequirementDecision::new(RequirementState::Undecided, "proof_stale"),
            );
            task.proven_done.retain(|p| p.id != id);
            task.status = task_status(&task.requirement_states);
        }
    }
}

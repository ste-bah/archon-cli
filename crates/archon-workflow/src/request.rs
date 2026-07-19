use crate::context;
use crate::error::WorkflowResult;
use crate::fanout::FanoutItem;
use crate::run::WorkflowRun;
use crate::runner::StageRunRequest;
use crate::spec::{ProviderTier, StageKind, StageSpec};
use crate::store::WorkflowStore;

pub(crate) fn stage_request(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> WorkflowResult<StageRunRequest> {
    let attempt = run
        .stages
        .get(&stage.id)
        .map(|state| state.attempt)
        .unwrap_or(1);
    let stage_kind = stage.kind;
    let task = constrained_task(
        stage.task.clone().unwrap_or_else(|| run.spec.task.clone()),
        stage_kind,
    );
    Ok(StageRunRequest {
        run_id: run.id.clone(),
        stage_id: stage.id.clone(),
        stage_kind,
        agent: stage.agent.clone(),
        task,
        attempt,
        provider_tier: stage.provider_tier.unwrap_or(ProviderTier::Planner),
        depends_on: stage.depends_on.clone(),
        input: context::stage_input(store, run, stage)?,
    })
}

pub(crate) fn fanout_item_request(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
    item: &FanoutItem,
) -> WorkflowResult<StageRunRequest> {
    let mut request = stage_request(store, run, stage)?;
    request.stage_id = item.id.clone();
    request.stage_kind = stage.effective_item_kind();
    request.task = constrained_task(request.task, request.stage_kind);
    request.input = context::fanout_input(store, run, stage, item)?;
    Ok(request)
}

pub(crate) const IMPLEMENTATION_CONSTRAINTS: &str = "\
Hard engineering constraints for implementation work:
- Keep every changed or newly-created source file at or below 500 lines.
- Keep every new or changed function's cyclomatic complexity at or below 15.
- Split code into focused modules before a file or function exceeds those limits.
- Do not treat tests, review text, or partial fixes as success when either limit is violated.";

fn constrained_task(task: String, kind: StageKind) -> String {
    if kind != StageKind::Implementation || task.contains("Hard engineering constraints") {
        return task;
    }
    format!("{task}\n\n{IMPLEMENTATION_CONSTRAINTS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementation_task_gets_constraints_once() {
        let first = constrained_task("Do work.".into(), StageKind::Implementation);
        assert!(first.contains("500 lines"));
        assert!(first.contains("cyclomatic complexity"));
        let second = constrained_task(first.clone(), StageKind::Implementation);
        assert_eq!(second, first);
    }

    #[test]
    fn non_implementation_task_is_unchanged() {
        let task = constrained_task("Review only.".into(), StageKind::Agent);
        assert_eq!(task, "Review only.");
    }
}

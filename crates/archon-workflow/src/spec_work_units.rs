use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::{StageKind, StageSpec};

pub(crate) fn validate_inline_implementation_work_units(stage: &StageSpec) -> WorkflowResult<()> {
    if stage.item_kind != Some(StageKind::Implementation) {
        return Ok(());
    }
    let Some(items) = stage
        .input
        .get("items")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(());
    };
    let stage_has_scope = !crate::work_unit_coverage::stage_required_units(stage).is_empty();
    if stage_has_scope {
        return Ok(());
    }
    for (idx, item) in items.iter().enumerate() {
        if crate::work_unit_coverage::item_required_units(item).is_empty() {
            return Err(WorkflowError::SpecInvalid(format!(
                "implementation fanout stage '{}' item {idx} requires work_unit_id, work_unit_ids, task_id, or task_ids",
                stage.id
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_direct_implementation_work_units(stage: &StageSpec) -> WorkflowResult<()> {
    if stage.kind != StageKind::Implementation
        || !crate::work_unit_coverage::stage_required_units(stage).is_empty()
    {
        return Ok(());
    }
    Err(WorkflowError::SpecInvalid(format!(
        "implementation stage '{}' requires completion_task_ids, required_work_units, work_unit_id, work_unit_ids, task_id, or task_ids",
        stage.id
    )))
}

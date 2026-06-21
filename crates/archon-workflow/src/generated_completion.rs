use serde_json::Value;

use crate::spec::{StageKind, StageSpec, WorkflowSpec};

pub(crate) fn ensure_generated_completion_contracts(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        if stage.kind != StageKind::Fanout
            || stage.effective_item_kind() != StageKind::Implementation
            || !has_inventory_foreach(stage)
            || stage_allows_empty_items(stage)
            || crate::stage::is_recovery_stage(stage)
        {
            continue;
        }
        let task_ids = completion_task_ids_for_stage(stage);
        if task_ids.is_empty() {
            continue;
        }
        stage
            .extra
            .entry("allow_empty_when_completed".into())
            .or_insert(Value::Bool(true));
        stage
            .extra
            .entry("completion_task_ids".into())
            .or_insert_with(|| strings(task_ids));
    }
}

fn stage_allows_empty_items(stage: &StageSpec) -> bool {
    stage
        .extra
        .get("allow_empty_items")
        .or_else(|| stage.input.get("allow_empty_items"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn has_inventory_foreach(stage: &StageSpec) -> bool {
    stage
        .foreach
        .as_deref()
        .is_some_and(|foreach| foreach.contains(".items"))
}

fn completion_task_ids_for_stage(stage: &StageSpec) -> Vec<String> {
    let mut ids = crate::work_unit_coverage::stage_required_units(stage)
        .into_iter()
        .collect::<Vec<_>>();
    if ids.is_empty() {
        ids = task_ids_from_filter(stage.filter.as_deref());
    }
    if ids.is_empty() {
        ids = task_ids_from_text(&stage.id);
    }
    if ids.is_empty() {
        ids = task_ids_from_text(stage.task.as_deref().unwrap_or_default());
    }
    ids.sort();
    ids.dedup();
    ids
}

fn task_ids_from_filter(filter: Option<&str>) -> Vec<String> {
    let Some(filter) = filter else {
        return Vec::new();
    };
    if let Some((_, rhs)) = filter.split_once("==") {
        return task_ids_from_text(rhs);
    }
    if let Some((_, rhs)) = filter.split_once(" in ") {
        return task_ids_from_text(rhs);
    }
    if let Some((_, rhs)) = filter.split_once(" contains ") {
        return task_ids_from_text(rhs);
    }
    Vec::new()
}

fn task_ids_from_text(text: &str) -> Vec<String> {
    let normalized = text
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>();
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let mut out = Vec::new();
    for idx in 0..tokens.len() {
        let token = tokens[idx].to_ascii_uppercase();
        if is_short_task_id(&token) {
            out.push(token);
            continue;
        }
        if (token == "TASK" || token == "TDL" || token == "TASKTDL")
            && let Some(next) = tokens.get(idx + 1)
            && let Some(task) = numeric_task_id(next)
        {
            out.push(task);
        }
    }
    out
}

fn is_short_task_id(token: &str) -> bool {
    token.len() == 4
        && token.starts_with('T')
        && token.chars().skip(1).all(|ch| ch.is_ascii_digit())
}

fn numeric_task_id(text: &str) -> Option<String> {
    let digits = text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then(|| format!("T{:0>3}", digits))
}

fn strings(values: Vec<String>) -> Value {
    Value::Array(values.into_iter().map(Value::String).collect())
}

use std::collections::BTreeSet;

use serde_json::Value;

use crate::spec::{ProviderTier, RetryPolicy, StageKind, StageSpec, WorkflowSpec};

pub(super) fn promote_quality_gate_entries(spec: &mut WorkflowSpec) {
    if spec
        .stages
        .iter()
        .any(|stage| matches!(stage.kind, StageKind::QualityGate))
    {
        return;
    }
    let existing: BTreeSet<String> = spec.stages.iter().map(|stage| stage.id.clone()).collect();
    for (key, value) in &spec.quality_gates {
        let Some(stage) = quality_gate_stage(key, value, &existing) else {
            continue;
        };
        spec.stages.push(stage);
    }
}

fn quality_gate_stage(key: &str, value: &Value, existing: &BTreeSet<String>) -> Option<StageSpec> {
    let object = value.as_object()?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(key)
        .to_string();
    if existing.contains(&id) {
        return None;
    }
    let task = object
        .get("task")
        .and_then(Value::as_str)
        .map(str::to_string);
    let depends_on = text_values(object.get("depends_on"));
    let provider_tier = object
        .get("provider_tier")
        .and_then(|value| serde_json::from_value::<ProviderTier>(value.clone()).ok());
    let extra = object
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "id" | "task" | "depends_on" | "provider_tier"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Some(StageSpec {
        id,
        kind: StageKind::QualityGate,
        task,
        agent: None,
        foreach: None,
        reducer: None,
        tool: None,
        condition: None,
        depends_on,
        provider_tier,
        retry: RetryPolicy::default(),
        input: Value::Null,
        model: None,
        provider: None,
        expected_target_files: Vec::new(),
        verify_command: None,
        max_parallelism: None,
        item_kind: None,
        extra,
    })
}

fn text_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

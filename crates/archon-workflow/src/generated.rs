use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::spec::{ProviderTier, RetryPolicy, StageKind, StageSpec, WorkflowSpec};

pub fn normalize_generated_spec(spec: &mut WorkflowSpec) {
    neutralize_provider_tiers(spec);
    normalize_under_specified_stages(spec);
    infer_dependencies_from_io(spec);
    promote_quality_gate_entries(spec);
}

/// Planner LLMs routinely emit a top-level `provider_tiers` map pinned to a
/// concrete provider/model (e.g. `planner: {provider: anthropic, model: ...}`).
/// That map is never consulted at runtime — stage execution resolves models from
/// each stage's `provider_tier` alias — yet a non-neutral value trips the strict
/// `HardcodedModel` guard and aborts the whole plan. Since this is *generated*
/// output (not a user-authored spec), drop any non-neutral entry so the plan
/// stays provider-neutral and valid instead of failing recoverable input.
fn neutralize_provider_tiers(spec: &mut WorkflowSpec) {
    spec.provider_tiers
        .retain(|_, value| crate::spec::is_neutral_tier_hint(value));
}

fn normalize_under_specified_stages(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        let missing_tool = stage.kind == StageKind::Tool && !has_text(stage.tool.as_deref());
        let missing_condition =
            stage.kind == StageKind::Condition && !has_text(stage.condition.as_deref());
        if missing_tool || missing_condition {
            let original = format!("{:?}", stage.kind);
            stage.kind = StageKind::Agent;
            stage
                .extra
                .insert("normalized_from_kind".into(), Value::String(original));
        }
    }
}

fn infer_dependencies_from_io(spec: &mut WorkflowSpec) {
    let mut producers = BTreeMap::new();
    for stage in &spec.stages {
        for output in text_values(stage.extra.get("outputs")) {
            producers.insert(output, stage.id.clone());
        }
    }

    for stage in &mut spec.stages {
        if !stage.depends_on.is_empty() {
            continue;
        }
        for input in text_values(stage.extra.get("inputs")) {
            if let Some(dep) = producers.get(&input)
                && dep != &stage.id
                && !stage.depends_on.contains(dep)
            {
                stage.depends_on.push(dep.clone());
            }
        }
    }
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
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

fn promote_quality_gate_entries(spec: &mut WorkflowSpec) {
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
        extra,
    })
}

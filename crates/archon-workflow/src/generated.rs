use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::spec::{ProviderTier, RetryPolicy, StageKind, StageSpec, WorkflowSpec};

pub fn normalize_generated_spec(spec: &mut WorkflowSpec) {
    neutralize_provider_tiers(spec);
    normalize_under_specified_stages(spec);
    bridge_decorative_fanouts(spec);
    infer_implementation_fanouts(spec);
    infer_dependencies_from_io(spec);
    promote_quality_gate_entries(spec);
}

fn infer_implementation_fanouts(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        if stage.infers_implementation_fanout()
            && (has_usable_foreach(stage)
                || stage.input.get("items").and_then(Value::as_array).is_some())
        {
            stage.item_kind = Some(StageKind::Implementation);
        }
    }
}

/// Planner LLMs frequently describe a fan-out with a decorative block
/// (`fanout: {over: ordered_workstreams, respect_dependencies: task_dag}`)
/// instead of the executable `foreach: ${producer.items}` form. That block
/// lands in `stage.extra` and is never read at runtime, so the fan-out silently
/// collapses to a single synthetic item. Bridge it: when the `over` token
/// resolves to a real upstream structured-items producer (a stage whose id is
/// the token, or a stage whose `outputs` advertise the token), rewrite it to a
/// proper `foreach` accessor and add the `depends_on` edge. Tokens that resolve
/// to nothing are left untouched so `validate_fanout_contracts` rejects them.
fn bridge_decorative_fanouts(spec: &mut WorkflowSpec) {
    let producers = items_producers(spec);
    for idx in 0..spec.stages.len() {
        if spec.stages[idx].kind != StageKind::Fanout {
            continue;
        }
        if has_usable_foreach(&spec.stages[idx]) {
            continue;
        }
        let Some(token) = fanout_over_token(&spec.stages[idx]) else {
            continue;
        };
        let Some(producer) = producers.get(token.trim()).cloned() else {
            continue;
        };
        if producer == spec.stages[idx].id {
            continue;
        }
        spec.stages[idx].foreach = Some(format!("${{{producer}.items}}"));
        if !spec.stages[idx].depends_on.contains(&producer) {
            spec.stages[idx].depends_on.push(producer.clone());
        }
        declare_items_output(spec, &producer);
    }
}

/// Ensure the bridged producer advertises `items` in its `outputs` list so the
/// resulting plan satisfies the producer side of the fan-out contract. The
/// producer's runtime job is to emit an `items:` document; recording it in
/// `outputs` keeps the generated spec self-consistent and lets dependency
/// inference treat it as the items source.
fn declare_items_output(spec: &mut WorkflowSpec, producer_id: &str) {
    let Some(stage) = spec.stages.iter_mut().find(|stage| stage.id == producer_id) else {
        return;
    };
    if text_values(stage.extra.get("outputs"))
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case("items"))
    {
        return;
    }
    let mut outputs = match stage.extra.remove("outputs") {
        Some(Value::Array(values)) => values,
        Some(Value::String(value)) => vec![Value::String(value)],
        _ => Vec::new(),
    };
    outputs.push(Value::String("items".to_string()));
    stage
        .extra
        .insert("outputs".to_string(), Value::Array(outputs));
}

/// Map every fan-out source token to the stage that produces it. A stage
/// produces a token when its id equals the token or when its `outputs` list
/// advertises the token (e.g. `ordered_workstreams`).
fn items_producers(spec: &WorkflowSpec) -> BTreeMap<String, String> {
    let mut producers = BTreeMap::new();
    for stage in &spec.stages {
        for output in text_values(stage.extra.get("outputs")) {
            producers.entry(output).or_insert_with(|| stage.id.clone());
        }
    }
    for stage in &spec.stages {
        producers
            .entry(stage.id.clone())
            .or_insert_with(|| stage.id.clone());
    }
    producers
}

fn has_usable_foreach(stage: &StageSpec) -> bool {
    stage
        .foreach
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

/// Extract the `over` token from a decorative fan-out, whether it sits inside a
/// nested `fanout` object or directly on the stage's extra map.
fn fanout_over_token(stage: &StageSpec) -> Option<String> {
    if let Some(Value::Object(fanout)) = stage.extra.get("fanout")
        && let Some(token) = fanout.get("over").and_then(Value::as_str)
        && !token.trim().is_empty()
    {
        return Some(token.to_string());
    }
    stage
        .extra
        .get("over")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
        .map(str::to_string)
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
        max_parallelism: None,
        item_kind: None,
        extra,
    })
}

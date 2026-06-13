use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::spec::{ProviderTier, StageKind, StageSpec, WorkflowSpec, has_decorative_fanout_keys};

#[path = "generated_remediation.rs"]
mod generated_remediation;
use generated_remediation::{
    ensure_generated_remediation_loop, implementation_target_inventory_stage, unique_stage_id,
};
#[path = "generated_quality.rs"]
mod generated_quality;
use generated_quality::promote_quality_gate_entries;

pub(crate) fn sanitize_generated_value(value: &mut Value) {
    let Some(stages) = value.get_mut("stages").and_then(Value::as_array_mut) else {
        return;
    };
    for stage in stages {
        sanitize_stage_provider_tier(stage);
    }
}

fn sanitize_stage_provider_tier(stage: &mut Value) {
    let Some(object) = stage.as_object_mut() else {
        return;
    };
    let Some(raw) = object.get("provider_tier") else {
        return;
    };
    if valid_provider_tier_value(raw) {
        return;
    }
    let tier = raw
        .as_str()
        .and_then(stage_provider_tier_alias)
        .unwrap_or_else(|| inferred_stage_provider_tier(object));
    object.insert("provider_tier".into(), Value::String(tier.into()));
}

fn valid_provider_tier_value(value: &Value) -> bool {
    value.as_str().is_some_and(|tier| {
        serde_json::from_value::<ProviderTier>(Value::String(tier.into())).is_ok()
    })
}

fn stage_provider_tier_alias(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "executor" | "execution" | "implementer" | "implementation" | "developer" | "engineer"
        | "builder" | "writer" | "patcher" => Some("coder"),
        "reviewer" | "auditor" | "skeptic" | "qa" | "quality" | "verifier" => Some("critic"),
        "synthesizer" | "synthesis" | "summarizer" | "reporter" => Some("reducer"),
        "research" | "analyst" | "analysis" | "investigator" => Some("researcher"),
        "orchestrator" | "coordinator" => Some("planner"),
        "fast" | "low_cost" => Some("cheap"),
        _ => None,
    }
}

fn inferred_stage_provider_tier(object: &serde_json::Map<String, Value>) -> &'static str {
    let text = format!(
        "{} {} {}",
        object.get("id").and_then(Value::as_str).unwrap_or_default(),
        object
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        object
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
    .to_ascii_lowercase();
    if contains_any(
        &text,
        &["implement", "remediate", "repair", "edit", "patch", "code"],
    ) {
        "coder"
    } else if contains_any(&text, &["review", "audit", "quality", "critic", "verify"]) {
        "critic"
    } else if contains_any(&text, &["reduce", "synthesis", "synthesize", "report"]) {
        "reducer"
    } else if contains_any(&text, &["research", "investigate", "evidence"]) {
        "researcher"
    } else {
        "planner"
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

pub fn normalize_generated_spec(spec: &mut WorkflowSpec) {
    neutralize_provider_tiers(spec);
    normalize_under_specified_stages(spec);
    promote_generated_implementation_agents(spec);
    normalize_generated_fanout_shapes(spec);
    bridge_decorative_fanouts(spec);
    normalize_generated_item_kinds(spec);
    infer_implementation_fanouts(spec);
    infer_dependencies_from_io(spec);
    normalize_targetless_implementation_stages(spec);
    promote_quality_gate_entries(spec);
    ensure_generated_remediation_loop(spec);
}

fn normalize_generated_fanout_shapes(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        if stage.kind == StageKind::Fanout || !has_fanout_shape(stage) {
            continue;
        }
        let original = format!("{:?}", stage.kind);
        stage.kind = StageKind::Fanout;
        stage
            .extra
            .insert("normalized_from_kind".into(), Value::String(original));
    }
}

fn normalize_generated_item_kinds(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        match (stage.kind, stage.item_kind) {
            (StageKind::Fanout, Some(StageKind::Implementation)) => {}
            (StageKind::Implementation, Some(_)) => stage.item_kind = None,
            (_, Some(_)) => stage.item_kind = None,
            (_, None) => {}
        }
    }
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

fn has_fanout_shape(stage: &StageSpec) -> bool {
    has_usable_foreach(stage)
        || stage.input.get("items").and_then(Value::as_array).is_some()
        || has_decorative_fanout_keys(stage)
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

fn promote_generated_implementation_agents(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        if stage.kind != StageKind::Agent || !agent_stage_implements_repo(stage) {
            continue;
        }
        stage.kind = StageKind::Implementation;
        stage.provider_tier.get_or_insert(ProviderTier::Coder);
        stage
            .extra
            .insert("normalized_from_kind".into(), Value::String("Agent".into()));
    }
}

fn agent_stage_implements_repo(stage: &StageSpec) -> bool {
    let id = stage.id.to_ascii_lowercase();
    let task = stage
        .task
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if contains_any(
        &id,
        &[
            "review",
            "audit",
            "test",
            "verify",
            "plan",
            "inventory",
            "discover",
            "synthesis",
            "report",
            "quality",
        ],
    ) || task.starts_with("perform read-only")
        || task.starts_with("produce an ordered implementation plan")
    {
        return false;
    }
    id == "implement"
        || id.starts_with("implement_")
        || id.starts_with("implement-")
        || id.ends_with("_implement")
        || id.ends_with("-implement")
        || id.contains("_implement_")
        || id.contains("-implement-")
        || task.starts_with("implement ")
        || task.contains("implement only")
        || task.contains("implement missing")
        || task.contains("modify repository")
        || task.contains("modify the repository")
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

fn normalize_targetless_implementation_stages(spec: &mut WorkflowSpec) {
    let mut existing = spec
        .stages
        .iter()
        .map(|stage| stage.id.clone())
        .collect::<BTreeSet<_>>();
    let mut normalized = Vec::with_capacity(spec.stages.len());

    for mut stage in std::mem::take(&mut spec.stages) {
        if stage.kind != StageKind::Implementation || has_declared_targets(&stage) {
            normalized.push(stage);
            continue;
        }

        if let Some(targets) = loose_target_files(&stage) {
            stage.expected_target_files = targets;
            normalized.push(stage);
            continue;
        }

        let inventory_id = unique_stage_id(&format!("{}-target-inventory", stage.id), &existing);
        existing.insert(inventory_id.clone());
        let inventory = implementation_target_inventory_stage(&inventory_id, &stage);
        stage.extra.insert(
            "normalized_from_kind".into(),
            Value::String("Implementation".into()),
        );
        stage.kind = StageKind::Fanout;
        stage.foreach = Some(format!("${{{inventory_id}.items}}"));
        stage.item_kind = Some(StageKind::Implementation);
        stage.max_parallelism.get_or_insert(1);
        stage
            .extra
            .insert("allow_empty_items".into(), Value::Bool(true));
        if !stage.depends_on.contains(&inventory_id) {
            stage.depends_on.insert(0, inventory_id.clone());
        }
        normalized.push(inventory);
        normalized.push(stage);
    }

    spec.stages = normalized;
}

fn has_declared_targets(stage: &StageSpec) -> bool {
    stage
        .expected_target_files
        .iter()
        .any(|target| has_text(Some(target)))
}

fn loose_target_files(stage: &StageSpec) -> Option<Vec<String>> {
    let mut targets = Vec::new();
    for key in [
        "target_files",
        "target_file",
        "target_path",
        "expected_target_files",
    ] {
        targets.extend(text_values(stage.extra.get(key)));
        targets.extend(text_values(stage.input.get(key)));
    }
    targets.retain(|target| !target.trim().is_empty());
    targets.sort();
    targets.dedup();
    (!targets.is_empty()).then_some(targets)
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

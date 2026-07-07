// Legacy generated-`WorkflowSpec` normalizers and patchers.
//
// These functions exist only for explicit legacy YAML/spec workflows. V2
// generated workflows carry control state through typed host calls and
// `WorkflowV2Result` records, not generated YAML patch-up passes.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::spec::{ProviderTier, StageKind, StageSpec, WorkflowSpec, has_decorative_fanout_keys};

#[path = "../generated_remediation.rs"]
mod generated_remediation;
use generated_remediation::{
    ensure_generated_remediation_loop, implementation_target_inventory_stage, unique_stage_id,
};
#[path = "../generated_quality.rs"]
mod generated_quality;
use generated_quality::promote_quality_gate_entries;
#[path = "../generated_sanitize.rs"]
mod generated_sanitize;
pub(crate) use generated_sanitize::sanitize_generated_value;
#[path = "../generated_completion.rs"]
mod generated_completion;
#[path = "../generated_remediation_contract.rs"]
mod generated_remediation_contract;
#[path = "../generated_task_items.rs"]
mod generated_task_items;

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

pub fn normalize_generated_spec(spec: &mut WorkflowSpec) {
    neutralize_provider_tiers(spec);
    normalize_generated_foreach_accessors(spec);
    ensure_generated_foreach_dependencies(spec);
    normalize_under_specified_stages(spec);
    promote_generated_implementation_agents(spec);
    normalize_generated_fanout_shapes(spec);
    bridge_decorative_fanouts(spec);
    normalize_generated_item_kinds(spec);
    infer_implementation_fanouts(spec);
    infer_dependencies_from_io(spec);
    let allow_generated_target_inventory = generated_expansion_requested(
        spec,
        &[
            "allow_generated_target_inventory",
            "enable_generated_target_inventory",
        ],
    );
    normalize_targetless_implementation_stages(spec, allow_generated_target_inventory);
    ensure_direct_implementation_work_units(spec);
    generated_completion::ensure_generated_completion_contracts(spec);
    generated_remediation_contract::ensure_remediation_contracts(spec);
    promote_quality_gate_entries(spec);
    ensure_generated_remediation_loop(spec);
}

fn generated_expansion_requested(spec: &WorkflowSpec, keys: &[&str]) -> bool {
    spec.stages
        .iter()
        .any(|stage| keys.iter().any(|key| bool_field(stage, key)))
}

fn bool_field(stage: &StageSpec, key: &str) -> bool {
    stage
        .extra
        .get(key)
        .or_else(|| stage.input.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn ensure_generated_foreach_dependencies(spec: &mut WorkflowSpec) {
    let stage_ids = spec
        .stages
        .iter()
        .map(|stage| stage.id.clone())
        .collect::<BTreeSet<_>>();
    let mut producers_to_declare = BTreeSet::new();
    for stage in &mut spec.stages {
        let Some(dep) = foreach_dependency(stage) else {
            continue;
        };
        if dep == stage.id || !stage_ids.contains(&dep) {
            continue;
        }
        if !stage.depends_on.contains(&dep) {
            stage.depends_on.push(dep.clone());
        }
        producers_to_declare.insert(dep);
    }
    for producer in producers_to_declare {
        declare_items_output(spec, &producer);
    }
}

fn foreach_dependency(stage: &StageSpec) -> Option<String> {
    let foreach = stage.foreach.as_deref()?.trim();
    let inner = foreach.strip_prefix("${")?.strip_suffix('}')?;
    let (dep, accessor) = inner.split_once('.')?;
    (accessor.trim() == "items")
        .then(|| dep.trim().to_string())
        .filter(|dep| !dep.is_empty())
}

fn normalize_generated_foreach_accessors(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        let Some(foreach) = stage.foreach.as_deref() else {
            continue;
        };
        if let Some(canonical) = canonical_foreach_accessor(foreach) {
            stage.foreach = Some(canonical);
        }
    }
}

fn canonical_foreach_accessor(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('$')?.trim();
    let inner = inner.trim_start_matches('{').trim_end_matches('}').trim();
    let (stage, accessor) = inner.split_once('.')?;
    let stage = stage.trim();
    let accessor = accessor.trim();
    if stage.is_empty() || accessor.is_empty() {
        return None;
    }
    Some(format!("${{{stage}.{accessor}}}"))
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

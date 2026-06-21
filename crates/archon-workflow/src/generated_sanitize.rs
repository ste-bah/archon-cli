use serde_json::Value;

use crate::spec::{ProviderTier, WORKFLOW_SCHEMA};

pub(crate) fn sanitize_generated_value(value: &mut Value) {
    sanitize_top_level(value);
    normalize_stages_shape(value);
    let Some(stages) = value.get_mut("stages").and_then(Value::as_array_mut) else {
        return;
    };
    for (idx, stage) in stages.iter_mut().enumerate() {
        let Some(object) = stage.as_object_mut() else {
            continue;
        };
        sanitize_stage_kind(object);
        sanitize_stage_id(idx, object);
        sanitize_stage_provider_tier(object);
    }
}

fn sanitize_top_level(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.retain(|key, _| {
        matches!(
            key.as_str(),
            "schema"
                | "name"
                | "task"
                | "target_repository_root"
                | "max_parallelism"
                | "max_agents"
                | "provider_tiers"
                | "stages"
                | "artifact_policy"
                | "permissions"
                | "quality_gates"
                | "learning_hooks"
        )
    });
    if !non_empty_string(object.get("schema")) {
        object.insert("schema".into(), Value::String(WORKFLOW_SCHEMA.into()));
    }
    if !non_empty_string(object.get("name")) {
        object.insert("name".into(), Value::String("generated-workflow".into()));
    }
}

fn normalize_stages_shape(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let Some(Value::Object(stages)) = object.get("stages") else {
        return;
    };
    let mut normalized = Vec::new();
    for (id, stage) in stages {
        let mut stage = stage.clone();
        if let Some(stage_object) = stage.as_object_mut() {
            stage_object
                .entry("id")
                .or_insert_with(|| Value::String(id.clone()));
        }
        normalized.push(stage);
    }
    object.insert("stages".into(), Value::Array(normalized));
}

fn sanitize_stage_id(idx: usize, object: &mut serde_json::Map<String, Value>) {
    if non_empty_string(object.get("id")) {
        return;
    }
    let id = ["name", "stage", "task", "kind"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str).map(slug_id))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| format!("stage_{}", idx + 1));
    object.insert("id".into(), Value::String(id));
    object
        .entry("normalized_from_missing_id")
        .or_insert(Value::Bool(true));
}

fn sanitize_stage_kind(object: &mut serde_json::Map<String, Value>) {
    if object
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.trim().is_empty())
    {
        return;
    }
    object.insert(
        "kind".into(),
        Value::String(inferred_stage_kind(object).into()),
    );
    object
        .entry("normalized_from_missing_kind")
        .or_insert(Value::Bool(true));
}

fn inferred_stage_kind(object: &serde_json::Map<String, Value>) -> &'static str {
    if has_fanout_shape(object) {
        return "fanout";
    }
    if object.contains_key("reducer") || stage_text_contains(object, &["reduce", "synthesis"]) {
        return "reduce";
    }
    if object.contains_key("tool") {
        return "tool";
    }
    if stage_text_contains(object, &["quality_gate", "quality gate"]) {
        return "quality_gate";
    }
    if stage_text_contains(object, &["human_gate", "human gate", "signoff", "sign-off"]) {
        return "human_gate";
    }
    if stage_text_contains(object, &["checkpoint"]) {
        return "checkpoint";
    }
    "agent"
}

fn has_fanout_shape(object: &serde_json::Map<String, Value>) -> bool {
    object
        .get("foreach")
        .and_then(Value::as_str)
        .is_some_and(|foreach| !foreach.trim().is_empty())
        || object
            .get("input")
            .and_then(|input| input.get("items"))
            .and_then(Value::as_array)
            .is_some()
        || ["fanout", "over", "respect_dependencies"]
            .iter()
            .any(|key| object.contains_key(*key))
}

fn sanitize_stage_provider_tier(object: &mut serde_json::Map<String, Value>) {
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
    if stage_text_contains(
        object,
        &["implement", "remediate", "repair", "edit", "patch", "code"],
    ) {
        "coder"
    } else if stage_text_contains(object, &["review", "audit", "quality", "critic", "verify"]) {
        "critic"
    } else if stage_text_contains(object, &["reduce", "synthesis", "synthesize", "report"]) {
        "reducer"
    } else if stage_text_contains(object, &["research", "investigate", "evidence"]) {
        "researcher"
    } else {
        "planner"
    }
}

fn stage_text_contains(object: &serde_json::Map<String, Value>, needles: &[&str]) -> bool {
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
    needles.iter().any(|needle| text.contains(needle))
}

fn non_empty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
}

fn slug_id(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
        if out.len() >= 48 {
            break;
        }
    }
    out.trim_matches('_').to_string()
}

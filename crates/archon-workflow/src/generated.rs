use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::spec::{
    ProviderTier, RetryPolicy, StageKind, StageSpec, WorkflowSpec, has_decorative_fanout_keys,
};

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

fn ensure_generated_remediation_loop(spec: &mut WorkflowSpec) {
    if !has_write_capable_stage(spec) || has_remediation_stage(spec) {
        return;
    }
    let Some(gate_idx) = spec
        .stages
        .iter()
        .rposition(|stage| stage.kind == StageKind::QualityGate)
    else {
        return;
    };
    let gate_deps = spec.stages[gate_idx].depends_on.clone();
    let inventory_deps = remediation_inventory_deps(spec, gate_idx, &gate_deps);
    if inventory_deps.is_empty() {
        return;
    }
    let existing = spec
        .stages
        .iter()
        .map(|stage| stage.id.clone())
        .collect::<BTreeSet<_>>();
    let inventory_id = unique_stage_id("remediation-inventory", &existing);
    let repair_id = unique_stage_id("remediate-failed-findings", &existing);
    let tests_id = unique_stage_id("post-remediation-focused-tests", &existing);
    let review_id = unique_stage_id("post-remediation-adversarial-review", &existing);
    let report_id = unique_stage_id("post-remediation-acceptance-report", &existing);
    let post_review_deps =
        post_remediation_review_deps(&inventory_id, &repair_id, &tests_id, &inventory_deps);
    let stages = vec![
        remediation_inventory_stage(&inventory_id, inventory_deps),
        remediation_fanout_stage(&repair_id, &inventory_id),
        post_remediation_tests_stage(&tests_id, &repair_id, &inventory_id),
        post_remediation_review_stage(&review_id, post_review_deps),
        remediation_report_stage(&report_id, &tests_id, &review_id),
    ];
    for (offset, stage) in stages.into_iter().enumerate() {
        spec.stages.insert(gate_idx + offset, stage);
    }
    spec.stages[gate_idx + 5].depends_on = vec![report_id];
}

fn has_write_capable_stage(spec: &WorkflowSpec) -> bool {
    spec.stages.iter().any(|stage| {
        stage.kind == StageKind::Implementation
            || stage.item_kind == Some(StageKind::Implementation)
    })
}

fn has_remediation_stage(spec: &WorkflowSpec) -> bool {
    spec.stages.iter().any(|stage| {
        let id = stage.id.to_ascii_lowercase();
        id.contains("remediation") || id.contains("remediate") || id.contains("repair")
    })
}

fn remediation_inventory_deps(
    spec: &WorkflowSpec,
    gate_idx: usize,
    gate_deps: &[String],
) -> Vec<String> {
    let mut deps = gate_deps.to_vec();
    for stage in spec.stages.iter().take(gate_idx) {
        if is_review_like_stage(stage) {
            push_unique_stage_id(&mut deps, stage.id.clone());
        }
    }
    deps
}

fn is_review_like_stage(stage: &StageSpec) -> bool {
    if stage.kind == StageKind::QualityGate {
        return false;
    }
    if stage.provider_tier == Some(ProviderTier::Critic) {
        return true;
    }
    let text = format!(
        "{} {}",
        stage.id.to_ascii_lowercase(),
        stage
            .task
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
    );
    ["adversarial", "review", "audit", "critic", "quality"]
        .iter()
        .any(|needle| text.contains(needle))
}

fn post_remediation_review_deps(
    inventory_id: &str,
    repair_id: &str,
    tests_id: &str,
    original_review_deps: &[String],
) -> Vec<String> {
    let mut deps = vec![
        inventory_id.to_string(),
        repair_id.to_string(),
        tests_id.to_string(),
    ];
    for dep in original_review_deps {
        push_unique_stage_id(&mut deps, dep.clone());
    }
    deps
}

fn push_unique_stage_id(deps: &mut Vec<String>, dep: String) {
    if !deps.contains(&dep) {
        deps.push(dep);
    }
}

fn unique_stage_id(base: &str, existing: &BTreeSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|idx| format!("{base}-{idx}"))
        .find(|id| !existing.contains(id))
        .unwrap_or_else(|| format!("{base}-fallback"))
}

fn generated_stage(
    id: &str,
    kind: StageKind,
    tier: ProviderTier,
    depends_on: Vec<String>,
    task: &str,
) -> StageSpec {
    StageSpec {
        id: id.to_string(),
        kind,
        task: Some(task.to_string()),
        agent: Some(id.to_string()),
        foreach: None,
        reducer: None,
        tool: None,
        condition: None,
        depends_on,
        provider_tier: Some(tier),
        retry: RetryPolicy::default(),
        input: Value::Null,
        model: None,
        provider: None,
        expected_target_files: Vec::new(),
        verify_command: None,
        max_parallelism: None,
        item_kind: None,
        extra: BTreeMap::new(),
    }
}

fn implementation_target_inventory_stage(id: &str, original: &StageSpec) -> StageSpec {
    let mut stage = generated_stage(
        id,
        StageKind::Agent,
        ProviderTier::Coder,
        original.depends_on.clone(),
        &format!(
            "Inspect the task evidence for implementation stage `{}` and emit a structured target inventory. Return exactly {{\"items\": [...]}}. Each item must include a non-empty target_files array, the task to perform, and required_tests. Do not edit files in this inventory stage. If no concrete repository target files can be determined, emit {{\"items\": []}} so the implementation fan-out fails fast instead of applying unsafe writes.",
            original.id
        ),
    );
    stage.input = serde_json::json!({
        "implementation_stage_id": original.id.clone(),
        "implementation_task": original.task.clone(),
        "implementation_input": original.input.clone(),
        "implementation_extra": original.extra.clone(),
    });
    stage.extra.insert(
        "outputs".into(),
        Value::Array(vec![Value::String("items".into())]),
    );
    stage
}

fn remediation_inventory_stage(id: &str, depends_on: Vec<String>) -> StageSpec {
    let mut stage = generated_stage(
        id,
        StageKind::Agent,
        ProviderTier::Critic,
        depends_on,
        "Build a remediation inventory from failed, timeout, unverifiable, or residual adversarial findings. Emit exactly {\"items\": []} when there are no blockers. Each repair item must include finding_id, related_task_id, target_files, failure, required_fix, and required_tests.",
    );
    stage.extra.insert(
        "outputs".into(),
        Value::Array(vec![Value::String("items".into())]),
    );
    stage
        .extra
        .insert("deterministic_empty_items".into(), Value::Bool(true));
    stage
}

fn remediation_fanout_stage(id: &str, inventory_id: &str) -> StageSpec {
    let mut stage = generated_stage(
        id,
        StageKind::Fanout,
        ProviderTier::Coder,
        vec![inventory_id.to_string()],
        "Implement each remediation item by editing only target_files, adding or updating required focused tests, and reporting exact files changed. If the inventory is empty, no-op.",
    );
    stage.foreach = Some(format!("${{{inventory_id}.items}}"));
    stage.item_kind = Some(StageKind::Implementation);
    stage.max_parallelism = Some(1);
    stage
        .extra
        .insert("allow_empty_items".into(), Value::Bool(true));
    stage
}

fn post_remediation_tests_stage(id: &str, repair_id: &str, inventory_id: &str) -> StageSpec {
    let mut stage = generated_stage(
        id,
        StageKind::Agent,
        ProviderTier::Coder,
        vec![repair_id.to_string(), inventory_id.to_string()],
        "Run focused verification for remediation items. Return status: verified only when commands pass; otherwise return status: failed, failed_timeout, or unverifiable with exact commands and evidence.",
    );
    stage.extra.insert(
        "allowed_tools".into(),
        Value::Array(vec![
            Value::String("Read".into()),
            Value::String("Grep".into()),
            Value::String("Glob".into()),
            Value::String("Bash".into()),
        ]),
    );
    stage
}

fn post_remediation_review_stage(id: &str, depends_on: Vec<String>) -> StageSpec {
    generated_stage(
        id,
        StageKind::Agent,
        ProviderTier::Critic,
        depends_on,
        "Re-run adversarial verification after remediation. Return status: verified only if every original blocker is fixed or no remediation was needed. Return status: failed or unverifiable for remaining blockers.",
    )
}

fn remediation_report_stage(id: &str, tests_id: &str, review_id: &str) -> StageSpec {
    let mut stage = generated_stage(
        id,
        StageKind::Reduce,
        ProviderTier::Reducer,
        vec![tests_id.to_string(), review_id.to_string()],
        "Synthesize final post-remediation acceptance evidence and preserve any remaining blockers.",
    );
    stage.agent = None;
    stage.reducer = Some(crate::spec::ReducerKind::EvidenceWeightedReport);
    stage
}

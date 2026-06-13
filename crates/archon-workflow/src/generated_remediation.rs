use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::spec::{ProviderTier, RetryPolicy, StageKind, StageSpec, WorkflowSpec};

pub(super) fn ensure_generated_remediation_loop(spec: &mut WorkflowSpec) {
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

pub(super) fn implementation_target_inventory_stage(id: &str, original: &StageSpec) -> StageSpec {
    let mut stage = generated_stage(
        id,
        StageKind::Agent,
        ProviderTier::Coder,
        original.depends_on.clone(),
        &format!(
            "Inspect the task evidence for implementation stage `{}` and emit a structured target inventory. Return exactly {{\"items\": [...]}}. Each item must include a non-empty target_files array, the task to perform, and required_tests. Do not edit files in this inventory stage. Emit {{\"items\": []}} only when upstream evidence says the implementation stage has no missing work. If work remains but concrete target files cannot be determined, return a blocked explanation without an `items` array so the implementation fan-out fails fast instead of applying unsafe writes.",
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

pub(super) fn unique_stage_id(base: &str, existing: &BTreeSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|idx| format!("{base}-{idx}"))
        .find(|id| !existing.contains(id))
        .unwrap_or_else(|| format!("{base}-fallback"))
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

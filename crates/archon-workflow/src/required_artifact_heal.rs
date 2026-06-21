use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::required_artifacts::{REQUIRED_ARTIFACT_INVENTORY_TOOL, required_artifact_paths};
use crate::spec::{ProviderTier, ReducerKind, RetryPolicy, StageKind, StageSpec, WorkflowSpec};

pub(crate) fn ensure_required_artifact_self_heal(spec: &mut WorkflowSpec) {
    if has_artifact_heal(spec) {
        return;
    }
    let Some(gate_idx) = final_required_artifact_gate(spec) else {
        return;
    };
    let gate_id = spec.stages[gate_idx].id.clone();
    let required = required_artifact_paths(&spec.stages[gate_idx]);
    if required.is_empty() {
        return;
    }
    let original_gate_deps = spec.stages[gate_idx].depends_on.clone();
    let existing = spec
        .stages
        .iter()
        .map(|stage| stage.id.clone())
        .collect::<BTreeSet<_>>();
    let ids = ArtifactHealIds::new(&existing);
    let stages = artifact_heal_stages(&ids, original_gate_deps.clone(), required);
    for (offset, stage) in stages.into_iter().enumerate() {
        spec.stages.insert(gate_idx + offset, stage);
    }
    spec.stages[gate_idx + 5].depends_on = vec![ids.report];
    spec.stages[gate_idx + 5].extra.insert(
        "self_heal_required_artifacts".into(),
        Value::String(gate_id),
    );
}

pub(crate) fn self_heal_requested(spec: &WorkflowSpec) -> bool {
    spec.stages.iter().any(|stage| {
        bool_field(stage, "enable_required_artifact_self_heal")
            || bool_field(stage, "self_heal_required_artifacts")
    })
}

fn final_required_artifact_gate(spec: &WorkflowSpec) -> Option<usize> {
    spec.stages.iter().rposition(|stage| {
        stage.kind == StageKind::QualityGate && !required_artifact_paths(stage).is_empty()
    })
}

fn has_artifact_heal(spec: &WorkflowSpec) -> bool {
    spec.stages.iter().any(|stage| {
        stage.tool.as_deref() == Some(REQUIRED_ARTIFACT_INVENTORY_TOOL)
            || stage
                .extra
                .get("artifact_self_heal")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    })
}

fn bool_field(stage: &StageSpec, key: &str) -> bool {
    stage
        .extra
        .get(key)
        .or_else(|| stage.input.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

struct ArtifactHealIds {
    inventory: String,
    repair: String,
    tests: String,
    review: String,
    report: String,
}

impl ArtifactHealIds {
    fn new(existing: &BTreeSet<String>) -> Self {
        Self {
            inventory: unique_stage_id("required-artifact-inventory", existing),
            repair: unique_stage_id("repair-required-artifacts", existing),
            tests: unique_stage_id("post-artifact-repair-tests", existing),
            review: unique_stage_id("post-artifact-repair-review", existing),
            report: unique_stage_id("post-artifact-repair-report", existing),
        }
    }
}

fn artifact_heal_stages(
    ids: &ArtifactHealIds,
    original_gate_deps: Vec<String>,
    required: Vec<String>,
) -> Vec<StageSpec> {
    vec![
        inventory_stage(&ids.inventory, original_gate_deps.clone(), required),
        repair_stage(&ids.repair, &ids.inventory, &original_gate_deps),
        tests_stage(&ids.tests, &ids.repair, &ids.inventory),
        review_stage(&ids.review, &ids.repair, &ids.tests, &ids.inventory),
        report_stage(&ids.report, &ids.tests, &ids.review, &original_gate_deps),
    ]
}

fn inventory_stage(id: &str, depends_on: Vec<String>, required: Vec<String>) -> StageSpec {
    let mut stage = base_stage(
        id,
        StageKind::Tool,
        ProviderTier::Local,
        depends_on,
        "Detect missing required workflow deliverables and emit implementation items for them.",
    );
    stage.tool = Some(REQUIRED_ARTIFACT_INVENTORY_TOOL.into());
    stage.input = json!({ "required_artifacts": required });
    stage.extra.insert(
        "outputs".into(),
        Value::Array(vec![Value::String("items".into())]),
    );
    mark_artifact_heal(&mut stage);
    stage
}

fn repair_stage(id: &str, inventory_id: &str, original_gate_deps: &[String]) -> StageSpec {
    let mut deps = vec![inventory_id.to_string()];
    push_unique_deps(&mut deps, original_gate_deps);
    let mut stage = base_stage(
        id,
        StageKind::Fanout,
        ProviderTier::Coder,
        deps,
        "Create or repair each missing required artifact. Use upstream PRD, task, test, and review evidence. Do not write placeholders. If the fanout item provides candidate_commands or repair_guidance, run the relevant command(s) before returning blocked. A blocked response must include artifact_path or resolved_path, reason or missing_evidence, and commands_run/attempted_commands/generation_attempts or command_discovery evidence with exact command, exit status, and output summary.",
    );
    stage.foreach = Some(format!("${{{inventory_id}.items}}"));
    stage.item_kind = Some(StageKind::Implementation);
    stage.max_parallelism = Some(1);
    stage
        .extra
        .insert("allow_empty_items".into(), Value::Bool(true));
    mark_artifact_heal(&mut stage);
    stage
}

fn tests_stage(id: &str, repair_id: &str, inventory_id: &str) -> StageSpec {
    let mut stage = base_stage(
        id,
        StageKind::Agent,
        ProviderTier::Coder,
        vec![repair_id.to_string(), inventory_id.to_string()],
        "Verify required artifact repairs using the inventory artifact's checked/resolved paths. Treat relative .archon/... deliverables as project-root artifacts, not repository-root artifacts. If inventory missing=[] and every checked entry exists=true, report status: verified. Otherwise inspect each missing target and run only focused checks needed for those artifacts.",
    );
    stage.extra.insert(
        "allowed_tools".into(),
        json!(["Read", "Grep", "Glob", "Bash"]),
    );
    mark_artifact_heal(&mut stage);
    stage
}

fn review_stage(id: &str, repair_id: &str, tests_id: &str, inventory_id: &str) -> StageSpec {
    let mut stage = base_stage(
        id,
        StageKind::Agent,
        ProviderTier::Critic,
        vec![
            repair_id.to_string(),
            tests_id.to_string(),
            inventory_id.to_string(),
        ],
        "Adversarially review required artifact repairs using the inventory artifact's checked/resolved paths, project_root, repository_root, and artifact_roots. Treat relative .archon/... deliverables as project-root artifacts unless the inventory resolved path says otherwise. Return status: verified only when every checked required deliverable exists at its resolved path and is non-placeholder.",
    );
    mark_artifact_heal(&mut stage);
    stage
}

fn report_stage(
    id: &str,
    tests_id: &str,
    review_id: &str,
    original_gate_deps: &[String],
) -> StageSpec {
    let mut deps = vec![tests_id.to_string(), review_id.to_string()];
    push_unique_deps(&mut deps, original_gate_deps);
    let mut stage = base_stage(
        id,
        StageKind::Reduce,
        ProviderTier::Reducer,
        deps,
        "Synthesize required-artifact repair evidence and preserve any remaining blockers.",
    );
    stage.agent = None;
    stage.reducer = Some(ReducerKind::EvidenceWeightedReport);
    mark_artifact_heal(&mut stage);
    stage
}

fn base_stage(
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
        filter: None,
        extra: BTreeMap::new(),
    }
}

fn mark_artifact_heal(stage: &mut StageSpec) {
    stage
        .extra
        .insert("artifact_self_heal".into(), Value::Bool(true));
}

fn push_unique_deps(deps: &mut Vec<String>, extra: &[String]) {
    for dep in extra {
        if !deps.contains(dep) {
            deps.push(dep.clone());
        }
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

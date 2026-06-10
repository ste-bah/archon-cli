use anyhow::{Context, Result, anyhow};
use archon_workflow::{
    ArtifactPolicy, ProviderTier, ReducerKind, RetryPolicy, StageKind, StageSpec, WorkflowSpec,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli_args::TradingCliWorkflowAction;

#[cfg(test)]
#[path = "trading_workflow_tests.rs"]
mod tests;

pub(crate) fn render_workflow(action: &TradingCliWorkflowAction) -> Result<String> {
    match action {
        TradingCliWorkflowAction::Plan {
            idea,
            repository,
            prd,
            tasks,
            kb,
            tradingview_replay,
            out,
        } => plan_workflow(WorkflowPlanInput {
            idea,
            repository,
            prd: prd.as_deref(),
            tasks: tasks.as_deref(),
            kb,
            tradingview_replay: *tradingview_replay,
            out,
        }),
    }
}

#[derive(Clone, Copy)]
struct WorkflowPlanInput<'a> {
    idea: &'a str,
    repository: &'a Path,
    prd: Option<&'a Path>,
    tasks: Option<&'a Path>,
    kb: &'a [String],
    tradingview_replay: bool,
    out: &'a Path,
}

fn plan_workflow(input: WorkflowPlanInput<'_>) -> Result<String> {
    if !input.repository.is_dir() {
        return Err(anyhow!(
            "repository does not exist: {}",
            input.repository.display()
        ));
    }
    let items = work_items(input)?;
    let spec = build_spec(input, items)?;
    let yaml = spec.to_yaml()?;
    if let Some(parent) = input.out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(input.out, yaml)
        .with_context(|| format!("failed to write {}", input.out.display()))?;
    Ok(format!(
        "Wrote Trading Lab workflow spec: {}\nRun it with: archon workflow run --spec-file {} --live",
        input.out.display(),
        input.out.display()
    ))
}

fn build_spec(input: WorkflowPlanInput<'_>, items: Vec<Value>) -> Result<WorkflowSpec> {
    let mut tiers = BTreeMap::new();
    tiers.insert(ProviderTier::Researcher, "auto".to_string());
    tiers.insert(ProviderTier::Coder, "auto".to_string());
    tiers.insert(ProviderTier::Critic, "auto".to_string());
    tiers.insert(ProviderTier::Reducer, "auto".to_string());
    let spec = WorkflowSpec {
        schema: "archon.workflow.v1".into(),
        name: "trading-lab-end-to-end".into(),
        task: root_task(input),
        target_repository_root: Some(input.repository.display().to_string()),
        max_parallelism: 4,
        max_agents: 64,
        provider_tiers: tiers,
        stages: vec![
            stage(
                "research-strategy-thesis",
                StageKind::Agent,
                ProviderTier::Researcher,
                vec![],
                research_task(input),
                json!({ "kb": input.kb, "prd": path_value(input.prd) }),
            ),
            implementation_fanout(items),
            stage(
                "adversarial-review",
                StageKind::Agent,
                ProviderTier::Critic,
                vec!["implement-trading-lab-workitems"],
                review_task(input),
                json!({
                    "repository": input.repository.display().to_string(),
                    "prd": path_value(input.prd),
                    "tasks": path_value(input.tasks)
                }),
            ),
            remediation_inventory(input),
            remediation_fanout(),
            post_remediation_tests(input),
            post_remediation_review(input),
            acceptance_report(),
            quality_gate("acceptance-report"),
        ],
        artifact_policy: ArtifactPolicy::default(),
        permissions: permissions(input.repository),
        quality_gates: BTreeMap::new(),
        learning_hooks: vec![
            "sona".into(),
            "reasoning_bank".into(),
            "reflexion".into(),
            "world_model".into(),
        ],
    };
    spec.validate()?;
    Ok(spec)
}

fn stage(
    id: &str,
    kind: StageKind,
    tier: ProviderTier,
    depends_on: Vec<&str>,
    task: String,
    input: Value,
) -> StageSpec {
    StageSpec {
        id: id.into(),
        kind,
        task: Some(task),
        agent: Some(id.into()),
        foreach: None,
        reducer: None,
        tool: None,
        condition: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        provider_tier: Some(tier),
        retry: RetryPolicy {
            max_attempts: 2,
            base_delay_ms: 1_000,
        },
        input,
        model: None,
        provider: None,
        expected_target_files: Vec::new(),
        verify_command: None,
        max_parallelism: None,
        item_kind: None,
        extra: BTreeMap::new(),
    }
}

fn implementation_fanout(items: Vec<Value>) -> StageSpec {
    let mut stage = stage(
        "implement-trading-lab-workitems",
        StageKind::Fanout,
        ProviderTier::Coder,
        vec!["research-strategy-thesis"],
        "Implement each structured Trading Lab work item. Read the PRD, task file, target files, and upstream thesis first. Modify only the declared target_files. Keep changed files under 500 lines, keep new function CCN <= 15, preserve provider-neutral execution, update tests/docs for the item, and report exact files changed.".into(),
        json!({ "items": items }),
    );
    stage.item_kind = Some(StageKind::Implementation);
    stage.max_parallelism = Some(2);
    stage.retry.max_attempts = 1;
    stage
}

fn quality_gate(depends_on: &str) -> StageSpec {
    let mut gate = stage(
        "trading-lab-quality",
        StageKind::QualityGate,
        ProviderTier::Critic,
        vec![depends_on],
        "Reject if post-remediation evidence, tests, docs, or safety gates are missing.".into(),
        json!({ "threshold": 0.75 }),
    );
    gate.agent = None;
    gate
}

fn remediation_inventory(input: WorkflowPlanInput<'_>) -> StageSpec {
    let mut stage = stage(
        "remediation-inventory",
        StageKind::Agent,
        ProviderTier::Critic,
        vec!["adversarial-review"],
        format!(
            "Read the adversarial review and emit a JSON/YAML object of the exact form {{\"items\": [...]}}. Create one item per failed, unverifiable, timeout, file-size, provider-neutrality, missing-test, or residual blocker. Each item must include finding_id, related_task_id, target_files, failure, required_fix, and required_tests. If there are no blockers, emit exactly {{\"items\": []}}. Use PRD {}, tasks {}, and repository evidence; do not invent target files.",
            path_label(input.prd),
            path_label(input.tasks)
        ),
        json!({
            "repository": input.repository.display().to_string(),
            "prd": path_value(input.prd),
            "tasks": path_value(input.tasks)
        }),
    );
    stage.extra.insert("outputs".into(), json!(["items"]));
    stage
        .extra
        .insert("deterministic_empty_items".into(), json!(true));
    stage
}

fn remediation_fanout() -> StageSpec {
    let mut stage = stage(
        "remediate-failed-findings",
        StageKind::Fanout,
        ProviderTier::Coder,
        vec!["remediation-inventory"],
        "For each remediation item, inspect the cited failure, edit only target_files, add or update required focused tests, and report exact files changed. Do not broaden scope beyond the finding. If no remediation items exist, this stage is a no-op.".into(),
        json!({ "allow_empty_items": true }),
    );
    stage.foreach = Some("${remediation-inventory.items}".into());
    stage.item_kind = Some(StageKind::Implementation);
    stage.max_parallelism = Some(1);
    stage.retry.max_attempts = 1;
    stage.extra.insert("allow_empty_items".into(), json!(true));
    stage
}

fn post_remediation_tests(input: WorkflowPlanInput<'_>) -> StageSpec {
    let mut stage = stage(
        "post-remediation-focused-tests",
        StageKind::Agent,
        ProviderTier::Coder,
        vec!["remediate-failed-findings", "remediation-inventory"],
        "Run the focused tests and checks required by the remediation items. If no remediation items exist, verify that the initial adversarial review was clean. Return structured status: verified only when commands pass; failed, failed_timeout, or unverifiable when they do not. Include exact commands and output summaries.".into(),
        json!({
            "repository": input.repository.display().to_string(),
            "prd": path_value(input.prd),
            "tasks": path_value(input.tasks)
        }),
    );
    stage.extra.insert(
        "allowed_tools".into(),
        json!(["Read", "Grep", "Glob", "Bash"]),
    );
    stage
}

fn post_remediation_review(input: WorkflowPlanInput<'_>) -> StageSpec {
    stage(
        "post-remediation-adversarial-review",
        StageKind::Agent,
        ProviderTier::Critic,
        vec![
            "adversarial-review",
            "remediation-inventory",
            "remediate-failed-findings",
            "post-remediation-focused-tests",
        ],
        "Re-run adversarial verification against the original findings and remediated code. Return status: verified only if every blocker is fixed or no remediation was needed. Return status: failed or unverifiable with concrete file/test evidence for any remaining blocker. Do not quote stale failed JSON as the current status unless the failure still exists.".into(),
        json!({
            "repository": input.repository.display().to_string(),
            "prd": path_value(input.prd),
            "tasks": path_value(input.tasks)
        }),
    )
}

fn acceptance_report() -> StageSpec {
    let mut stage = stage(
        "acceptance-report",
        StageKind::Reduce,
        ProviderTier::Reducer,
        vec![
            "post-remediation-focused-tests",
            "post-remediation-adversarial-review",
        ],
        "Synthesize final acceptance from post-remediation test and review evidence only. Preserve remaining blockers; do not claim completion when any current evidence reports failed, timeout, or unverifiable status.".into(),
        json!({}),
    );
    stage.agent = None;
    stage.reducer = Some(ReducerKind::EvidenceWeightedReport);
    stage
}

fn work_items(input: WorkflowPlanInput<'_>) -> Result<Vec<Value>> {
    if let Some(tasks) = input.tasks {
        return task_file_items(tasks, input.prd);
    }
    Ok(default_lifecycle_items(input.tradingview_replay))
}

fn task_file_items(tasks: &Path, prd: Option<&Path>) -> Result<Vec<Value>> {
    let mut files = task_files(tasks)?;
    if files.is_empty() {
        return Err(anyhow!("no TASK*.md files found in {}", tasks.display()));
    }
    files.sort();
    files
        .into_iter()
        .map(|path| task_item(&path, prd))
        .collect()
}

fn task_files(tasks: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(tasks)
        .with_context(|| format!("failed to read task directory {}", tasks.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("TASK") && name.ends_with(".md") {
            files.push(path);
        }
    }
    Ok(files)
}

fn task_item(path: &Path, prd: Option<&Path>) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read task file {}", path.display()))?;
    let target_files = yaml_list(&text, "target_files");
    if target_files.is_empty() {
        return Err(anyhow!("task file has no target_files: {}", path.display()));
    }
    let id = yaml_scalar(&text, "task_id").unwrap_or_else(|| {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("task")
            .to_string()
    });
    Ok(json!({
        "id": id,
        "task_file": path.display().to_string(),
        "prd": path_value(prd),
        "target_files": target_files,
        "instructions": "Implement this task exactly; inspect existing code first; run focused tests; do not defer declared requirements."
    }))
}

fn yaml_scalar(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(clean_yaml_value))
}

fn yaml_list(text: &str, key: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut in_list = false;
    let prefix = format!("{key}:");
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == prefix {
            in_list = true;
            continue;
        }
        if in_list && let Some(value) = trimmed.strip_prefix("- ") {
            values.push(clean_yaml_value(value));
            continue;
        }
        if in_list && !trimmed.is_empty() && !line.starts_with(' ') {
            break;
        }
    }
    values
}

fn clean_yaml_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn default_lifecycle_items(tradingview_replay: bool) -> Vec<Value> {
    let mut items = vec![
        lifecycle_item("strategy-spec", ".archon/trading-lab/strategy-spec.json"),
        lifecycle_item("pine-prototypes", ".archon/trading-lab/pine/manifest.json"),
        lifecycle_item(
            "openbb-dataset",
            ".archon/trading-lab/data/dataset-report.json",
        ),
        lifecycle_item(
            "native-backtest",
            ".archon/trading-lab/backtest/report.json",
        ),
        lifecycle_item("paper-order", ".archon/trading-lab/paper/report.json"),
        lifecycle_item(
            "promotion-check",
            ".archon/trading-lab/promotion/report.json",
        ),
    ];
    if tradingview_replay {
        items.push(lifecycle_item(
            "tradingview-replay-paper",
            ".archon/trading-lab/paper/tradingview-replay.json",
        ));
    }
    items
}

fn lifecycle_item(id: &str, target: &str) -> Value {
    json!({
        "id": id,
        "target_files": [target],
        "instructions": "Create the declared Trading Lab artifact from upstream evidence and validate it with the matching archon trading command."
    })
}

fn root_task(input: WorkflowPlanInput<'_>) -> String {
    format!(
        "Trading Lab workflow for: {}\nRepository: {}\nPRD: {}\nTasks: {}",
        input.idea,
        input.repository.display(),
        path_label(input.prd),
        path_label(input.tasks)
    )
}

fn research_task(input: WorkflowPlanInput<'_>) -> String {
    format!(
        "Research the trading idea using only source-backed evidence and these KBs: {}. Produce exact rules, invalidation, risk, data, backtest, Pine, paper, and promotion constraints. Do not claim profitability.",
        if input.kb.is_empty() {
            "none supplied".into()
        } else {
            input.kb.join(", ")
        }
    )
}

fn review_task(input: WorkflowPlanInput<'_>) -> String {
    format!(
        "Adversarially review the Trading Lab implementation against {}, {}, and all generated artifacts. List every residual gap with file paths.",
        path_label(input.prd),
        path_label(input.tasks)
    )
}

fn permissions(repository: &Path) -> BTreeMap<String, Value> {
    let mut permissions = BTreeMap::new();
    permissions.insert(
        "filesystem".into(),
        json!({ "allowed_roots": [repository.display().to_string()], "write": true }),
    );
    permissions
}

fn path_value(path: Option<&Path>) -> Value {
    path.map(|path| json!(path.display().to_string()))
        .unwrap_or(Value::Null)
}

fn path_label(path: Option<&Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "not supplied".into())
}

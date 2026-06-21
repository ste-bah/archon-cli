use archon_workflow::{WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStore};

#[test]
fn generated_final_gate_records_candidate_artifacts_from_prd_layout() {
    let temp = tempfile::tempdir().unwrap();
    let prd = temp.path().join("PRD.md");
    std::fs::write(&prd, prd_with_required_layouts()).unwrap();
    let spec = WorkflowSpec::from_generated_yaml(&workflow_yaml(&prd), "Fallback task").unwrap();
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "quality")
        .unwrap();
    let artifacts = required_artifacts(gate);
    let candidates = candidate_artifacts(gate);

    assert!(
        artifacts.is_empty(),
        "inferred markdown layout paths must not become hard required artifacts: {artifacts:?}"
    );
    assert_contains(&candidates, ".archon/trading-lab/data/registry.json");
    assert_contains(
        &candidates,
        ".archon/trading-lab/data/provider-capabilities.json",
    );
    assert_contains(&candidates, ".archon/trading-lab/data/coverage/latest.json");
    assert_contains(&candidates, ".archon/trading-lab/data/coverage/latest.md");
    assert_contains(
        &candidates,
        ".archon/trading-lab/data/datasets/*/*/metadata.json",
    );
    assert_contains(
        &candidates,
        ".archon/trading-lab/data/datasets/*/*/ohlcv.jsonl",
    );
    assert_contains(&candidates, ".archon/trading-lab/data/datasets/*/*/raw.csv");
    assert_contains(
        &candidates,
        "/tmp/archon-project/.archon/trading-lab/data/datasets/*/*/validation-report.json",
    );
    assert_contains(
        &candidates,
        "/tmp/archon-project/.archon/trading-lab/strategies/AHDM-v1/backtests/*/trades.jsonl",
    );
    assert_contains(
        &candidates,
        ".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json",
    );
    assert_contains(
        &candidates,
        ".archon/trading-lab/strategies/AHDM-v1/evidence/kb-rule-inventory.md",
    );
    assert_contains(
        &candidates,
        ".archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-indicator.pine",
    );
    assert_contains(
        &candidates,
        ".archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md",
    );
    assert_contains(
        &candidates,
        ".archon/trading-lab/strategies/AHDM-v1/backtests/*/report.json",
    );
    assert!(
        !candidates
            .iter()
            .any(|artifact| artifact.contains("<timestamp>") || artifact.contains("<run-id>")),
        "candidate_artifacts={candidates:?}"
    );
    assert_eq!(gate.depends_on, vec!["post-remediation-acceptance-report"]);
}

#[test]
fn executor_start_infers_artifact_contract_for_explicit_specs() {
    let temp = tempfile::tempdir().unwrap();
    let prd = temp.path().join("PRD.md");
    std::fs::write(&prd, prd_with_required_layouts()).unwrap();
    let yaml = format!(
        r#"
schema: archon.workflow.v1
name: explicit-with-prd-artifacts
task: Implement the PRD at {prd}.
stages:
  - id: quality
    kind: quality_gate
"#,
        prd = prd.display()
    );
    let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
    let executor = WorkflowExecutor::new(WorkflowStore::project(temp.path()), permissive_policy());

    let run = executor.start(spec).unwrap();
    let gate = run
        .spec
        .stages
        .iter()
        .find(|stage| stage.id == "quality")
        .unwrap();
    let artifacts = required_artifacts(gate);
    let candidates = candidate_artifacts(gate);

    assert!(
        artifacts.is_empty(),
        "executor start must not promote inferred candidates into hard gates"
    );
    assert_contains(&candidates, ".archon/trading-lab/data/registry.json");
    assert_contains(
        &candidates,
        ".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json",
    );
    assert!(gate.depends_on.is_empty());
}

#[test]
fn referenced_task_directory_does_not_starve_explicit_prd_file() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks");
    std::fs::create_dir(&tasks).unwrap();
    for index in 0..32 {
        std::fs::write(
            tasks.join(format!("TASK-{index:02}.md")),
            "No artifacts here.",
        )
        .unwrap();
    }
    std::fs::create_dir(tasks.join("context")).unwrap();
    std::fs::write(tasks.join("context").join("activeContext.md"), "Context.").unwrap();
    let prd = temp.path().join("PRD.md");
    std::fs::write(&prd, prd_with_required_layouts()).unwrap();

    let spec = WorkflowSpec::from_generated_yaml(
        &workflow_yaml_with_task_dir_then_prd(&tasks, &prd),
        "Fallback task",
    )
    .unwrap();
    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "quality")
        .unwrap();
    let artifacts = required_artifacts(gate);
    let candidates = candidate_artifacts(gate);

    assert!(artifacts.is_empty());
    assert_contains(&candidates, ".archon/trading-lab/data/registry.json");
    assert_contains(
        &candidates,
        ".archon/trading-lab/data/datasets/*/*/metadata.json",
    );
}

fn workflow_yaml(prd: &std::path::Path) -> String {
    format!(
        r#"
schema: archon.workflow.v1
name: generated-with-prd-artifacts
task: Implement the PRD at {prd}.
stages:
  - id: implement
    kind: agent
    task: Implement missing work.
  - id: review
    kind: agent
    provider_tier: critic
    depends_on: [implement]
  - id: quality
    kind: quality_gate
    depends_on: [review]
"#,
        prd = prd.display()
    )
}

fn workflow_yaml_with_task_dir_then_prd(tasks: &std::path::Path, prd: &std::path::Path) -> String {
    format!(
        r#"
schema: archon.workflow.v1
name: generated-with-task-dir-first
task: Implement the decomposed PRD at {tasks}. Read {prd} and every task/context/spec file.
stages:
  - id: quality
    kind: quality_gate
"#,
        tasks = tasks.display(),
        prd = prd.display()
    )
}

fn prd_with_required_layouts() -> &'static str {
    r#"
Required layout:

Absolute examples from generated task inventories must also be enforced:

- /tmp/archon-project/.archon/trading-lab/data/datasets/<dataset-id>/<version>/validation-report.json
- /tmp/archon-project/.archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/trades.jsonl

```text
.archon/trading-lab/data/
  registry.json
  provider-capabilities.json
  datasets/
    <dataset-id>/
      <version>/
        metadata.json
        ohlcv.jsonl
        raw.csv
  coverage/
    latest.json
    latest.md
    history/<timestamp>.json
```

AHDM-v1 artifacts must live under:

```text
.archon/trading-lab/strategies/AHDM-v1/
  strategy-spec.json
  evidence/
    kb-rule-inventory.md
    citations.json
  pine/
    AHDM-v1-indicator.pine
    AHDM-v1-strategy.pine
  backtests/
    <run-id>/
      report.json
  readiness/
    paper-trading-readiness.md
```
"#
}

fn required_artifacts(stage: &archon_workflow::StageSpec) -> Vec<&str> {
    stage
        .extra
        .get("required_artifacts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect()
}

fn candidate_artifacts(stage: &archon_workflow::StageSpec) -> Vec<&str> {
    stage
        .extra
        .get("workflow_contracts")
        .and_then(|contracts| contracts.get("candidate_artifacts"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect()
}

fn assert_contains(values: &[&str], expected: &str) {
    assert!(
        values.contains(&expected),
        "missing {expected}; got {values:?}"
    );
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

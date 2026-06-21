use std::fs;

use archon_workflow::{StageKind, WorkflowSpec};
use serde_json::Value;

#[test]
fn generated_targetless_implementation_uses_task_file_targets() {
    let temp = tempfile::tempdir().unwrap();
    let task_dir = temp.path().join("tasks/PRD-TDL");
    fs::create_dir_all(&task_dir).unwrap();
    let task_file = task_dir.join("TASK-TDL-001-data-lake-gap-audit.md");
    fs::write(
        &task_file,
        r#"# TASK-TDL-001 — Data Lake Gap Audit

```yaml
task_id: TASK-TDL-001
```

## Files Expected to Change

- `crates/archon-trading/src/data_lake.rs`
- `src/command/trading_data.rs`

## Focused Tests

- `cargo test -p archon-trading data_lake`
"#,
    )
    .unwrap();
    let readme = task_dir.join("README.md");
    fs::write(&readme, "# PRD task pack").unwrap();
    let yaml = format!(
        r#"
schema: archon.workflow.v1
name: generated-task-item-targets
task: |
  Implement the decomposed PRD at {}.
  Read: {}
stages:
  - id: discovery
    kind: agent
  - id: wave1_t001
    kind: implementation
    task: Plan and implement only missing work for T001.
    provider_tier: coder
    depends_on: [discovery]
"#,
        task_dir.display(),
        readme.display()
    );

    let spec = WorkflowSpec::from_generated_yaml(&yaml, "fallback").unwrap();
    assert!(
        spec.stages
            .iter()
            .all(|stage| stage.id != "wave1_t001-target-inventory")
    );
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_t001")
        .unwrap();
    assert_eq!(stage.kind, StageKind::Fanout);
    assert_eq!(stage.item_kind, Some(StageKind::Implementation));
    assert!(stage.foreach.is_none());
    let items = stage.input.get("items").and_then(Value::as_array).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("target_files").unwrap(),
        &serde_json::json!([
            "crates/archon-trading/src/data_lake.rs",
            "src/command/trading_data.rs"
        ])
    );
    assert_eq!(
        items[0].get("required_tests").unwrap(),
        &serde_json::json!(["`cargo test -p archon-trading data_lake`"])
    );
}

#[test]
fn generated_targetless_implementation_without_task_targets_uses_inventory() {
    let yaml = r#"
schema: archon.workflow.v1
name: generated-no-task-targets
task: Implement the decomposed PRD.
stages:
  - id: discovery
    kind: agent
  - id: wave1_t001
    kind: implementation
    task: Plan and implement only missing work for T001.
    provider_tier: coder
    depends_on: [discovery]
"#;

    let spec = WorkflowSpec::from_generated_yaml(yaml, "fallback").unwrap();
    assert!(
        spec.stages
            .iter()
            .any(|stage| stage.id == "wave1_t001-target-inventory")
    );
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == "wave1_t001")
        .unwrap();
    assert_eq!(
        stage.foreach.as_deref(),
        Some("${wave1_t001-target-inventory.items}")
    );
}

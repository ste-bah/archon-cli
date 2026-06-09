use std::path::PathBuf;

use serde_json::Value;

use super::*;

#[test]
fn generated_trading_workflow_has_bounded_remediation_loop() {
    let repo = temp_repo();
    let spec = build_spec(
        WorkflowPlanInput {
            idea: "test idea",
            repository: &repo,
            prd: None,
            tasks: None,
            kb: &[],
            tradingview_replay: false,
            out: &repo.join("workflow.yaml"),
        },
        default_lifecycle_items(false),
    )
    .unwrap();

    let ids = spec
        .stages
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "research-strategy-thesis",
            "implement-trading-lab-workitems",
            "adversarial-review",
            "remediation-inventory",
            "remediate-failed-findings",
            "post-remediation-focused-tests",
            "post-remediation-adversarial-review",
            "acceptance-report",
            "trading-lab-quality",
        ]
    );

    let remediation = spec
        .stages
        .iter()
        .find(|stage| stage.id == "remediate-failed-findings")
        .unwrap();
    assert_eq!(
        remediation.foreach.as_deref(),
        Some("${remediation-inventory.items}")
    );
    assert_eq!(remediation.item_kind, Some(StageKind::Implementation));
    assert_eq!(
        remediation
            .extra
            .get("allow_empty_items")
            .and_then(Value::as_bool),
        Some(true)
    );

    let inventory = spec
        .stages
        .iter()
        .find(|stage| stage.id == "remediation-inventory")
        .unwrap();
    assert_eq!(
        inventory
            .extra
            .get("outputs")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_str),
        Some("items")
    );
    assert_eq!(
        inventory
            .extra
            .get("deterministic_empty_items")
            .and_then(Value::as_bool),
        Some(true)
    );

    let gate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "trading-lab-quality")
        .unwrap();
    assert_eq!(gate.depends_on, vec!["acceptance-report"]);
    assert!(
        !gate.depends_on.contains(&"adversarial-review".to_string()),
        "final gate must not depend on stale pre-remediation review"
    );
}

fn temp_repo() -> PathBuf {
    let unique = format!(
        "archon-trading-workflow-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).unwrap();
    path
}

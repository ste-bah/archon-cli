//! Batch O: the coverage audit's inventory comes from the host, and the host
//! itself adds the requirements no reviewed task claims.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

const PRD: &str = "# Widget Service\n\n## 8. Requirements\n\n- REQ-WS-001: Widgets are stored with provenance.\n- REQ-WS-002: Missing provenance fails closed.\n- REQ-WS-003: Stale widgets are refused.\n";

/// A project holding `prds/widget.md` and the task set `tasks/set/`.
fn project() -> (tempfile::TempDir, WorkflowV2TaskUniverse) {
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("tasks/set");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::create_dir_all(dir.path().join("prds")).unwrap();
    std::fs::write(dir.path().join("prds/widget.md"), PRD).unwrap();
    std::fs::write(
        set.join(ACCEPTANCE_CONTRACT_FILE),
        json!({
            "schema_version": 1,
            "prd": {"path": "prds/widget.md", "digest": "d"},
            "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
            "acceptance": [], "supplementary": []
        })
        .to_string(),
    )
    .unwrap();
    let task = |id: &str, implements: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        implements: implements.iter().map(|i| i.to_string()).collect(),
        ..Default::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![set.display().to_string()],
        tasks: vec![task("T-1", &["REQ-WS-001"]), task("T-2", &["REQ-WS-002"])],
    };
    (dir, universe)
}

#[test]
fn the_view_carries_every_requirement_and_each_tasks_claims() {
    let (_dir, universe) = project();
    let view = script_view(Some(&universe));
    assert_eq!(view["source"], "host");
    assert_eq!(view["requirements"].as_array().unwrap().len(), 3);
    assert_eq!(view["by_task"]["T-1"], json!(["REQ-WS-001"]));
    assert_eq!(view["unclaimed"], json!(["REQ-WS-003"]));
    assert!(script_view(None)["unavailable"].is_string());
}

#[test]
fn the_host_adds_unclaimed_and_unreviewed_requirements() {
    let (_dir, universe) = project();
    // T-2 was blocked, so the audit never reviewed it.
    let findings = inventory_findings(Some(&universe), &BTreeSet::from(["T-1".to_string()]));
    let ids: Vec<&str> = findings.iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["unreviewed-claim-REQ-WS-002", "unclaimed-REQ-WS-003"]);
    assert_eq!(findings[0]["canonical_task_ids"], json!(["T-2"]));
    assert!(findings[1].get("canonical_task_ids").is_none());
}

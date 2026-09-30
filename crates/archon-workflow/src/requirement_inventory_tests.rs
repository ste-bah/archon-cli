//! Batch O: the host's requirement inventory -- made-up PRD, made-up domain.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

const PRD: &str = r#"
# Widget Service

## 8. Requirements

- REQ-WS-001: Widgets are stored with provenance.
- REQ-WS-002 — Missing provenance fails closed.
- REQ-WS-003: Stale widgets are refused.

## 12. Acceptance Criteria

| ID | Acceptance criterion |
|---|---|
| AC-WS-001 | Native widget ingestion stores a raw artifact and a registry entry. |
"#;

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, implements: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        implements: implements.iter().map(|i| i.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-WS-001", &["REQ-WS-001", "AC-WS-001"]),
            task("TASK-WS-002", &["REQ-WS-001", "REQ-WS-404"]),
            task("TASK-WS-003", &[]),
        ],
    }
}

fn contract() -> AcceptanceContract {
    serde_json::from_value(json!({
        "schema_version": 1,
        "prd": {"path": "prd.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-WS-001", "criterion": "c",
            "check": {"kind": "command", "command": "true", "cwd": "project_root"},
            "gap_permitted": false,
            "judgment": {"verdict": "accepted", "counterexample": "x", "reason": "y", "host_call_id": "z"},
            "covers": ["REQ-WS-002"]
        }],
        "supplementary": []
    }))
    .expect("a contract")
}

#[test]
fn the_inventory_names_every_requirement_its_claims_its_checks_and_every_gap() {
    let inventory = requirement_inventory(PRD, &universe(), Some(&contract()));
    let ids: Vec<&str> = inventory["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["AC-WS-001", "REQ-WS-001", "REQ-WS-002", "REQ-WS-003"]);
    let req1 = &inventory["requirements"][1];
    assert_eq!(req1["text"], "Widgets are stored with provenance.");
    assert_eq!(req1["claimed_by"], json!(["TASK-WS-001", "TASK-WS-002"]));
    assert_eq!(
        inventory["requirements"][2]["checked_by"],
        json!(["AC-WS-001"])
    );
    assert_eq!(inventory["claim_map"]["AC-WS-001"], json!(["TASK-WS-001"]));
    assert_eq!(inventory["unclaimed"], json!(["REQ-WS-002", "REQ-WS-003"]));
    assert_eq!(inventory["unchecked"], json!(["REQ-WS-001", "REQ-WS-003"]));
    assert_eq!(
        inventory["phantom_claims"],
        json!([{"task_id": "TASK-WS-002", "id": "REQ-WS-404"}])
    );
    assert_eq!(inventory["tasks_without_claims"], json!(["TASK-WS-003"]));
    assert_eq!(inventory["check_ids"], json!(["AC-WS-001"]));
}

#[test]
fn without_a_contract_every_requirement_is_unchecked() {
    let dir = tempfile::tempdir().unwrap();
    let prd = dir.path().join("prd.md");
    std::fs::write(&prd, PRD).unwrap();
    let inventory = requirement_inventory_from_files(&prd, dir.path(), &universe()).unwrap();
    assert_eq!(inventory["unchecked"].as_array().unwrap().len(), 4);
    std::fs::write(dir.path().join(ACCEPTANCE_CONTRACT_FILE), "{").unwrap();
    assert!(requirement_inventory_from_files(&prd, dir.path(), &universe()).is_err());
}

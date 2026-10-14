use super::*;

#[test]
fn workflow_status_detail_reports_unknown_usage_cost_as_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(test_spec()).unwrap();
    store
        .write_run_json(
            &run.id,
            "agent-outputs/stage/item.json",
            &serde_json::json!({
                "stage_id": "stage",
                "item_id": "item",
                "status": "complete",
                "provider": "provider",
                "model": "model",
                "tokens_in": null,
                "tokens_out": null,
                "cost_usd": 0.0,
                "body": "result"
            }),
        )
        .unwrap();
    let source_path = store.run_dir(&run.id).join("agent-outputs/stage/item.json");
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&source_path).unwrap()).unwrap();
    assert!(persisted["tokens_in"].is_null());
    assert!(persisted["tokens_out"].is_null());
    assert_eq!(persisted["cost_usd"], 0.0);

    let detail = crate::command::workflow::status_detail_text(&store, &run.id).unwrap();

    assert!(detail.contains("tokens=None/None cost=unknown"), "{detail}");
    let output_line = detail
        .lines()
        .find(|line| line.contains("stage=stage item=item"))
        .expect("agent output line");
    eprintln!(
        "EVIDENCE #369 workflow status ACTION: rendered persisted agent output with unknown token counts and default zero cost; EXPECTED cost=unknown; ACTUAL: {output_line}; SOURCE READ: {} contains tokens_in={}, tokens_out={}, cost_usd={}",
        source_path.display(),
        persisted["tokens_in"],
        persisted["tokens_out"],
        persisted["cost_usd"]
    );
}

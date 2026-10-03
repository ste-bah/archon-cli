//! Issue 248: v1 coverage reads agent output as produced, never the public,
//! log-redacted `agent-outputs/` copy.

use serde_json::json;

use super::bundles_from_agent_records;
use crate::events::REDACTION_MARKER;
use crate::runner::StageRunOutput;
use crate::store::WorkflowStore;

const COMMAND: &str = "curl -fsS -H token=\"x\" http://localhost/health";

fn record(store: &WorkflowStore, run_id: &str) {
    let body = json!({
        "work_unit_id": "WU-1",
        "status": "verified",
        "changed_files": ["src/health.rs"],
        "commands_run": [{ "command": COMMAND, "exit_status": 0 }],
    })
    .to_string();
    crate::persistence::record_captured_agent_output(
        store,
        run_id,
        "implement",
        "item-1",
        &StageRunOutput::markdown(body),
    )
    .expect("record agent output");
}

fn commands(store: &WorkflowStore, run_id: &str) -> Vec<String> {
    let bundles =
        bundles_from_agent_records(&store.run_dir(run_id), "implement", ["item-1".to_string()]);
    bundles["item-1"]
        .iter()
        .flat_map(|bundle| &bundle.evidence)
        .filter_map(|item| item.command.clone())
        .collect()
}

#[test]
fn an_agent_output_reaches_coverage_as_produced_not_log_redacted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path());
    record(&store, "run-1");
    assert_eq!(commands(&store, "run-1"), vec![COMMAND.to_string()]);
    // The public copy stays redacted for display.
    let public = std::fs::read_to_string(
        store
            .run_dir("run-1")
            .join("agent-outputs/implement/item-1.json"),
    )
    .expect("public copy");
    assert!(public.contains(REDACTION_MARKER), "{public}");
    assert!(!public.contains("token=\\\"x\\\""), "{public}");
}

/// A run recorded before the authoritative copy existed: its public copy is
/// read only when log redaction left no marker in it.
#[test]
fn a_legacy_public_copy_is_read_only_when_redaction_changed_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path());
    record(&store, "run-1");
    std::fs::remove_dir_all(store.run_dir("run-1").join(super::AGENT_RESULTS_DIR))
        .expect("drop the authoritative copy");
    assert!(commands(&store, "run-1").is_empty());
    let clean = json!({
        "body": { "work_unit_id": "WU-1", "status": "verified",
                  "commands_run": [{ "command": "cargo test", "exit_status": 0 }] },
    });
    let path = store
        .run_dir("run-1")
        .join("agent-outputs/implement/item-1.json");
    std::fs::write(&path, serde_json::to_vec(&clean).unwrap()).unwrap();
    assert_eq!(commands(&store, "run-1"), vec!["cargo test".to_string()]);
}

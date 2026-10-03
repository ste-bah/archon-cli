use super::*;
use archon_workflow::{HeuristicWorkflowPlanner, WorkflowPlanner, WorkflowStore};

#[test]
fn workflow_responses_preserve_history_damage_count() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let run = store
        .create_run(HeuristicWorkflowPlanner.plan("Audit codebase").unwrap())
        .unwrap();
    store.append_event_line(&run.id, "{damaged").unwrap();
    let summary = from_workflow_summary(archon_workflow::web_api::summary(&store, 10).unwrap());
    assert_eq!(
        serde_json::to_value(summary).unwrap()["damagedEventLines"],
        1
    );
    let detail = from_detail(archon_workflow::web_api::detail(&store, &run.id).unwrap());
    assert_eq!(
        serde_json::to_value(detail).unwrap()["damagedEventLines"],
        1
    );
}

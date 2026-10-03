//! Issue-256: a restart is refused while a live executor holds the run's
//! executor lease, and proceeds once the lease is free.

use super::*;
use crate::command::workflow_executor_lease;

fn run_with_stage(temp: &tempfile::TempDir) -> (WorkflowStore, WorkflowRun) {
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: restart-lease
task: test
stages:
  - id: build
    kind: agent
"#,
    )
    .expect("spec");
    let run = store.create_run(spec).expect("run");
    (store, run)
}

fn is_live_refusal(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("is live") && text.contains("restart")
}

#[test]
fn restart_task_is_refused_while_a_live_executor_holds_the_lease() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = run_with_stage(&temp);
    let lease = workflow_executor_lease::acquire(&store.run_dir(&run.id), &run.id).unwrap();
    let before = store.load_state(&run.id).unwrap();

    let refused = restart_task_workflow(&store, &run.id, "build").unwrap_err();
    assert!(is_live_refusal(&refused), "{refused:#}");
    assert_eq!(
        store.load_state(&run.id).unwrap().generation,
        before.generation,
        "a refused restart writes nothing"
    );

    drop(lease);
    restart_task_workflow(&store, &run.id, "build").expect("lease free: restart proceeds");
    assert!(store.load_state(&run.id).unwrap().generation > before.generation);
}

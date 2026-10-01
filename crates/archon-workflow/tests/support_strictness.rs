//! Batch O2 (m6): the test support that judges runs is no softer than the
//! live host it stands in for.
#[path = "support/acceptance_ran.rs"]
mod acceptance_ran;
#[path = "support/task_stage.rs"]
mod task_stage;

use std::collections::BTreeSet;

use archon_workflow::v2::acceptance_stage::{ACCEPTANCE_STAGE_CALL_PREFIX, ACCEPTANCE_STAGE_TOOL};
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredCallRole, authored_call_facts,
};
use archon_workflow::*;
use serde_json::json;

fn acceptance_call() -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert("tool".into(), json!(ACCEPTANCE_STAGE_TOOL));
    // The round the prelude names on every acceptance call.
    options.extra.insert("round".into(), json!(1));
    WorkflowV2HostCall {
        id: format!("{ACCEPTANCE_STAGE_CALL_PREFIX}1"),
        method: WorkflowV2HostMethod::Tool,
        write_mode: None,
        options,
    }
}

fn round(store: &WorkflowV2ResultStore, data: serde_json::Value) -> WorkflowV2HostCall {
    let call = acceptance_call();
    let result = WorkflowV2Result {
        data,
        ..WorkflowV2Result::accepted("acceptance round")
    };
    let record = WorkflowV2CallRecord::new("run", call.clone(), 1, "h".into(), result, vec![]);
    store.save_call_record(&record).unwrap();
    call
}

#[test]
fn a_recorded_round_the_run_never_listed_is_missing_never_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    let call = round(
        &store,
        json!({"final": true, "failing": [], "operational_errors": [], "contract_present": true}),
    );
    assert!(archon_workflow::v2::script::is_acceptance_stage_call(&call));
    let ran = acceptance_ran::AcceptanceRan::of(&store);
    // The live host binds a round to the acceptance call the run listed.
    let none = authored_call_facts(&[], |id| store.load_call_record(id)).unwrap();
    assert!(matches!(
        ran.fact(&none),
        AuthoredAcceptanceGateFact::Missing
    ));
    let listed = authored_call_facts(&[call], |id| store.load_call_record(id)).unwrap();
    assert!(matches!(
        ran.fact(&listed),
        AuthoredAcceptanceGateFact::Recorded { .. }
    ));
    // And a run with no round at all is Missing, never NotRequired.
    let empty = WorkflowV2ResultStore::new(dir.path().join("other/v2"));
    let ran = acceptance_ran::AcceptanceRan::of(&empty);
    assert!(matches!(
        ran.fact(&listed),
        AuthoredAcceptanceGateFact::Missing
    ));
}

#[test]
#[should_panic(expected = "records whether a contract was present")]
fn a_round_that_does_not_say_whether_a_contract_was_present_is_never_read_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    round(
        &store,
        json!({"final": true, "failing": [], "operational_errors": []}),
    );
    let _ = acceptance_ran::AcceptanceRan::of(&store);
}

#[test]
fn the_task_stage_is_recorded_and_read_back_by_the_production_facts() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    let tasks = BTreeSet::from(["TASK-A".to_string(), "TASK-B".to_string()]);
    let calls = task_stage::with_task_stage(&store, &tasks, &[]);
    let facts = authored_call_facts(&calls, |id| store.load_call_record(id)).unwrap();
    let roles: Vec<&AuthoredCallRole> = facts.iter().map(|fact| &fact.role).collect();
    assert_eq!(
        roles,
        [&AuthoredCallRole::Write, &AuthoredCallRole::TaskVerify]
    );
    for fact in &facts {
        assert_eq!(
            fact.tasks.keys().cloned().collect::<BTreeSet<_>>(),
            tasks,
            "{fact:?}"
        );
        assert!(
            fact.tasks
                .values()
                .all(|outcome| outcome.status == WorkflowV2Status::Accepted)
        );
    }
}

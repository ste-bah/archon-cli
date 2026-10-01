//! The host's terminal status for a remediation-only test's run, judged as
//! the live host judges it: on the acceptance round the prelude recorded
//! (REM-13, `acceptance_ran`), after the task stage the test does not run
//! (REM-14, `task_stage`).
#![allow(dead_code)]
#[path = "acceptance_ran.rs"]
mod acceptance_ran;
#[path = "task_stage.rs"]
mod task_stage;

use std::collections::BTreeSet;

use archon_workflow::v2::script::{
    AuthoredRunFacts, authored_call_facts, authored_run_terminal_status, writable_task_ids,
};
use archon_workflow::*;
use serde_json::Value;

use super::harness::Host;

/// The terminal status of `host`'s run for `accounting`, its explanation
/// printed.
pub fn terminal_status(host: &Host, accounting: &Value) -> WorkflowV2Status {
    let calls = host.calls.borrow().clone();
    let ran = acceptance_ran::AcceptanceRan::of(&host.store);
    let accounting = accounting.to_string();
    let universe = host.f.universe.as_ref().unwrap();
    let universe_tasks: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.clone())
        .collect();
    // m6: the task stage recorded in the store, and the facts built from
    // the records as the live host builds them.
    let staged = task_stage::with_task_stage(&host.store, &universe_tasks, &calls);
    let facts = authored_call_facts(&staged, |id| host.store.load_call_record(id)).unwrap();
    let outcome = authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: WorkflowV2Status::NeedsReview,
        host_terminal_failure: None,
        script_result: Some(&task_stage::named(&universe_tasks, &accounting)),
        acceptance_gate: ran.fact(&facts),
        calls: &facts,
        writable_tasks: &writable_task_ids(Some(universe)),
        universe_tasks: &universe_tasks,
    });
    eprintln!("{}", outcome.explanation());
    outcome.status
}

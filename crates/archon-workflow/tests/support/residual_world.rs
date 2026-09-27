//! The Issue-117 residual world the end-to-end tests share: two tasks, a
//! store file no task declares, the review script that ends in
//! `resolveResiduals`, and the run's terminal status as the live host
//! decides it.
#![allow(dead_code)]
use std::collections::BTreeSet;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::residual_plan::residual_verdict;
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredRunFacts, authored_call_facts,
    authored_run_terminal_status_with, writable_task_ids,
};
use archon_workflow::*;
use serde_json::{Value, json};

use super::harness::{Answer, Host};
use super::support::{Edits, Fixture, git};

/// The file no task declares, whose consistency TASK-A's contract requires.
pub const STORE: &str = "crates/shared/src/store.rs";
pub const A: &str = "crates/a/src/lib.rs";
pub const B: &str = "crates/b/src/lib.rs";
pub const CROSS: &str = "cross:TASK-A+TASK-B";

pub const SCRIPT: &str = r#"export const meta = { name: 'residual', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['crates/b/src/lib.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const opts = { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }
const review = await remediateFindings(FINDINGS, opts)
FORGED
const residuals = typeof resolveResiduals === 'function' ? await resolveResiduals(opts) : null
return { review, residuals }
"#;

pub fn findings() -> Value {
    json!([{"id": "seam", "attributable_to_task": false, "canonical_task_ids": ["TASK-A", "TASK-B"],
        "severity": "high", "claim": "the two lanes disagree"}])
}

pub fn script() -> String {
    script_forging("")
}

pub fn script_forging(forged: &str) -> String {
    SCRIPT
        .replace("FINDINGS", &findings().to_string())
        .replace("FORGED", forged)
}

pub fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [(A, "// a\n"), (B, "// b\n"), (STORE, "// store\n")] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    // The task files live beside the repository, as a task set does.
    let tasks = f.repo.parent().unwrap().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("TASK-A.md"),
        format!("- `{STORE}` and `{A}` both write versions; they must stay consistent.\n"),
    )
    .unwrap();
    std::fs::write(tasks.join("TASK-B.md"), "B's lane.\n").unwrap();
    let task = |id: &str, owns: &str, forbids: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: tasks.join(format!("{id}.md")).display().to_string(),
        files_expected_to_change: vec![owns.into()],
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            // A forbids the store lane exactly: the round's lift opens it,
            // and nothing else A forbids.
            task(
                "TASK-A",
                A,
                &[
                    &format!("`{STORE}` (observed, not a deliverable)"),
                    "`.mcp.json`",
                ],
            ),
            task("TASK-B", B, &[]),
        ],
    });
    f
}

pub fn edits(files: Vec<(&'static str, &'static str)>) -> Edits {
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

/// The review's cross-task round fixes A's lane; a round for TASK-A (only a
/// residual round is one) fixes the store lane, one for TASK-B refreshes B.
pub fn writes(key: &str, _round: u64, _escalated: bool) -> Edits {
    match key {
        CROSS => edits(vec![(A, "// a: seam fixed\n")]),
        "TASK-A" => edits(vec![(STORE, "// store: canonical instrument\n")]),
        _ => edits(vec![(B, "// b: refreshed\n")]),
    }
}

pub fn host() -> Rc<Host> {
    host_on(fixture())
}

pub fn host_on(f: Fixture) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(writes)))
}

pub fn next(host: Rc<Host>) -> Rc<Host> {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    host_on(host.f)
}

/// The run's terminal status as the live host decides it: the terminal rule
/// with the residual gate folded in.
pub fn terminal(host: &Host, result: &Value) -> (WorkflowV2Status, String) {
    let calls = host.calls.borrow().clone();
    let facts = authored_call_facts(&calls, |id| host.store.load_call_record(id)).unwrap();
    let accounting = json!({"accepted": [], "blocked": [], "adversarial_findings": findings(),
        "uncovered_requirements": [], "review_remediation": result["review"]})
    .to_string();
    let universe = host.f.universe.as_ref().unwrap();
    let universe_tasks: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.clone())
        .collect();
    let residual = residual_verdict(&calls, &host.store, Some(universe), Some(&host.f.repo));
    let outcome = authored_run_terminal_status_with(
        &AuthoredRunFacts {
            accumulated_status: WorkflowV2Status::NeedsReview,
            host_terminal_failure: None,
            script_result: Some(&accounting),
            acceptance_gate: AuthoredAcceptanceGateFact::NotRequired,
            calls: &facts,
            writable_tasks: &writable_task_ids(Some(universe)),
            universe_tasks: &universe_tasks,
        },
        &residual.discharged,
    )
    .with_residual_gate(residual.blocking, residual.notes);
    (outcome.status, outcome.explanation())
}

pub fn answers(host: &Host) -> Vec<(String, Answer)> {
    host.answers.borrow().clone()
}

pub fn ran(host: &Host) -> Vec<String> {
    answers(host)
        .into_iter()
        .filter(|(_, answer)| *answer == Answer::Ran)
        .map(|(id, _)| id)
        .collect()
}

pub fn residual_calls(host: &Host) -> Vec<String> {
    answers(host)
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| id.contains("residual-"))
        .collect()
}

pub const HIGH_GAP: (&str, &str, &str) = (
    "gap-store-canonical-instrument",
    "high",
    "crates/shared/src/store.rs:226-229 recovers the canonical instrument via split('-').nth(2), the timeframe",
);

pub const PATHLESS: (&str, &str, &str) = (
    "gap-roster",
    "high",
    "the provider lanes are declared by no task in this run",
);

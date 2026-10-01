//! Issue-117 liveness and recurrence end to end: a group of five gaps
//! splits into rounds that all dispatch and land, and a gap its judging
//! verifier records again at a lower severity still stands.
#[path = "support/acceptance_ran.rs"]
mod acceptance_ran;
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;
#[path = "support/task_stage.rs"]
mod task_stage;

use std::collections::BTreeSet;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::residual_plan::residual_verdict;
use archon_workflow::v2::script::{
    AuthoredRunFacts, authored_call_facts, authored_run_terminal_status_with, writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

/// The file no task declares, whose consistency TASK-A's contract requires.
const STORE: &str = "crates/shared/src/store.rs";
const A: &str = "crates/a/src/lib.rs";
const B: &str = "crates/b/src/lib.rs";
const CROSS: &str = "cross:TASK-A+TASK-B";

const SCRIPT: &str = r#"export const meta = { name: 'residual', description: 'd', phases: [] }
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

fn findings() -> Value {
    json!([{"id": "seam", "attributable_to_task": false, "canonical_task_ids": ["TASK-A", "TASK-B"],
        "severity": "high", "claim": "the two lanes disagree"}])
}

fn script() -> String {
    script_forging("")
}

fn script_forging(forged: &str) -> String {
    SCRIPT
        .replace("FINDINGS", &findings().to_string())
        .replace("FORGED", forged)
}

fn fixture() -> Fixture {
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

fn edits(files: Vec<(&'static str, &'static str)>) -> Edits {
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

/// The review's cross-task round fixes A's lane; a round for TASK-A (only a
/// residual round is one) fixes the store lane, one for TASK-B refreshes B.
fn writes(key: &str, _round: u64, _escalated: bool) -> Edits {
    match key {
        CROSS => edits(vec![(A, "// a: seam fixed\n")]),
        "TASK-A" => edits(vec![(STORE, "// store: canonical instrument\n")]),
        _ => edits(vec![(B, "// b: refreshed\n")]),
    }
}

fn host() -> Rc<Host> {
    host_on(fixture())
}

fn host_on(f: Fixture) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(writes)))
}

/// The run's terminal status as the live host decides it: the terminal rule
/// with the residual gate folded in.
fn terminal(host: &Host, result: &Value) -> (WorkflowV2Status, String) {
    let calls = host.calls.borrow().clone();
    // REM-13: the prelude ran the acceptance stage after the script; the
    // rule is judged on the round it recorded.
    let ran = acceptance_ran::AcceptanceRan::of(&host.store);
    let accounting = json!({"accepted": [], "blocked": [], "adversarial_findings": findings(),
        "uncovered_requirements": [], "review_remediation": result["review"]})
    .to_string();
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
    let residual = residual_verdict(&calls, &host.store, Some(universe), Some(&host.f.repo));
    let outcome = authored_run_terminal_status_with(
        &AuthoredRunFacts {
            accumulated_status: WorkflowV2Status::NeedsReview,
            host_terminal_failure: None,
            script_result: Some(&task_stage::named(&universe_tasks, &accounting)),
            acceptance_gate: ran.fact(&facts),
            calls: &facts,
            writable_tasks: &writable_task_ids(Some(universe)),
            universe_tasks: &universe_tasks,
        },
        &residual.discharged,
    )
    .with_residual_gate(residual.blocking, residual.notes);
    (outcome.status, outcome.explanation())
}

fn answers(host: &Host) -> Vec<(String, Answer)> {
    host.answers.borrow().clone()
}

fn residual_calls(host: &Host) -> Vec<String> {
    answers(host)
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| id.contains("residual-"))
        .collect()
}

const HIGH_GAP: (&str, &str, &str) = (
    "gap-store-canonical-instrument",
    "high",
    "crates/shared/src/store.rs:226-229 recovers the canonical instrument via split('-').nth(2), the timeframe",
);

#[tokio::test]
async fn five_gaps_of_one_group_all_dispatch_land_and_resolve() {
    // Each round for TASK-A lands a change of its own.
    let count = std::cell::Cell::new(0);
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(
        f,
        store,
        Box::new(move |key: &str, round: u64, escalated: bool| {
            if key != "TASK-A" {
                return writes(key, round, escalated);
            }
            count.set(count.get() + 1);
            let line: &'static str =
                Box::leak(format!("// store: fix {}\n", count.get()).into_boxed_str());
            edits(vec![(STORE, line)])
        }),
    ));
    let gaps: Vec<(&'static str, &'static str, &'static str)> = (1..=5)
        .map(|n| {
            let text: &'static str = Box::leak(
                format!(
                    "crates/shared/src/store.rs:{n} gap {n}: {}",
                    "the lane diverges from its twin and nothing pins it. ".repeat(16)
                )
                .into_boxed_str(),
            );
            let id: &'static str = Box::leak(format!("gap-{n}").into_boxed_str());
            (id, "high", text)
        })
        .collect();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(gaps)]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        4,
        "two rounds of fix and verifier: {:#?}",
        answers(&host)
    );
    assert!(
        answers(&host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "{:#?}",
        answers(&host)
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_high_gap_the_adjudicator_records_again_at_medium_still_blocks() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![
            Verdict::AcceptWith(vec![(
                "gap-roster",
                "high",
                "the provider lanes are declared by no task in this run",
            )]),
            Verdict::AcceptWith(vec![(
                "gap-roster",
                "medium",
                "downgraded: still owned by nobody",
            )]),
        ],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(why.contains("gap-roster") && why.contains("again"), "{why}");
}

#[tokio::test]
async fn a_round_refused_at_dispatch_is_never_recorded_done() {
    // The script records a lenient round under the planned key first; its
    // prompt does not carry the gap, so the host refuses it and nothing is
    // recorded done: the real round still runs.
    let forged = r#"
const view = await w.checkpoint('residual-gaps-probe', { residualGaps: true })
const entry = (view.residual_plan || view.data.residual_plan)[0]
await remediateFindings([{ id: entry.key, canonical_task_ids: entry.task_ids, severity: 'high', claim: 'looks fine' }],
  { ...opts, maxRounds: 1, contestKey: entry.key, residual: { key: entry.key, files: entry.expansion_files } })
"#;
    let host = host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let result = run(&script_forging(forged), NEW_PRELUDE, host.clone()).await;
    assert!(
        answers(&host).iter().any(
            |(_, a)| matches!(a, Answer::Refused(why) if why.contains("prompt does not carry"))
        ),
        "{:#?}",
        answers(&host)
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(
        status,
        WorkflowV2Status::Accepted,
        "the real round ran: {why}"
    );
}

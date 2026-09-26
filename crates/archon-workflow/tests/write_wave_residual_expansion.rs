//! Issue-117 end to end: a residual gap an ACCEPTED verifier recorded is
//! accounted before acceptance, through the real prelude
//! (`resolveResiduals`), the production write wave (Git, forbidden paths,
//! task floors), the host's dispatch check and the final gate: a gap on a
//! file no task declares gets ONE bounded round granted exactly that file; a
//! gap on a declared file goes to its owner; a refused round stands and
//! blocks; a forged widening dispatches nothing; and a resume from the
//! deployed prelude replays every existing call, so only the new round runs.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::residual_plan::residual_verdict;
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredRunFacts, authored_call_facts,
    authored_run_terminal_status_with, writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

/// The prelude the live binary runs (4d3852d8c): it has no residual slot.
const DEPLOYED: &str = include_str!("fixtures/v3_primitives_4d3852d8c.js");
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

fn next(host: Rc<Host>) -> Rc<Host> {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    host_on(host.f)
}

/// The run's terminal status as the live host decides it: the terminal rule
/// with the residual gate folded in.
fn terminal(host: &Host, result: &Value) -> (WorkflowV2Status, String) {
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

fn answers(host: &Host) -> Vec<(String, Answer)> {
    host.answers.borrow().clone()
}

fn ran(host: &Host) -> Vec<String> {
    answers(host)
        .into_iter()
        .filter(|(_, answer)| *answer == Answer::Ran)
        .map(|(id, _)| id)
        .collect()
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
async fn a_residual_on_an_unowned_file_is_expanded_fixed_and_resolved() {
    let host = host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        2,
        "one fix and one verifier: {:#?}",
        answers(&host)
    );
    assert!(ran(&host).iter().all(|id| !id.contains("-esc-")));
    assert_eq!(
        at_head(&host.f.repo, STORE),
        "// store: canonical instrument"
    );
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    let contract = &fix.call.options.extra["remediationContract"];
    assert_eq!(contract["residual"]["files"], json!([STORE]));
    assert_eq!(
        (contract["round"].clone(), contract["maxRounds"].clone()),
        (json!(1), json!(1))
    );
    let prompts = host.prompts.borrow();
    let (_, prompt) = prompts.iter().find(|(id, _)| *id == calls[0]).unwrap();
    assert!(
        prompt.contains(&format!("may ALSO write {STORE}")),
        "{prompt}"
    );
    assert!(
        prompt.contains("split('-').nth(2)"),
        "the gap, verbatim: {prompt}"
    );
    let verifier = host.store.load_call_record(&calls[1]).unwrap().unwrap();
    assert_eq!(verifier.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    assert!(
        answers(&host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "the host's own plan is never refused"
    );
    assert_eq!(result["residuals"][0]["files"], json!([STORE]), "{result}");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(why.contains("round") && why.contains("resolved"), "{why}");
}

#[tokio::test]
async fn a_refused_expansion_is_reported_blocks_and_is_never_asked_again() {
    let first = host();
    first.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    // Refused over B's file: a regular round would buy the cross-owner
    // escalation; a residual round buys nothing.
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![B])]);
    let result = run(&script(), NEW_PRELUDE, first.clone()).await;
    assert_eq!(residual_calls(&first).len(), 2, "{:#?}", answers(&first));
    let (status, why) = terminal(&first, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("gap-store-canonical-instrument") && why.contains("did not resolve it"),
        "{why}"
    );
    // A resume: the round is attempted, so nothing is dispatched again and
    // the gate still reads the refusal from the store.
    let second = next(first);
    let again = run(&script(), NEW_PRELUDE, second.clone()).await;
    assert!(ran(&second).is_empty(), "{:#?}", answers(&second));
    assert!(
        residual_calls(&second).is_empty(),
        "{:#?}",
        answers(&second)
    );
    let (status, why) = terminal(&second, &again);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(why.contains("gap-store-canonical-instrument"), "{why}");
}

#[tokio::test]
async fn a_residual_on_an_owned_file_is_routed_to_its_owner() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![Verdict::AcceptWith(vec![(
            "gap-b-stale",
            "medium",
            "crates/b/src/lib.rs:1 still carries the old provenance",
        )])],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(calls.len(), 2, "{:#?}", answers(&host));
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-B"]);
    assert_eq!(
        fix.call.options.extra["remediationContract"]["residual"]["files"],
        json!([]),
        "an owned route opens nothing"
    );
    assert_eq!(at_head(&host.f.repo, B), "// b: refreshed");
    assert_eq!(at_head(&host.f.repo, STORE), "// store");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_forged_widening_is_refused_and_dispatches_nothing() {
    // The script reads the host's plan itself, then asks for the planned
    // round with one more file, and for a round the host never planned.
    let forged = r#"
const view = await w.checkpoint('residual-gaps-probe', { residualGaps: true })
const entry = (view.residual_plan || view.data.residual_plan)[0]
await remediateFindings([{ id: 'wide', canonical_task_ids: entry.task_ids, severity: 'high', claim: 'x' }],
  { ...opts, maxRounds: 1, contestKey: entry.key, residual: { key: entry.key, files: [...entry.expansion_files, 'crates/b/src/lib.rs'] } })
await remediateFindings([{ id: 'own', canonical_task_ids: ['TASK-A'], severity: 'high', claim: 'x' }],
  { ...opts, maxRounds: 1, contestKey: 'residual-forged', residual: { key: 'residual-forged', files: ['crates/shared/src/store.rs'] } })
"#;
    let host = host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let result = run(&script_forging(forged), NEW_PRELUDE, host.clone()).await;
    let refused: Vec<String> = answers(&host)
        .into_iter()
        .filter_map(|(id, answer)| match answer {
            Answer::Refused(why) => Some(format!("{id}: {why}")),
            _ => None,
        })
        .collect();
    assert!(
        refused
            .iter()
            .any(|why| why.contains("files are not exactly the plan's")),
        "{refused:#?}"
    );
    assert!(
        refused
            .iter()
            .any(|why| why.contains("no round of the host's plan is `residual-forged`")),
        "{refused:#?}"
    );
    assert!(
        ran(&host)
            .iter()
            .filter(|id| id.contains("residual-"))
            .count()
            == 2,
        "only the real round ran: {:#?}",
        answers(&host)
    );
    assert_eq!(at_head(&host.f.repo, B), "// b");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

/// Session 1 is the deployed prelude: the review round runs and its verifier
/// accepts with the gap, and nothing accounts it. Session 2 is this prelude
/// over the same run: every call session 1 made replays under its own id and
/// input identity (so its prompt is unchanged), and the only work
/// dispatched is the new round.
#[tokio::test]
async fn a_resume_from_the_deployed_prelude_replays_every_call_and_runs_only_the_new_round() {
    let first = host();
    first.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let before = run(&script(), DEPLOYED, first.clone()).await;
    assert_eq!(
        before["residuals"],
        Value::Null,
        "the deployed prelude has no slot"
    );
    let recorded: Vec<(String, Answer)> = answers(&first);
    assert_eq!(recorded.len(), 2, "{recorded:#?}");
    assert!(recorded.iter().all(|(_, answer)| *answer == Answer::Ran));
    let (status, why) = terminal(&first, &before);
    assert_eq!(
        status,
        WorkflowV2Status::NeedsReview,
        "under this host, the gap no round carried blocks: {why}"
    );
    let second = next(first);
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    let answered = answers(&second);
    for (id, _) in &recorded {
        assert!(
            answered.contains(&(id.clone(), Answer::Replayed)),
            "{id} replays: {answered:#?}"
        );
    }
    let new: Vec<&String> = answered
        .iter()
        .filter(|(_, answer)| *answer != Answer::Replayed)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(new.len(), 2, "{answered:#?}");
    assert!(
        new.iter().all(|id| id.contains("residual-")),
        "{answered:#?}"
    );
    assert_eq!(
        at_head(&second.f.repo, STORE),
        "// store: canonical instrument"
    );
    let (status, why) = terminal(&second, &after);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_glob_resolves_to_exact_files_and_is_expanded_like_a_named_file() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![Verdict::AcceptWith(vec![(
            "gap-lanes",
            "high",
            "the crates/shared/src/*.rs lanes carry the wrong instrument",
        )])],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(calls.len(), 2, "{:#?}", answers(&host));
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(
        fix.call.options.extra["remediationContract"]["residual"]["files"],
        json!([STORE])
    );
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

const PATHLESS: (&str, &str, &str) = (
    "gap-roster",
    "high",
    "the provider lanes are declared by no task in this run",
);

#[tokio::test]
async fn a_pathless_high_gap_is_resolved_by_an_accepting_adjudication() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![Verdict::AcceptWith(vec![PATHLESS]), Verdict::Accept],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        1,
        "one read-only verifier: {:#?}",
        answers(&host)
    );
    assert!(calls[0].ends_with("-adjudicate"), "{calls:?}");
    let record = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert!(record.call.write_mode.is_none());
    assert_eq!(
        record.dispatched_items[0].canonical_task_ids,
        ["TASK-A", "TASK-B"],
        "the recording unit's tasks"
    );
    let prompt = record.call.options.task.clone().unwrap_or_default();
    assert!(
        prompt.contains("declared by no task in this run"),
        "{prompt}"
    );
    assert!(
        prompt.contains("every finding resolved"),
        "the recording summary: {prompt}"
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    // Once per gap: a resume asks nothing again.
    let second = next(host);
    run(&script(), NEW_PRELUDE, second.clone()).await;
    assert!(ran(&second).is_empty(), "{:#?}", answers(&second));
}

#[tokio::test]
async fn a_pathless_high_gap_the_adjudicator_records_again_blocks_by_name() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![
            Verdict::AcceptWith(vec![PATHLESS]),
            Verdict::AcceptWith(vec![(
                "gap-roster-again",
                "high",
                "the lanes are still owned by no task",
            )]),
        ],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert_eq!(residual_calls(&host).len(), 1, "{:#?}", answers(&host));
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("gap-roster") && why.contains("recorded high gap(s) again"),
        "{why}"
    );
}

//! REM-10, wave level: a blocked task that review remediation finishes was
//! never reviewed -- the mandatory maps ran over the accepted tasks before
//! it was done. Through the real prelude, the production write wave, the
//! host's remediation plan, the host's review attachment and the terminal
//! rule: both maps run again over exactly the task that moved, what they
//! find is remediated in a pass of its own, an open late finding holds the
//! run, a task that did not move is not re-reviewed, and a resume replays
//! every call.
#[path = "support/acceptance_ran.rs"]
mod acceptance_ran;
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/review_answers.rs"]
mod review;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::review_finding_ids::finding_id_of;
use archon_workflow::v2::script::{
    AuthoredRunFacts, AuthoredRunOutcome, authored_call_facts, authored_run_terminal_status_with,
    writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, Verdict};
use review::{Reviewer, queue_verdicts, run_reviewing};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const A: &str = "crates/a/src/lib.rs";
const B: &str = "crates/b/src/lib.rs";
const REASON: &str = "its verifier refused it twice";

const SCRIPT: &str = r#"export const meta = { name: 'moved-review', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['crates/b/src/lib.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const evidenceFor = (id) => [{ task: id, taskFile: byId(id).file }]
const accepted_ids = []
const blockedTasks = []
for (const t of tasks) {
  const impl = await agent('Implement ' + t.id + ' per ' + t.file, { label: 'implement-' + t.id.toLowerCase(), write: true, taskIds: [t.id], targetFiles: t.targetFiles })
  const check = await agent('Verify ' + t.id + ' per ' + t.file, { label: 'verify-' + t.id.toLowerCase(), verify: true, taskIds: [t.id] })
  if (usable(impl) && accepted(check)) accepted_ids.push(t.id)
  else blockedTasks.push({ taskId: t.id, reason: 'its verifier refused it twice' })
}
const adversarial_findings = await adversarialReview(accepted_ids, { evidenceFor })
const uncovered_requirements = await coverageAudit(accepted_ids, { evidenceFor })
const review = await remediateFindings([...adversarial_findings, ...uncovered_requirements], { blockedTasks, movedReview: true, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
SECOND
log('after review remediation')
return { accepted: accepted_ids, blocked: blockedTasks, adversarial_findings, uncovered_requirements, review }
"#;

/// The script, its review pass handed `movedReview: true` (a key no script
/// may use to skip the late review); `again` adds a second pass over the
/// same blocked task.
fn script(again: bool) -> String {
    SCRIPT.replace(
        "SECOND",
        if again {
            "const second = await remediateFindings([], { blockedTasks, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })"
        } else {
            ""
        },
    )
}

fn late_finding() -> Value {
    json!({"id": "b-late", "severity": "high",
        "claim": "crates/b/src/lib.rs accepts an empty range"})
}

/// The finding the prelude folds in for the blocked task, as it sends it.
fn blocked_id() -> String {
    finding_id_of(&json!({
        "canonical_task_ids": ["TASK-B"],
        "id": "blocked-task-task-b",
        "blocked_task": "TASK-B",
        "description": format!("This task exhausted its remediation budget without passing verification. Last verifier summary: {REASON}"),
    }))
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [(A, "// a\n"), (B, "// b\n")] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    let tasks = f.repo.parent().unwrap().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    for id in ["TASK-A", "TASK-B"] {
        std::fs::write(tasks.join(format!("{id}.md")), format!("{id}'s lane.\n")).unwrap();
    }
    let task = |id: &str, owns: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: tasks.join(format!("{id}.md")).display().to_string(),
        files_expected_to_change: vec![owns.into()],
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-A", A), task("TASK-B", B)],
    });
    f
}

static FIX: AtomicUsize = AtomicUsize::new(0);

/// Every fix lands a change of its own, so each is verified.
fn writes(key: &str, round: u64, _escalated: bool) -> Edits {
    let path = if key == "TASK-A" { A } else { B };
    let n = FIX.fetch_add(1, Ordering::SeqCst);
    let content: &'static str =
        Box::leak(format!("// {key} round {round} fix {n}\n").into_boxed_str());
    Edits {
        report: vec![path],
        files: vec![(path, content)],
        via_adapter: false,
    }
}

/// A session over `f`: the moved map finds the late finding on TASK-B, and
/// its verdicts are queued once its host id is known.
fn session(f: Fixture, late: &'static str) -> Rc<Reviewer> {
    session_then(f, late, Vec::new())
}

/// [`session`], queuing `then` for TASK-B after the late finding's verdicts.
fn session_then(f: Fixture, late: &'static str, then: Vec<Verdict>) -> Rc<Reviewer> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(writes)));
    Rc::new(Reviewer {
        host,
        maps: vec![(
            "adversarial-review-moved-1-map",
            "TASK-B",
            vec![late_finding()],
        )],
        on_map: Box::new(move |host, call, attached| {
            if call == "adversarial-review-moved-1-map" {
                let ids: Vec<(String, &'static str)> =
                    attached.iter().map(|f| (finding_id_of(f), late)).collect();
                queue_verdicts(
                    host,
                    "TASK-B",
                    // One round closes a resolved finding; an open one is
                    // judged in both of its unit's rounds.
                    [
                        if late == "resolved" {
                            vec![Verdict::Dispose(ids)]
                        } else {
                            vec![Verdict::Dispose(ids.clone()), Verdict::Dispose(ids)]
                        },
                        then.clone(),
                    ]
                    .concat(),
                );
            }
        }),
    })
}

fn terminal(host: &Host, result: &Value) -> AuthoredRunOutcome {
    let ran = acceptance_ran::AcceptanceRan::of(&host.store);
    let calls = host.calls.borrow().clone();
    let facts = authored_call_facts(&calls, |id| host.store.load_call_record(id)).unwrap();
    let accounting = json!({"accepted": result["accepted"], "blocked": result["blocked"],
        "adversarial_findings": result["adversarial_findings"],
        "uncovered_requirements": result["uncovered_requirements"],
        "review_remediation": result["review"]})
    .to_string();
    let universe = host.f.universe.as_ref().unwrap();
    let universe_tasks: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.clone())
        .collect();
    authored_run_terminal_status_with(
        &AuthoredRunFacts {
            accumulated_status: WorkflowV2Status::NeedsReview,
            host_terminal_failure: None,
            script_result: Some(&accounting),
            acceptance_gate: ran.fact(&facts),
            calls: &facts,
            writable_tasks: &writable_task_ids(Some(universe)),
            universe_tasks: &universe_tasks,
        },
        &BTreeSet::new(),
    )
}

fn ids(host: &Host) -> Vec<String> {
    host.answers
        .borrow()
        .iter()
        .map(|(id, _)| id.clone())
        .collect()
}

fn dispatched_tasks(host: &Host, call: &str) -> Vec<String> {
    let record = host.store.load_call_record(call).unwrap().expect(call);
    record
        .dispatched_items
        .iter()
        .flat_map(|item| item.canonical_task_ids.clone())
        .collect()
}

fn late_fixes(host: &Host) -> Vec<WorkflowV2HostCall> {
    host.calls
        .borrow()
        .iter()
        .filter(|call| {
            call.write_mode.is_some()
                && call
                    .options
                    .extra
                    .get("remediationContract")
                    .and_then(|contract| contract.get("sourceReduceCallIds"))
                    == Some(&json!([
                        "adversarial-review-moved-1-reduce",
                        "coverage-audit-moved-1-reduce"
                    ]))
        })
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_task_remediation_finished_is_reviewed_by_both_maps_and_its_finding_remediated() {
    let reviewer = session(fixture(), "resolved");
    queue_verdicts(
        &reviewer.host,
        "TASK-B",
        vec![
            // Its own verifier refuses TASK-B: it is blocked.
            Verdict::Refuse(vec![]),
            Verdict::Dispose(vec![(blocked_id(), "resolved")]),
        ],
    );
    let result = run_reviewing(&script(false), reviewer.clone()).await;
    let host = &reviewer.host;

    // Both maps ran again, over exactly the task that moved.
    let answered = ids(host);
    for call in [
        "adversarial-review-moved-1-map",
        "coverage-audit-moved-1-map",
    ] {
        assert!(answered.iter().any(|id| id == call), "{answered:#?}");
        assert_eq!(dispatched_tasks(host, call), ["TASK-B"]);
    }
    for call in ["adversarial-review-map", "coverage-audit-map"] {
        assert_eq!(
            dispatched_tasks(host, call),
            ["TASK-A"],
            "the mandatory maps are as they were"
        );
    }
    let moved = &result["review"]["movedReview"];
    assert_eq!(moved["taskIds"], json!(["TASK-B"]), "{result}");
    let late = moved["adversarial_findings"].as_array().unwrap();
    assert_eq!(late.len(), 1, "{moved}");
    let late_id = finding_id_of(&late[0]);

    // The late finding went through a remediation pass of its own, whose
    // units name the late reviews as their source.
    // Major 3: the late pass files its calls in its own id space and names
    // its own checkpoints; the shared counters do not move for it, so the
    // script's next call keeps the id it has without a late review.
    let store = &host.store;
    for id in ["remediation-plan-moved-1", "coverage-inventory-moved-1"] {
        assert!(store.load_call_record(id).unwrap().is_some(), "{id}");
    }
    for id in ["remediation-plan-2", "coverage-inventory-2"] {
        assert!(store.load_call_record(id).unwrap().is_none(), "{id}");
    }
    let late_calls: Vec<&String> = answered
        .iter()
        .filter(|id| {
            id.starts_with("review-remediate-")
                || id.starts_with("verification-wave-review-verify-")
        })
        .filter(|id| !id.ends_with("-5") && !id.ends_with("-6"))
        .collect();
    assert!(!late_calls.is_empty(), "{answered:#?}");
    assert!(
        late_calls.iter().all(|id| id.contains("-late1-")),
        "{late_calls:#?}"
    );
    assert!(
        store.load_call_record("log-7").unwrap().is_some(),
        "implement/verify A and B, then B's round fix and verify are 1..6: the log is 7"
    );
    let fixes = late_fixes(host);
    assert_eq!(fixes.len(), 1, "{answered:#?}");
    let contract = &fixes[0].options.extra["remediationContract"];
    assert_eq!(contract["findingIds"], json!([late_id]));
    assert_eq!(contract["taskId"], json!("TASK-B"));
    let resolved: Vec<&Value> = result["review"]["resolved"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert!(
        resolved
            .iter()
            .any(|entry| entry["findingIds"] == json!([late_id])),
        "{result}"
    );
    assert!(
        host.answers
            .borrow()
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "nothing refused at dispatch"
    );

    let outcome = terminal(host, &result);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
    assert!(
        outcome
            .notes
            .iter()
            .any(|note| note.contains("finished by review remediation")),
        "{}",
        outcome.explanation()
    );
}

#[tokio::test]
async fn an_open_late_finding_holds_the_run_by_its_id() {
    let reviewer = session(fixture(), "open");
    queue_verdicts(
        &reviewer.host,
        "TASK-B",
        vec![
            // Its own verifier refuses TASK-B: it is blocked.
            Verdict::Refuse(vec![]),
            Verdict::Dispose(vec![(blocked_id(), "resolved")]),
        ],
    );
    let result = run_reviewing(&script(false), reviewer.clone()).await;
    let host = &reviewer.host;
    let late = result["review"]["movedReview"]["adversarial_findings"][0].clone();
    let late_id = finding_id_of(&late);
    let unresolved = result["review"]["unresolved"].as_array().unwrap();
    assert!(
        unresolved
            .iter()
            .any(|entry| entry["findingId"] == json!(late_id)),
        "{result}"
    );
    let outcome = terminal(host, &result);
    assert_ne!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
    assert!(
        outcome
            .blocking
            .iter()
            .any(|clause| clause.contains(&late_id)),
        "{}",
        outcome.explanation()
    );
}

#[tokio::test]
async fn a_task_that_did_not_move_is_not_re_reviewed() {
    let reviewer = session(fixture(), "resolved");
    queue_verdicts(
        &reviewer.host,
        "TASK-B",
        vec![
            // Its own verifier refuses TASK-B: it is blocked.
            Verdict::Refuse(vec![]),
            Verdict::Dispose(vec![(blocked_id(), "open")]),
            Verdict::Dispose(vec![(blocked_id(), "open")]),
        ],
    );
    let result = run_reviewing(&script(false), reviewer.clone()).await;
    let answered = ids(&reviewer.host);
    assert!(
        answered.iter().all(|id| !id.contains("-moved-")),
        "{answered:#?}"
    );
    assert_eq!(result["review"]["movedReview"], Value::Null, "{result}");
    let outcome = terminal(&reviewer.host, &result);
    assert_ne!(outcome.status, WorkflowV2Status::Accepted);
}

/// A resume under the same prelude replays every call the first session
/// made, the late review and its remediation included, and dispatches
/// nothing.
#[tokio::test]
async fn a_resume_replays_the_late_review_and_its_remediation() {
    let first = session(fixture(), "resolved");
    queue_verdicts(
        &first.host,
        "TASK-B",
        vec![
            // Its own verifier refuses TASK-B: it is blocked.
            Verdict::Refuse(vec![]),
            Verdict::Dispose(vec![(blocked_id(), "resolved")]),
        ],
    );
    let before = run_reviewing(&script(false), first.clone()).await;
    let recorded = ids(&first.host);
    let Ok(reviewer) = Rc::try_unwrap(first) else {
        panic!("session still referenced")
    };
    let Ok(host) = Rc::try_unwrap(reviewer.host) else {
        panic!("host still referenced")
    };
    let second = session(host.f, "resolved");
    // A refused task verdict is not reusable, so the host asks it again, as
    // live; it refuses again.
    queue_verdicts(&second.host, "TASK-B", vec![Verdict::Refuse(vec![])]);
    let after = run_reviewing(&script(false), second.clone()).await;
    let answers = second.host.answers.borrow().clone();
    let ran: Vec<&String> = answers
        .iter()
        .filter(|(_, answer)| *answer == Answer::Ran)
        .map(|(id, _)| id)
        .collect();
    assert!(
        ran.iter()
            .all(|id| id.starts_with("verification-wave-verify-task-b-")),
        "only the refused task verdict is asked again: {answers:#?}"
    );
    assert!(
        answers
            .iter()
            .filter(|(id, _)| id.contains("-moved-") || id.starts_with("review-remediate-"))
            .all(|(_, answer)| *answer == Answer::Replayed),
        "the late review and every remediation replay: {answers:#?}"
    );
    for id in &recorded {
        assert!(
            answers.iter().any(|(seen, _)| seen == id),
            "{id} answered again: {answers:#?}"
        );
    }
    assert_eq!(after["review"], before["review"]);
}

/// Minor 5: a task is reviewed late once per session, whichever pass moves
/// it: a second pass that finishes it again reviews nothing again.
#[tokio::test]
async fn a_task_is_reviewed_late_once() {
    let reviewer = session_then(
        fixture(),
        "resolved",
        vec![Verdict::Dispose(vec![(blocked_id(), "resolved")])],
    );
    queue_verdicts(
        &reviewer.host,
        "TASK-B",
        vec![
            Verdict::Refuse(vec![]),
            Verdict::Dispose(vec![(blocked_id(), "resolved")]),
        ],
    );
    let result = run_reviewing(&script(true), reviewer.clone()).await;
    let answered = ids(&reviewer.host);
    assert!(
        answered
            .iter()
            .any(|id| id == "adversarial-review-moved-1-map"),
        "{answered:#?}"
    );
    assert!(
        answered.iter().all(|id| !id.contains("-moved-2-")),
        "no second late review: {answered:#?}"
    );
    assert_eq!(
        result["review"]["movedReview"]["taskIds"],
        json!(["TASK-B"])
    );
}

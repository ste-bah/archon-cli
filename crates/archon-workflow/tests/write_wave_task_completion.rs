//! REM-14, wave level: the authored script implements one universe task and
//! skips the other. Before the first review the host names the skipped task
//! on its completion plan, and the prelude runs it as a task through the
//! production write wave with real Git: an implementation write, a
//! verification, the retry budget on a refusal, then both reviews over it
//! with the script's own task. The runner folds the unit's outcome into the
//! accounting, and the terminal rule decides from the host's records.
#[path = "support/acceptance_ran.rs"]
mod acceptance_ran;
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::task_completion::completion_unit;
use archon_workflow::v2::script::{
    AuthoredRunFacts, AuthoredRunOutcome, authored_call_facts, authored_run_terminal_status,
    writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const A: &str = "crates/a/src/lib.rs";
const B: &str = "crates/b/src/lib.rs";

/// The script dispatches TASK-A and never mentions TASK-B's work.
const SCRIPT: &str = r#"export const meta = { name: 'skips-a-task', description: 'd', phases: [] }
const acceptedIds = []
const blockedTasks = []
const impl = await agent('Implement TASK-A per tasks/TASK-A.md', { label: 'implement-task-a', write: true, taskIds: ['TASK-A'], targetFiles: ['crates/a/src/lib.rs'] })
const check = await agent('Verify TASK-A against tasks/TASK-A.md', { label: 'verify-task-a', verify: true, taskIds: ['TASK-A'] })
if (accepted(check)) acceptedIds.push('TASK-A')
else blockedTasks.push({ taskId: 'TASK-A', reason: 'refused' })
const adversarial_findings = await adversarialReview(acceptedIds)
const uncovered_requirements = await coverageAudit(acceptedIds)
const review_remediation = await remediateFindings([...adversarial_findings, ...uncovered_requirements], { blockedTasks })
return { accepted: acceptedIds, blocked: blockedTasks, adversarial_findings, uncovered_requirements, review_remediation }
"#;

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
        focused_tests: vec![format!("cargo test -p {}", id.to_lowercase())],
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-A", A), task("TASK-B", B)],
    });
    f
}

/// Each task's write lands a change in its own file.
fn writes(key: &str, _round: u64, _escalated: bool) -> Edits {
    let (path, content) = if key == "TASK-A" {
        (A, "// a implemented\n")
    } else {
        (B, "// b implemented\n")
    };
    Edits {
        report: vec![path],
        files: vec![(path, content)],
        via_adapter: false,
    }
}

fn host(f: Fixture) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(writes)))
}

fn next(host: Rc<Host>) -> Rc<Host> {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    self::host(host.f)
}

fn terminal(host: &Host, accounting: &Value) -> AuthoredRunOutcome {
    let ran = acceptance_ran::AcceptanceRan::of(&host.store);
    let calls = host.calls.borrow().clone();
    let facts = authored_call_facts(&calls, |id| host.store.load_call_record(id)).unwrap();
    let universe = host.f.universe.as_ref().unwrap();
    let universe_tasks: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.clone())
        .collect();
    let accounting = accounting.to_string();
    authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: WorkflowV2Status::NeedsReview,
        host_terminal_failure: None,
        script_result: Some(&accounting),
        acceptance_gate: ran.fact(&facts),
        calls: &facts,
        writable_tasks: &writable_task_ids(Some(universe)),
        universe_tasks: &universe_tasks,
    })
}

/// The tasks the host dispatched to the map of the review `label`.
fn reviewed(host: &Host, label: &str) -> BTreeSet<String> {
    let record = host
        .store
        .load_call_record(&format!("{label}-map"))
        .unwrap()
        .expect("the review map ran");
    record
        .dispatched_items
        .iter()
        .flat_map(|item| item.canonical_task_ids.clone())
        .collect()
}

#[tokio::test]
async fn a_task_the_script_skipped_is_implemented_verified_and_reviewed_by_the_run_itself() {
    let host = host(fixture());
    let unit = completion_unit("TASK-B");
    let result = run(SCRIPT, NEW_PRELUDE, host.clone()).await;

    // The host's completion unit wrote TASK-B's own file, through the wave.
    let calls = host.calls.borrow().clone();
    let write = calls
        .iter()
        .find(|call| call.id == format!("{unit}-impl-1"))
        .expect("the completion write was dispatched");
    assert!(write.write_mode.is_some());
    assert_eq!(at_head(&host.f.repo, B), "// b implemented");
    let verify_id = format!("verification-wave-{unit}-verify-1");
    assert!(
        calls.iter().any(|call| call.id == verify_id),
        "the completion verifier ran"
    );
    let record = host.store.load_call_record(&write.id).unwrap().unwrap();
    assert_eq!(
        record.dispatched_items[0].canonical_task_ids,
        vec!["TASK-B".to_string()]
    );
    // Before the reviews, which reviewed it with the script's own task.
    let first_review = calls
        .iter()
        .position(|call| call.id == "adversarial-review-map")
        .expect("reviewed");
    let completion_at = calls.iter().position(|call| call.id == write.id).unwrap();
    assert!(completion_at < first_review);
    let both: BTreeSet<String> = ["TASK-A", "TASK-B"].map(String::from).into();
    assert_eq!(reviewed(&host, "adversarial-review"), both);
    assert_eq!(reviewed(&host, "coverage-audit"), both);

    // The accounting names it, as the host's unit completed it.
    assert_eq!(result["accepted"], json!(["TASK-A", "TASK-B"]), "{result}");
    assert_eq!(result["blocked"], json!([]));
    assert_eq!(result["task_completion"][0]["taskId"], json!("TASK-B"));
    assert_eq!(result["task_completion"][0]["outcome"], json!("accepted"));

    // The terminal rule accepts it from the host's records -- also when the
    // accounting is the script's own, which never named TASK-B.
    let outcome = terminal(&host, &result);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
    let mut own = result.clone();
    own["accepted"] = json!(["TASK-A"]);
    let outcome = terminal(&host, &own);
    // An accounting that omits it does not stand, whatever the records show.
    assert_eq!(
        outcome.status,
        WorkflowV2Status::NeedsReview,
        "{}",
        outcome.explanation()
    );
    assert!(
        outcome
            .explanation()
            .contains("task TASK-B of the task universe is missing from the script's accounting"),
        "{}",
        outcome.explanation()
    );

    // A resumed session asks the same question, is planned the same unit,
    // and replays it: nothing is dispatched again.
    let second = next(host);
    let again = run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    assert_eq!(again["accepted"], json!(["TASK-A", "TASK-B"]), "{again}");
    let answers = second.answers.borrow().clone();
    for id in [format!("{unit}-impl-1"), verify_id] {
        let answer = answers.iter().find(|(call, _)| *call == id).map(|(_, a)| a);
        assert_eq!(answer, Some(&Answer::Replayed), "{id}: {answers:?}");
    }
    assert!(
        !answers.iter().any(|(id, _)| id.ends_with("-impl-2")),
        "no further attempt: {answers:?}"
    );
    let outcome = terminal(&second, &again);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
}

#[tokio::test]
async fn a_refused_completion_is_retried_with_the_refusal_and_accepted() {
    let host = host(fixture());
    host.verdicts("TASK-B", vec![Verdict::Refuse(vec![B])]);
    let unit = completion_unit("TASK-B");
    let result = run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    let prompts = host.prompts.borrow().clone();
    let retry = prompts
        .iter()
        .find(|(id, _)| *id == format!("{unit}-impl-2"))
        .map(|(_, prompt)| prompt.clone())
        .expect("a second attempt was dispatched");
    assert!(retry.contains("REJECTED"), "{retry}");
    assert!(
        retry.contains("must-pass baseline tests fail"),
        "the refusal travels verbatim: {retry}"
    );
    assert_eq!(result["accepted"], json!(["TASK-A", "TASK-B"]), "{result}");
    assert_eq!(result["task_completion"][0]["attempts"], json!(2));
    let outcome = terminal(&host, &result);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
}

#[tokio::test]
async fn a_completion_no_verifier_accepts_holds_the_run_by_name() {
    let host = host(fixture());
    host.verdicts("TASK-B", vec![Verdict::Refuse(vec![B]); 12]);
    let result = run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    assert_eq!(result["accepted"], json!(["TASK-A"]), "{result}");
    let blocked = &result["blocked"][0];
    assert_eq!(blocked["taskId"], json!("TASK-B"), "{result}");
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .contains("completion unit"),
        "{result}"
    );
    // Not reviewed as if done: only TASK-A's work was.
    assert_eq!(
        reviewed(&host, "adversarial-review"),
        ["TASK-A".to_string()].into()
    );
    // Handed to review remediation as a blocked task, which failed too.
    assert!(remediation_fixes(&host, "TASK-B") > 0, "{result}");
    let outcome = terminal(&host, &result);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::NeedsReview,
        "{}",
        outcome.explanation()
    );
    assert!(
        outcome
            .blocking
            .iter()
            .any(|clause| clause.contains("TASK-B")),
        "{}",
        outcome.explanation()
    );
    // And the script's own accounting, which never names it, holds it too.
    let mut own = result.clone();
    own["blocked"] = json!([]);
    let outcome = terminal(&host, &own);
    assert!(
        outcome.blocking.iter().any(|clause| clause
            .contains("task TASK-B of the task universe reached no accepted outcome")),
        "{}",
        outcome.explanation()
    );
}

/// Review-remediation fixes the host dispatched for `task`'s unit.
fn remediation_fixes(host: &Host, task: &str) -> usize {
    host.calls
        .borrow()
        .iter()
        .filter(|call| {
            call.write_mode.is_some()
                && call
                    .options
                    .extra
                    .get("remediationContract")
                    .is_some_and(|contract| contract["taskId"] == json!(task))
        })
        .count()
}

/// Every write lands a change of its own, so a remediation fix after the
/// completion attempts lands something to verify.
fn counting_writes() -> impl Fn(&str, u64, bool) -> Edits {
    let written = std::cell::Cell::new(0u32);
    move |key, _round, _escalated| {
        written.set(written.get() + 1);
        let path = if key == "TASK-A" { A } else { B };
        let content: &'static str =
            Box::leak(format!("// {key} write {}\n", written.get()).into_boxed_str());
        Edits {
            report: vec![path],
            files: vec![(path, content)],
            via_adapter: false,
        }
    }
}

#[tokio::test]
async fn a_failed_completion_is_remediated_as_a_blocked_task_and_then_accepted() {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(counting_writes())));
    // Every completion attempt the budget funds is refused; the review
    // remediation's verifier then accepts the fix.
    host.verdicts("TASK-B", vec![Verdict::Refuse(vec![B]); 6]);
    let unit = completion_unit("TASK-B");
    let result = run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    let answered = |id: String| host.answers.borrow().iter().any(|(call, _)| *call == id);
    assert!(
        answered(format!("{unit}-impl-6")),
        "the budget's attempts ran"
    );
    assert!(!answered(format!("{unit}-impl-7")), "and no more");
    assert_eq!(result["task_completion"][0]["outcome"], json!("blocked"));
    assert_eq!(result["blocked"][0]["taskId"], json!("TASK-B"), "{result}");
    // Review remediation took it as a blocked task and closed it.
    assert!(remediation_fixes(&host, "TASK-B") > 0, "{result}");
    let resolved = result["review_remediation"]["resolved"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["taskId"] == json!("TASK-B"));
    assert!(resolved, "{result}");
    let outcome = terminal(&host, &result);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
    assert!(
        outcome
            .explanation()
            .contains("blocked task TASK-B was finished by review remediation"),
        "{}",
        outcome.explanation()
    );
}

/// The script never runs a review remediation pass.
const NO_REMEDIATION: &str = r#"export const meta = { name: 'never-remediates', description: 'd', phases: [] }
const impl = await agent('Implement TASK-A per tasks/TASK-A.md', { label: 'implement-task-a', write: true, taskIds: ['TASK-A'], targetFiles: ['crates/a/src/lib.rs'] })
const check = await agent('Verify TASK-A against tasks/TASK-A.md', { label: 'verify-task-a', verify: true, taskIds: ['TASK-A'] })
const adversarial_findings = await adversarialReview(['TASK-A'])
const uncovered_requirements = await coverageAudit(['TASK-A'])
return { accepted: ['TASK-A'], blocked: [], adversarial_findings, uncovered_requirements, review_remediation: { resolved: [], unresolved: [], unassigned: [] } }
"#;

#[tokio::test]
async fn a_failed_completion_the_script_never_remediated_is_remediated_by_the_run() {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(counting_writes())));
    host.verdicts("TASK-B", vec![Verdict::Refuse(vec![B]); 6]);
    let result = run(NO_REMEDIATION, NEW_PRELUDE, host.clone()).await;
    assert_eq!(result["blocked"][0]["taskId"], json!("TASK-B"), "{result}");
    // The run, not the script, remediated it before the acceptance stage.
    assert!(remediation_fixes(&host, "TASK-B") > 0, "{result}");
    let outcome = terminal(&host, &result);
    assert_eq!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
}

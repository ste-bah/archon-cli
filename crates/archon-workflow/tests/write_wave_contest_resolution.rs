//! Issue-112b end to end: a contested shared deliverable is resolved inside
//! the run. Through the real prelude (`resolveContests`), the production
//! write wave (Git, scope grant, post-apply audit) and the host's contest
//! rule: the host names the unconfirmed declarer, the prelude dispatches the
//! one confirmation the host planned, and the verdict decides -- confirmed,
//! the contest resolves; refused, the declarer's remediation runs and the
//! other declarer is asked in turn, each pair once.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::cell::Cell;
use std::rc::Rc;

use archon_workflow::repository_audit::contest::judge;
use archon_workflow::repository_audit::reuse::load_state;
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::audit_contest_plan::confirmation_id;
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use support::{DELETE, Edits, Fixture, git};

const REPORT: &str = "report.json";
const KEEP: &str = "TASK-KEEP";
const DEL: &str = "TASK-DEL";

const SCRIPT: &str = r#"export const meta = { name: 'contests', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-KEEP', file: 'tasks/TASK-KEEP.md', targetFiles: ['keep.txt', 'report.json'] },
  { id: 'TASK-DEL', file: 'tasks/TASK-DEL.md', targetFiles: ['del.txt'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const opts = { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }
const review = await remediateFindings([
  { id: 'keep', canonical_task_ids: ['TASK-KEEP'], severity: 'high', claim: 'refresh the report' },
  { id: 'stray', canonical_task_ids: ['TASK-DEL'], severity: 'high', claim: 'a stray report sits at the root' },
], opts)
const contests = await resolveContests(opts)
return { review, contests }
"#;

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    let task = |id: &str, owns: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![task(KEEP, &["keep.txt", REPORT]), task(DEL, &["del.txt"])],
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

/// The keeper delivers the report and its own file; the deleter fixes its
/// own file and deletes the report as a stray (granted: unclaimed, at the
/// root). A keeper remediation after that re-delivers the report.
fn writes(key: &str, _round: u64, _escalated: bool) -> Edits {
    match key {
        KEEP => edits(vec![("keep.txt", "kept\n"), (REPORT, "{\"report\": 1}\n")]),
        _ => edits(vec![("del.txt", "fixed\n"), (REPORT, DELETE)]),
    }
}

fn host() -> Rc<Host> {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(writes)))
}

/// The contests the host's rule finds now, as (path, state, unconfirmed).
fn contests(host: &Host) -> Vec<(String, String, Vec<String>)> {
    let state = load_state(&host.store).unwrap().unwrap();
    let report = state.ledger.history.last().unwrap().clone();
    let run_dir = host.store.root().parent().unwrap().to_path_buf();
    judge(&run_dir, &report, &host.f.repo)
        .1
        .into_iter()
        .map(|c| (c.declared_path, c.state, c.unconfirmed))
        .collect()
}

fn ran(host: &Host) -> Vec<String> {
    host.answers
        .borrow()
        .iter()
        .filter(|(_, answer)| *answer == Answer::Ran)
        .map(|(id, _)| id.clone())
        .collect()
}

fn outcome(result: &Value, task: &str) -> String {
    result["contests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["taskId"] == task)
        .map(|entry| entry["outcome"].as_str().unwrap().to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_contest_is_resolved_by_the_unconfirmed_declarers_confirmation() {
    let host = host();
    let result = run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    let confirm = format!(
        "verification-wave-{}",
        confirmation_id(KEEP, REPORT, "absent")
    );
    assert!(
        ran(&host).contains(&confirm),
        "{:#?}",
        host.answers.borrow()
    );
    assert_eq!(outcome(&result, KEEP), "confirmed", "{result}");
    let record = host.store.load_call_record(&confirm).unwrap().unwrap();
    assert_eq!(record.dispatched_items[0].canonical_task_ids, [KEEP]);
    let prompt = record.call.options.task.clone().unwrap_or_default();
    assert!(prompt.contains(REPORT) && prompt.contains(DEL), "{prompt}");
    assert!(prompt.contains("does NOT exist"), "{prompt}");
    assert!(
        prompt.contains("no later landing re-created it"),
        "{prompt}"
    );
    assert!(contests(&host).is_empty(), "the declarers agree now");
    assert!(git(&host.f.repo, &["ls-tree", "HEAD", "--", REPORT]).is_empty());
    // Asked once: no further confirmation was planned or dispatched.
    let confirmations = ran(&host)
        .into_iter()
        .filter(|id| id.contains("audit-confirm"))
        .count();
    assert_eq!(confirmations, 1);
}

#[tokio::test]
async fn a_refused_confirmation_routes_to_remediation_and_the_other_declarer_judges_again() {
    let host = host();
    // KEEP: its review round, then a refused confirmation, then the
    // verifier of the remediation that re-delivered the report.
    host.verdicts(
        KEEP,
        vec![Verdict::Accept, Verdict::Refuse(vec![]), Verdict::Accept],
    );
    let result = run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    let keep = format!(
        "verification-wave-{}",
        confirmation_id(KEEP, REPORT, "absent")
    );
    let del = format!(
        "verification-wave-{}",
        confirmation_id(DEL, REPORT, "present")
    );
    let order = ran(&host);
    let at = |id: &str| order.iter().position(|ran| ran == id);
    let remediation = order
        .iter()
        .rposition(|id| {
            id.starts_with("review-remediate-task-keep-")
                && !id.starts_with("review-remediate-task-keep-1-")
        })
        .expect("the keeper's remediation ran");
    assert!(at(&keep).unwrap() < remediation, "{order:#?}");
    assert!(
        remediation < at(&del).expect("the deleter was asked in turn"),
        "{order:#?}"
    );
    assert_eq!(outcome(&result, KEEP), "refused", "{result}");
    assert_eq!(outcome(&result, DEL), "confirmed", "{result}");
    let asked = host.store.load_call_record(&del).unwrap().unwrap();
    let prompt = asked.call.options.task.clone().unwrap_or_default();
    assert!(
        prompt.contains("TASK-DEL deleted it in its landing review-remediate-task-del-1-3")
            && prompt.contains("TASK-KEEP later re-created it")
            && prompt.contains("it currently EXISTS"),
        "{prompt}"
    );
    assert_eq!(
        harness::at_head(&host.f.repo, REPORT),
        "{\"report\": 1}",
        "the keeper re-delivered the report"
    );
    assert!(contests(&host).is_empty(), "{:?}", contests(&host));
    // Each pair once: the keeper's refused confirmation is not asked again.
    assert_eq!(
        order.iter().filter(|id| **id == keep).count(),
        1,
        "{order:#?}"
    );
}

fn host_with(writes: impl Fn(&str, u64, bool) -> Edits + 'static) -> Rc<Host> {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(writes)))
}

fn next(host: Rc<Host>, writes: impl Fn(&str, u64, bool) -> Edits + 'static) -> Rc<Host> {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    let store = WorkflowV2ResultStore::new(host.f.v2.root().to_path_buf());
    Rc::new(Host::new(host.f, store, Box::new(writes)))
}

/// A contest's remediation is its own unit: on a resume every call of the
/// review's remediation of the same task replays -- its round-1 verdict
/// included -- and nothing is dispatched.
#[tokio::test]
async fn a_resume_after_a_contest_remediation_replays_the_review() {
    let first = host();
    first.verdicts(
        KEEP,
        vec![Verdict::Accept, Verdict::Refuse(vec![]), Verdict::Accept],
    );
    run(SCRIPT, NEW_PRELUDE, first.clone()).await;
    let second = next(first, writes);
    run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    let answers = second.answers.borrow().clone();
    assert!(ran(&second).is_empty(), "{answers:#?}");
    assert!(
        answers.iter().any(
            |(id, a)| id == "verification-wave-review-verify-task-keep-1-2"
                && *a == Answer::Replayed
        ),
        "{answers:#?}"
    );
}

/// The keeper's first write delivers the report; every later one only
/// rewrites its own file, so its contest remediation lands nothing.
fn keeper_lands_once() -> impl Fn(&str, u64, bool) -> Edits {
    let keeper_writes = Cell::new(0);
    move |key, round, escalated| {
        if key == KEEP {
            keeper_writes.set(keeper_writes.get() + 1);
            if keeper_writes.get() > 1 {
                return edits(vec![("keep.txt", "kept\n")]);
            }
        }
        writes(key, round, escalated)
    }
}

/// A session that stopped after a refused confirmation and before its
/// routed remediation ended: the resume runs that remediation -- not the
/// verifier again -- and records the pair done.
#[tokio::test]
async fn a_resume_after_a_refused_confirmation_runs_its_remediation_not_a_new_confirmation() {
    let first = host_with(keeper_lands_once());
    first.verdicts(KEEP, vec![Verdict::Accept, Verdict::Refuse(vec![])]);
    run(SCRIPT, NEW_PRELUDE, first.clone()).await;
    let keep = confirmation_id(KEEP, REPORT, "absent");
    let root = first.store.root().to_path_buf();
    // Undo everything the routed remediation left: the session stopped there.
    let contested: Vec<String> = first
        .store
        .load_call_records()
        .unwrap()
        .into_iter()
        .filter(|r| {
            r.call
                .options
                .extra
                .get("remediationContract")
                .is_some_and(|c| c.get("contest").is_some())
                || r.call.id == format!("{keep}-done")
        })
        .map(|r| r.call.id)
        .collect();
    assert!(contested.len() >= 2, "{contested:?}");
    for id in &contested {
        for entry in std::fs::read_dir(root.join("results")).unwrap().flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}-"))
            {
                std::fs::remove_file(entry.path()).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(root.join("branches").join(id));
        let _ = std::fs::remove_dir_all(
            root.parent()
                .unwrap()
                .join("write-coordination/stages")
                .join(id),
        );
    }
    let second = next(first, keeper_lands_once());
    run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    let answers = second.answers.borrow().clone();
    assert!(
        answers
            .iter()
            .all(|(id, _)| *id != format!("verification-wave-{keep}")),
        "the verifier is not asked again: {answers:#?}"
    );
    let remediated = answers.iter().any(|(id, answer)| {
        id.starts_with("review-remediate-task-keep-")
            && !id.starts_with("review-remediate-task-keep-1-")
            && *answer == Answer::Ran
    });
    assert!(remediated, "{answers:#?}");
    assert!(
        second
            .store
            .load_call_record(&format!("{keep}-done"))
            .unwrap()
            .is_some(),
        "the pair is done"
    );
    // Still contested, and never asked again: the final gate names it.
    assert_eq!(contests(&second).len(), 1, "{:?}", contests(&second));
    let third = next(second, keeper_lands_once());
    run(SCRIPT, NEW_PRELUDE, third.clone()).await;
    assert!(ran(&third).is_empty(), "{:#?}", third.answers.borrow());
}

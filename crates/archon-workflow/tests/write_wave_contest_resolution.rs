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
        .rposition(|id| id.starts_with("review-remediate-task-keep-1-"))
        .expect("the keeper's remediation ran");
    assert!(at(&keep).unwrap() < remediation, "{order:#?}");
    assert!(
        remediation < at(&del).expect("the deleter was asked in turn"),
        "{order:#?}"
    );
    assert_eq!(outcome(&result, KEEP), "refused", "{result}");
    assert_eq!(outcome(&result, DEL), "confirmed", "{result}");
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

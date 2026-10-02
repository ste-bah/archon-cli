//! Issue-226 end to end: a declared data root outside both the project and
//! the repository lands only inside a directory the run's policy file lists
//! (`[workflow.acceptance_execution] external_data_roots`), by the
//! project-input landing's own guarantees -- granted on the run's
//! scope-amendment ledger, seeded into the branch's own staging copy,
//! captured, landed by temp+rename after the landing gate with the replaced
//! copy kept, and put back when the unit's verdict refuses it. Every check
//! reads the files back.
//!
//! A branch's own declarations (a missing root created and undone, a
//! read-only directory, an unlisted root, a link out of a listed one, `..`,
//! an empty list) are `write_wave_external_declared`.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

#[path = "support/acceptance_grants_world.rs"]
mod world;

#[path = "support/external_roots_world.rs"]
mod external;

use archon_workflow::task_scope_amendment::{ScopeAmendmentLedger, ScopeGrantRoot};
use archon_workflow::v2::review_finding_ids::finding_id_of;
use external::{
    failure_naming, fix_writing, kept_copies, landed, outside, put, read, run_with, session_with,
};
use harness::{Answer, NEW_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use world::{FIX, SCRIPT, answer, host_round};

const BEFORE: &str = "{\"close\": 0}\n";
const AFTER: &str = "{\"close\": 101}\n";

#[tokio::test]
async fn an_allowlisted_external_root_is_granted_and_its_fix_lands() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = outside();
    let bars = dirs.allowed.join("lake/data/bars.json");
    put(&bars, BEFORE);
    let f = run_with(
        &temp.path().join("scratch"),
        &[&dirs.allowed],
        &[dirs.allowed.join("lake/data/index.json")],
    );
    let (record, round) = host_round(&f, &failure_naming(&[&bars]));
    let routing = record.checks[0].routing.clone().expect("routed");
    let key = bars.display().to_string();
    assert_eq!(
        routing.project_grants.get(&key),
        Some(&vec!["TASK-A".to_string()]),
        "{routing:?}"
    );
    assert!(routing.unwritable.is_empty(), "{routing:?}");
    let ledger = ScopeAmendmentLedger::load(&f.store.run_dir(&f.run)).unwrap();
    assert!(
        (ledger.set.grants.iter()).any(|g| g.task_id == "TASK-A" && g.path == key),
        "{:?}",
        ledger.set.grants
    );
    let host = session_with(f, round, fix_writing(&bars, AFTER), vec![Verdict::Accept]);
    run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    assert_eq!(
        answer(&host, FIX),
        Some(Answer::Ran),
        "{:#?}",
        host.answers.borrow()
    );
    assert_eq!(read(&bars).as_deref(), Some(AFTER), "the fix landed there");
    // The unit is told where the file lands: its tree and its path there.
    let told = format!(
        "{key} lands in the external data root {}, at {key}, through the host's audited project-input landing",
        dirs.allowed.display()
    );
    assert!(
        (host.prompts.borrow().iter()).any(|(id, p)| id == FIX && p.contains(&told)),
        "{told}"
    );
    let lines = landed(&host.f, &bars);
    assert!(
        lines.iter().any(|(outcome, _)| outcome == "applied"),
        "{lines:?}"
    );
    assert!(
        (kept_copies(&host.f).iter()).any(|(_, bytes)| bytes == BEFORE),
        "the replaced copy is kept under the run"
    );
    let project = world::project_root(&host.f);
    assert!(
        !project.join("lake").exists() && !host.f.repo.join("lake").exists(),
        "never the project or the repository"
    );
}

#[tokio::test]
async fn a_refused_external_landing_is_undone_to_the_prior_content() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = outside();
    let bars = dirs.allowed.join("lake/data/bars.json");
    put(&bars, BEFORE);
    let f = run_with(
        &temp.path().join("scratch"),
        &[&dirs.allowed],
        &[dirs.allowed.join("lake/data/index.json")],
    );
    let (_, round) = host_round(&f, &failure_naming(&[&bars]));
    let refuse = vec![Verdict::Refuse(vec![])];
    let host = session_with(f, round, fix_writing(&bars, AFTER), refuse);
    run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    let lines = landed(&host.f, &bars);
    assert!(
        lines.iter().any(|(outcome, _)| outcome == "applied"),
        "it landed first: {lines:?}"
    );
    assert!(
        lines.iter().any(|(outcome, _)| outcome == "reverted"),
        "{lines:?}"
    );
    assert_eq!(read(&bars).as_deref(), Some(BEFORE), "the prior content");
}

const REVIEW: &str = r#"export const meta = { name: 'review-data', description: 'd', phases: [] }
const tasks = [{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] }]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const review = await remediateFindings(FINDINGS, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
return { review }
"#;

/// A review finding that names a file under an allowlisted external root is
/// routed to its owner with an External grant, and the owner's fix lands.
#[tokio::test]
async fn a_review_finding_on_an_external_file_is_fixed_by_its_owner() {
    let temp = tempfile::tempdir().unwrap();
    let dirs = outside();
    let bars = dirs.allowed.join("lake/data/bars.json");
    put(&bars, BEFORE);
    let mut f = run_with(
        &temp.path().join("scratch"),
        &[&dirs.allowed],
        &[dirs.allowed.join("lake/data/index.json")],
    );
    let project = world::project_root(&f);
    f.universe.as_mut().unwrap().source_roots = vec![project.join("tasks").display().to_string()];
    let finding = json!({"id": "stale-close", "canonical_task_ids": ["TASK-A"],
        "severity": "medium", "claim": format!("{} holds a stale value", bars.display())});
    let run_root = f.v2.root().parent().unwrap().to_path_buf();
    let dispose = vec![Verdict::Dispose(vec![(
        finding_id_of(&finding),
        "resolved",
    )])];
    let host = external::session_review(f, fix_writing(&bars, AFTER), dispose);
    let script = REVIEW.replace("FINDINGS", &Value::from(vec![finding]).to_string());
    let result = run(&script, NEW_PRELUDE, host.clone()).await;
    let review = &result["review"];
    assert!(
        review["unresolved"].as_array().is_none_or(Vec::is_empty),
        "{review}"
    );
    let ledger = ScopeAmendmentLedger::load(&run_root).unwrap();
    let key = bars.display().to_string();
    assert!(
        (ledger.set.grants.iter()).any(|g| g.task_id == "TASK-A"
            && g.path == key
            && g.root == ScopeGrantRoot::External
            && g.kind.writable()),
        "{ledger:#?}"
    );
    assert_eq!(
        read(&bars).as_deref(),
        Some(AFTER),
        "the owner's fix landed there"
    );
}

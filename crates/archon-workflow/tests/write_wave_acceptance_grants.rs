//! Batch O2 (ACC-H3) end to end: an acceptance check that fails in a file no
//! task declares, or on stored project data, is remediated through the
//! prelude's `acceptance()` over the production write wave -- and every
//! grant the host routing makes is recorded on the run's chained
//! scope-amendment ledger first, one link per check, so the wave plans with
//! the amended universe and stored data lands through the audited
//! project-input landing, never the repository patch.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

#[path = "support/acceptance_grants_world.rs"]
mod world;

use archon_workflow::task_scope_amendment::{ScopeAmendmentLedger, ScopeGrantRoot};
use harness::{Answer, NEW_PRELUDE, at_head, run};
use support::{Edits, git};
use world::{
    EXTRA, FIX, SCRIPT, STORED, answer, fixture, host_round, project_root, session,
    with_project_inputs,
};

/// The run records the acceptance policy's project root, as a live launch
/// does, so stored data can land through the project inputs.
fn with_project_policy(f: &support::Fixture, scratch: &std::path::Path) {
    with_project_inputs(f, scratch, &[".archon/lab/data", ".archon/store/data"]);
}

fn fix_extra(_key: &str, _round: u64, _escalated: bool) -> Edits {
    Edits {
        files: vec![(EXTRA, "// extra, fixed\n")],
        report: vec![EXTRA],
        via_adapter: false,
    }
}

fn fix_stored(_key: &str, _round: u64, _escalated: bool) -> Edits {
    Edits {
        files: vec![
            ("crates/a/src/lib.rs", "// a, fixed\n"),
            (STORED, "{\"close\": 101}\n"),
        ],
        report: vec!["crates/a/src/lib.rs"],
        via_adapter: false,
    }
}

#[tokio::test]
async fn a_file_no_task_declares_is_granted_on_the_ledger_and_its_fix_lands() {
    let f = fixture();
    let panic = format!("thread 'main' panicked at {EXTRA}:3:5:\nboom\n");
    let (record, round) = host_round(&f, &panic);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.granted_files, [EXTRA]);
    // Recorded before anything is dispatched: one link, naming the check.
    let run_root = f.store.run_dir(&f.run);
    let ledger = ScopeAmendmentLedger::load(&run_root).unwrap();
    assert_eq!(ledger.lineage.len(), 1, "{ledger:?}");
    assert!(ledger.lineage[0].trigger.contains("acceptance check AC-1"));
    assert!(
        (ledger.set.grants.iter())
            .any(|g| g.task_id == "TASK-A" && g.path == EXTRA && g.kind.writable())
    );
    let host = session(f, round, fix_extra);
    run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    assert_eq!(
        answer(&host, FIX),
        Some(Answer::Ran),
        "{:#?}",
        host.answers.borrow()
    );
    assert_eq!(at_head(&host.f.repo, EXTRA), "// extra, fixed");
    let prompts = host.prompts.borrow();
    assert!(
        (prompts.iter()).any(|(id, p)| id == FIX && p.contains("granted to this unit")),
        "the unit is told the file is its"
    );
}

#[tokio::test]
async fn stored_project_data_a_failure_names_lands_through_the_project_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture();
    with_project_policy(&f, &temp.path().join("scratch"));
    let project = project_root(&f);
    let failure = format!(
        "AssertionError: {} holds a stale close\n",
        project.join(STORED).display()
    );
    let (record, round) = host_round(&f, &failure);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(
        routing.project_grants.get(STORED),
        Some(&vec!["TASK-A".to_string()]),
        "{routing:?}"
    );
    assert!(routing.granted_files.is_empty(), "never a script target");
    let ledger = ScopeAmendmentLedger::load(&f.store.run_dir(&f.run)).unwrap();
    assert!(
        (ledger.set.grants.iter()).any(|g| g.path == STORED && g.root == ScopeGrantRoot::Project)
    );
    let host = session(f, round, fix_stored);
    run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    assert_eq!(
        answer(&host, FIX),
        Some(Answer::Ran),
        "{:#?}",
        host.answers.borrow()
    );
    assert_eq!(
        std::fs::read_to_string(project.join(STORED)).unwrap(),
        "{\"close\": 101}\n",
        "the fix reached the project's stored data"
    );
    let log = std::fs::read_to_string(
        host.f
            .store
            .run_dir(&host.f.run)
            .join("write-coordination/project-inputs.jsonl"),
    )
    .unwrap();
    assert!(
        log.lines()
            .any(|line| line.contains(STORED) && line.contains("\"applied\"")),
        "{log}"
    );
    assert_eq!(
        git(&host.f.repo, &["ls-files", ".archon"]),
        "",
        "never the patch"
    );
    let prompts = host.prompts.borrow();
    assert!(
        (prompts.iter()).any(|(id, p)| id == FIX
            && support::contains_path_text(
                p,
                &format!(
                    "STORED DATA granted to this unit: {STORED} lands in the project root, at {}",
                    project_root(&host.f).join(STORED).display()
                )
            )),
        "the unit is told it may fix the stored data: {:?}",
        (prompts.iter())
            .flat_map(|(_, p)| p.lines().filter(|l| l.contains("STORED DATA")))
            .collect::<Vec<_>>()
    );
}

/// Stored data is what lives under a root the run's own records declare --
/// the acceptance policy's inputs and the directories of the artifacts the
/// task set declares -- never every project-data path a failure names: a
/// declared artifact's directory is granted, a store no record declares is
/// not.
#[test]
fn stored_data_roots_come_from_the_runs_records_never_a_path_convention() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    with_project_inputs(&f, &temp.path().join("scratch"), &[".archon/lab/data"]);
    let task = &mut f.universe.as_mut().unwrap().tasks[0];
    task.artifact_requirements = vec![".archon/vault/records/index.json".into()];
    let project = project_root(&f);
    let declared = ".archon/vault/records/entries.json";
    std::fs::create_dir_all(project.join(".archon/vault/records")).unwrap();
    std::fs::write(project.join(declared), "{}\n").unwrap();
    let failure = format!(
        "AssertionError: {} and {} disagree\n",
        project.join(declared).display(),
        project.join(STORED).display()
    );
    let (record, _) = host_round(&f, &failure);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(
        routing.project_grants.keys().collect::<Vec<_>>(),
        [declared],
        "{routing:?}"
    );
    assert_eq!(routing.project_grants[declared], ["TASK-A".to_string()]);
}

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

use std::path::PathBuf;
use std::rc::Rc;

use archon_workflow::task_scope_amendment::{ScopeAmendmentLedger, ScopeGrantRoot};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::acceptance_routing::{
    mark_blocked, record_routed_grants, recorded_ownership, reroute, route_failures_owned,
};
use archon_workflow::v2::acceptance_stage::{
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, AcceptanceRoundRecordV1,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const SCRIPT: &str = r#"export const meta = { name: 'grants', description: 'd', phases: [] }
const tasks = [{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] }]
const byId = (id) => tasks.find((t) => t.id === id) || {}
return await acceptance({ maxRounds: 2, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

const FIX: &str = "review-remediate-task-a-1-1";
const EXTRA: &str = "crates/a/src/extra.rs";
const STORED: &str = ".archon/store/data/bars.json";

fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
    .canonicalize()
    .unwrap()
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
        (EXTRA, "// extra\n"),
        (".gitignore", ".archon/*\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            source_path: "tasks/TASK-A.md".into(),
            files_expected_to_change: vec!["crates/a/src/lib.rs".into()],
            implements: vec!["AC-1".into()],
            ..Default::default()
        }],
    });
    f
}

/// The run records the acceptance policy's project root, as a live launch
/// does, so stored data can land through the project inputs.
fn with_project_policy(f: &Fixture, scratch: &std::path::Path) {
    let project = project_root(f);
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::create_dir_all(project.join(".archon/lab/data")).unwrap();
    let stored = project.join(STORED);
    std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
    std::fs::write(&stored, "{\"close\": 0}\n").unwrap();
    let policy = json!({
        "repository": f.repo.canonicalize().unwrap(), "project": project,
        "task_root": project.join("tasks"), "scratch_parent": scratch,
        // The store's data root is one the run's acceptance policy records.
        "project_inputs": [".archon/lab/data", ".archon/store/data"], "project_input_excludes": [],
        "combined": true, "toolchain_path": "/usr/bin:/bin", "environment": {},
        "environment_allowlist": [], "cargo_seed": null, "timeout_secs": 60,
        "output_bytes": 4096, "scratch_bytes": 1u64 << 30,
    });
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy, "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
    let path = f.store.run_dir(&f.run).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
}

/// The host's acceptance round 1, routed and recorded exactly as the live
/// stage does after running its checks: AC-1 failed with `stderr`.
fn host_round(f: &Fixture, stderr: &str) -> (AcceptanceRoundRecordV1, Value) {
    let check = AcceptanceCheckRecordV1 {
        check_id: "AC-1".into(),
        criterion: "the gate holds".into(),
        kind: "command".into(),
        status: AcceptanceCheckStatus::Failed,
        exit_code: Some(1),
        operational_error: None,
        owning_tasks: vec!["TASK-A".into()],
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    };
    let mut record = AcceptanceRoundRecordV1 {
        schema_version: 1,
        run_id: f.run.clone(),
        call_id: "acceptance-contract-run-1".into(),
        round: 1,
        attempt: 1,
        max_rounds: 2,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks: vec![check],
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    };
    let run_root = f.store.run_dir(&f.run);
    let universe = f.universe.as_ref();
    let owned = recorded_ownership(&run_root).unwrap();
    route_failures_owned(universe, &f.repo, &[], &owned, &mut record);
    mark_blocked(&mut record);
    reroute(universe, &mut record);
    record_routed_grants(&run_root, universe, &f.repo, &mut record);
    assert!(record.operational_errors.is_empty(), "{record:?}");
    // The failing entry as the live stage renders it to the script.
    let mut view = serde_json::to_value(&record.checks[0]).unwrap();
    view["remediable"] = json!(true);
    let data = json!({ "round": 1, "final": false, "passed": [], "operational_errors": [],
        "contract_present": true, "failing": [view] });
    (record, data)
}

fn session(f: Fixture, round: Value, edits: fn(&str, u64, bool) -> Edits) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(edits)));
    host.verdicts("TASK-A", vec![Verdict::Accept, Verdict::Accept]);
    host.acceptance.borrow_mut().push_back(round);
    host
}

fn answer(host: &Host, id: &str) -> Option<Answer> {
    (host.answers.borrow().iter())
        .find(|(call, _)| call == id)
        .map(|(_, answer)| answer.clone())
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
        (prompts.iter()).any(|(id, p)| id == FIX && p.contains("STORED PROJECT DATA")),
        "the unit is told it may fix the stored data"
    );
}

//! The acceptance-grant world shared by the grant e2e suites: one task, a
//! run whose records name a project root apart from the repository, the
//! host's acceptance round routed and recorded exactly as the live stage
//! does, and a session that drives the prelude's `acceptance()` over the
//! production write wave.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::acceptance_routing::{
    mark_blocked, record_routed_grants, recorded_ownership, reroute, route_failures_owned,
};
use archon_workflow::v2::acceptance_stage::{
    AcceptanceCheckRecordV1, AcceptanceCheckStatus, AcceptanceRoundRecordV1,
};
use archon_workflow::*;
use serde_json::{Value, json};

use super::harness::{Answer, Host, Verdict};
use super::support::{Edits, Fixture, git};

pub const SCRIPT: &str = r#"export const meta = { name: 'grants', description: 'd', phases: [] }
const tasks = [{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] }]
const byId = (id) => tasks.find((t) => t.id === id) || {}
return await acceptance({ maxRounds: 2, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

pub const FIX: &str = "review-remediate-task-a-1-1";
pub const EXTRA: &str = "crates/a/src/extra.rs";
pub const STORED: &str = ".archon/store/data/bars.json";

pub fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
    .canonicalize()
    .unwrap()
}

pub fn fixture() -> Fixture {
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
/// does, with `inputs` as its project inputs.
pub fn with_project_inputs(f: &Fixture, scratch: &Path, inputs: &[&str]) {
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
        "project_inputs": inputs, "project_input_excludes": [],
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
pub fn host_round(f: &Fixture, stderr: &str) -> (AcceptanceRoundRecordV1, Value) {
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

pub fn session(f: Fixture, round: Value, edits: fn(&str, u64, bool) -> Edits) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(edits)));
    host.verdicts("TASK-A", vec![Verdict::Accept, Verdict::Accept]);
    host.acceptance.borrow_mut().push_back(round);
    host
}

pub fn answer(host: &Host, id: &str) -> Option<Answer> {
    (host.answers.borrow().iter())
        .find(|(call, _)| call == id)
        .map(|(_, answer)| answer.clone())
}

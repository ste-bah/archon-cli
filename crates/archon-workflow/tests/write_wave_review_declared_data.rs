//! Issue-223, review remediation end to end: a finding that names stored
//! data under a root the run's records declare -- outside `.archon/` and
//! outside the repository -- is granted to the finding's owner on the run's
//! scope-amendment ledger (the ledger's own root list, never a path
//! convention), and the owner's fix lands in the project through the
//! audited project-input landing.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

#[path = "support/acceptance_grants_world.rs"]
mod world;

use std::rc::Rc;

use archon_workflow::task_scope_amendment::{ScopeAmendmentLedger, ScopeGrantRoot};
use archon_workflow::v2::review_finding_ids::finding_id_of;
use archon_workflow::*;
use harness::{Host, NEW_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use support::Edits;
use world::{fixture, project_root, with_project_inputs};

const LAKE: &str = "lake/data/bars.json";

const SCRIPT: &str = r#"export const meta = { name: 'review-data', description: 'd', phases: [] }
const tasks = [{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] }]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const review = await remediateFindings(FINDINGS, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
return { review }
"#;

fn fix(_key: &str, _round: u64, _escalated: bool) -> Edits {
    Edits {
        files: vec![
            ("crates/a/src/lib.rs", "// a, fixed\n"),
            ("@copy:lake/data/bars.json", "{\"close\": 101}\n"),
        ],
        report: vec!["crates/a/src/lib.rs"],
        via_adapter: false,
    }
}

#[tokio::test]
async fn a_finding_on_data_under_a_declared_non_archon_root_is_fixed_by_its_owner() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    with_project_inputs(&f, &temp.path().join("scratch"), &[".archon/lab/data"]);
    let project = project_root(&f);
    std::fs::create_dir_all(project.join("lake/data")).unwrap();
    std::fs::write(project.join(LAKE), "{\"close\": 0}\n").unwrap();
    let universe = f.universe.as_mut().unwrap();
    universe.source_roots = vec![project.join("tasks").display().to_string()];
    // Only TASK-A's artifact declaration makes `lake/data` a data root.
    universe.tasks[0].artifact_requirements = vec!["lake/data/index.json".into()];
    let finding = json!({"id": "stale-close", "canonical_task_ids": ["TASK-A"],
        "severity": "medium",
        "claim": format!("{} holds a stale close", project.join(LAKE).display())});
    let run_root = f.v2.root().parent().unwrap().to_path_buf();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(fix)));
    host.verdicts(
        "TASK-A",
        vec![Verdict::Dispose(vec![(
            finding_id_of(&finding),
            "resolved",
        )])],
    );
    let script = SCRIPT.replace("FINDINGS", &Value::from(vec![finding]).to_string());
    let result = run(&script, NEW_PRELUDE, host.clone()).await;
    let review = &result["review"];
    assert!(
        review["unresolved"].as_array().is_none_or(Vec::is_empty),
        "{review}"
    );
    let ledger = ScopeAmendmentLedger::load(&run_root).unwrap();
    assert!(
        (ledger.set.grants.iter()).any(|g| g.task_id == "TASK-A"
            && g.path == LAKE
            && g.root == ScopeGrantRoot::Project
            && g.kind.writable()),
        "{ledger:#?}"
    );
    assert_eq!(
        std::fs::read_to_string(project.join(LAKE)).unwrap(),
        "{\"close\": 101}\n",
        "the owner's fix reached the declared root"
    );
}

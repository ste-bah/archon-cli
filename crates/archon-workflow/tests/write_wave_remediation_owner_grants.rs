//! Batch O, wave level: the ownership map is not write scope.
//!
//! A file no task declares that a task's declared code uses is recorded as
//! that task's (an ownership record), and is writable for a unit of it only
//! once something routed to the unit names it. Through the production write
//! wave with real Git writes and the host's remediation plan:
//!
//! - a finding naming no task but an owned file is routed to the file's
//!   owner, the owner's unit is granted the file on demand (one logged link
//!   for the unit), and its fix lands a change to it;
//! - owned files nothing names stay ownership records: no unit gets them as
//!   targets and the write universe does not declare them.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::rc::Rc;

use archon_workflow::task_scope_amendment::{
    ScopeAmendmentLedger, ScopeGrantKind, amended_universe,
};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::review_finding_ids::finding_id_of;
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const A: &str = "crates/a/src/lib.rs";
const A_HELPER: &str = "crates/a/src/helper.rs";
const A_OTHER: &str = "crates/a/src/other.rs";
const B: &str = "crates/b/src/lib.rs";
const B_UTIL: &str = "crates/b/src/util.rs";

const SCRIPT: &str = r#"export const meta = { name: 'owner-grants', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['crates/b/src/lib.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const review = await remediateFindings(FINDINGS, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
return { review }
"#;

fn findings() -> Vec<Value> {
    vec![
        // Names no task: only an owned-but-undeclared file.
        json!({"id": "helper-panic", "severity": "medium",
            "claim": "crates/a/src/helper.rs panics on an empty input"}),
        // Names its own task's declared file only.
        json!({"id": "b-bounds", "canonical_task_ids": ["TASK-B"], "severity": "low",
            "claim": "crates/b/src/lib.rs accepts an empty range"}),
    ]
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        (
            A,
            "mod helper;\nmod other;\npub fn run() { helper::go(); }\n",
        ),
        (A_HELPER, "pub fn go() {}\n"),
        (A_OTHER, "pub fn idle() {}\n"),
        ("crates/b/Cargo.toml", "[package]\nname = \"b\"\n"),
        (B, "mod util;\npub fn range() { util::check(); }\n"),
        (B_UTIL, "pub fn check() {}\n"),
    ] {
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

/// TASK-A's fix changes the helper the finding names; TASK-B's its own file.
fn writes(key: &str, _round: u64, _escalated: bool) -> Edits {
    let path = if key == "TASK-A" { A_HELPER } else { B };
    Edits {
        report: vec![path],
        files: vec![(path, "// fixed\n")],
        via_adapter: false,
    }
}

/// What TASK's first fix branch was dispatched with as its declared
/// targets: the host's own stamp, after every scope floor.
fn fix_targets(host: &Host, task: &str) -> Vec<String> {
    let calls = host.calls.borrow();
    let call = calls
        .iter()
        .find(|call| {
            call.write_mode.is_some()
                && call.options.extra["remediationContract"]["taskId"] == json!(task)
        })
        .unwrap_or_else(|| panic!("a fix for {task}"));
    serde_json::from_value(
        host.f
            .input_stamp(&call.id, agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY),
    )
    .unwrap()
}

#[tokio::test]
async fn an_owned_file_is_granted_to_its_owners_unit_only_when_routed_work_names_it() {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let run_root = f.v2.root().parent().unwrap().to_path_buf();
    let host = Rc::new(Host::new(f, store, Box::new(writes)));
    let ids: Vec<String> = findings().iter().map(finding_id_of).collect();
    host.verdicts(
        "TASK-A",
        vec![Verdict::Dispose(vec![(ids[0].clone(), "resolved")])],
    );
    host.verdicts(
        "TASK-B",
        vec![Verdict::Dispose(vec![(ids[1].clone(), "resolved")])],
    );
    let script = SCRIPT.replace("FINDINGS", &Value::from(findings()).to_string());
    let result = run(&script, NEW_PRELUDE, host.clone()).await;
    let review = &result["review"];
    assert!(
        review["unresolved"].as_array().is_none_or(Vec::is_empty),
        "{review}"
    );

    // The ledger: ownership for every owned file, a write grant only for the
    // one a routed finding names, logged against its unit.
    let ledger = ScopeAmendmentLedger::load(&run_root).unwrap();
    let kind_of = |task: &str, path: &str| {
        ledger
            .set
            .grants
            .iter()
            .find(|g| g.task_id == task && g.path == path)
            .map(|g| g.kind)
    };
    assert_eq!(
        kind_of("TASK-A", A_HELPER),
        Some(ScopeGrantKind::OwnerlessAssignment),
        "{ledger:#?}"
    );
    assert_eq!(kind_of("TASK-A", A_OTHER), Some(ScopeGrantKind::Owner));
    assert_eq!(kind_of("TASK-B", B_UTIL), Some(ScopeGrantKind::Owner));
    assert_eq!(kind_of("TASK-B", A_HELPER), None, "never another task's");
    let helper = ledger
        .set
        .grants
        .iter()
        .find(|g| g.task_id == "TASK-A" && g.path == A_HELPER)
        .unwrap();
    assert!(helper.evidence.contains(&ids[0]), "{helper:?}");
    assert!(
        ledger
            .lineage
            .iter()
            .any(|link| link.trigger.contains("unit TASK-A")
                && link.changed_task_ids.contains("TASK-A")),
        "one link logs the unit's grant: {:#?}",
        ledger.lineage
    );

    // Dispatch scope: the write universe declares the named file for TASK-A
    // alone, and neither unnamed owned file for anyone.
    let universe = host.f.universe.clone().unwrap();
    let writes = amended_universe(&universe, &ledger.set);
    let declares = |task: &str, path: &str| {
        writes
            .tasks
            .iter()
            .find(|t| t.canonical_task_id == task)
            .unwrap()
            .files_expected_to_change
            .iter()
            .any(|entry| entry.contains(path))
    };
    assert!(declares("TASK-A", A_HELPER));
    assert!(!declares("TASK-A", A_OTHER));
    assert!(!declares("TASK-B", B_UTIL));

    // The units the script dispatched: the ownerless finding went to the
    // helper's owner, with the helper a target; nothing unnamed is a target.
    let a_targets = fix_targets(&host, "TASK-A");
    assert!(a_targets.iter().any(|t| t == A_HELPER), "{a_targets:?}");
    assert!(!a_targets.iter().any(|t| t == A_OTHER), "{a_targets:?}");
    let b_targets = fix_targets(&host, "TASK-B");
    assert!(!b_targets.iter().any(|t| t == B_UTIL), "{b_targets:?}");

    // And the owner's fix landed its change to the granted file.
    assert!(
        host.answers
            .borrow()
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "nothing refused at dispatch"
    );
    let landed = host
        .store
        .load_call_records()
        .unwrap()
        .into_iter()
        .filter(|record| record.call.write_mode.is_some())
        .any(|record| {
            record
                .result
                .files_changed
                .iter()
                .any(|file| file.path == A_HELPER)
        });
    assert!(landed, "the granted helper change landed");
    let head = git(&host.f.repo, &["show", "HEAD:crates/a/src/helper.rs"]);
    assert_eq!(head.trim(), "// fixed");
}

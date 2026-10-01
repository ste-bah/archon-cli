//! Batch O2, wave level: the unit's own verifier names an owned file.
//!
//! A file no task declares that a task's declared code uses is an ownership
//! record, not write scope. When the verifier of a unit of its owner leaves
//! a finding open and names that file as where the fix still has to change,
//! the host grants the unit the file (one logged link, marked as the
//! verifier's), the next round's branch is dispatched with it as a declared
//! target, and that round's fix lands its change. An owned file nothing
//! names stays an ownership record throughout.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::rc::Rc;

use archon_workflow::task_scope_amendment::{ScopeAmendmentLedger, ScopeGrantKind};
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

const SCRIPT: &str = r#"export const meta = { name: 'verifier-grants', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['crates/b/src/lib.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const review = await remediateFindings(FINDINGS, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
return { review }
"#;

fn findings() -> Vec<Value> {
    // Names only its own task's declared file: nothing grants the helper up
    // front.
    vec![
        json!({"id": "a-empty", "canonical_task_ids": ["TASK-A"], "severity": "medium",
        "claim": "crates/a/src/lib.rs panics on an empty input"}),
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

/// Round 1 changes the declared file; round 2 the helper the verifier named.
fn writes(_key: &str, round: u64, _escalated: bool) -> Edits {
    let path = if round == 1 { A } else { A_HELPER };
    Edits {
        report: vec![path],
        files: vec![(path, "// fixed\n")],
        via_adapter: false,
    }
}

/// The declared targets TASK-A's `nth` fix branch was dispatched with: the
/// host's own stamp, after every scope floor.
fn fix_targets(host: &Host, nth: usize) -> Vec<String> {
    let calls = host.calls.borrow();
    let fixes: Vec<&WorkflowV2HostCall> = calls
        .iter()
        .filter(|call| {
            call.write_mode.is_some()
                && call.options.extra["remediationContract"]["taskId"] == json!("TASK-A")
        })
        .collect();
    let call = fixes
        .get(nth)
        .unwrap_or_else(|| panic!("fix {nth} of {}", fixes.len()));
    serde_json::from_value(
        host.f
            .input_stamp(&call.id, agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY),
    )
    .unwrap()
}

#[tokio::test]
async fn a_verifier_naming_an_owned_file_grants_it_to_the_next_round_which_lands_it() {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let run_root = f.v2.root().parent().unwrap().to_path_buf();
    let host = Rc::new(Host::new(f, store, Box::new(writes)));
    let id = finding_id_of(&findings()[0]);
    host.verdicts(
        "TASK-A",
        vec![
            Verdict::DisposeNaming(vec![(id.clone(), "open")], A_HELPER),
            Verdict::Dispose(vec![(id.clone(), "resolved")]),
        ],
    );
    let script = SCRIPT.replace("FINDINGS", &Value::from(findings()).to_string());
    let result = run(&script, NEW_PRELUDE, host.clone()).await;
    let review = &result["review"];
    assert!(
        review["unresolved"].as_array().is_none_or(Vec::is_empty),
        "{review}"
    );
    let resolved = serde_json::to_string(&review["resolved"]).unwrap();
    assert!(resolved.contains(&id), "{review}");

    // Round 1 could not write the helper; round 2 was dispatched with it.
    let first = fix_targets(&host, 0);
    assert!(!first.iter().any(|t| t == A_HELPER), "{first:?}");
    let second = fix_targets(&host, 1);
    assert!(second.iter().any(|t| t == A_HELPER), "{second:?}");
    assert!(!second.iter().any(|t| t == A_OTHER), "{second:?}");

    // The ledger: the verifier's grant, logged for the unit; the unnamed
    // owned files stay ownership records.
    let ledger = ScopeAmendmentLedger::load(&run_root).unwrap();
    let grant = |task: &str, path: &str| {
        ledger
            .set
            .grants
            .iter()
            .find(|g| g.task_id == task && g.path == path)
            .cloned()
    };
    let helper = grant("TASK-A", A_HELPER).expect("the helper is granted");
    assert_eq!(
        helper.kind,
        ScopeGrantKind::OwnerlessAssignment,
        "{helper:?}"
    );
    assert_eq!(helper.evidence, "the unit's verifier names it");
    assert_eq!(
        grant("TASK-A", A_OTHER).map(|g| g.kind),
        Some(ScopeGrantKind::Owner)
    );
    assert_eq!(
        grant("TASK-B", B_UTIL).map(|g| g.kind),
        Some(ScopeGrantKind::Owner)
    );
    assert!(
        ledger
            .lineage
            .iter()
            .any(|link| link.trigger.contains("(verifier)")
                && link.changed_task_ids.contains("TASK-A")),
        "{:#?}",
        ledger.lineage
    );

    // Nothing refused, and round 2's change to the helper landed.
    assert!(
        host.answers
            .borrow()
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "nothing refused at dispatch"
    );
    let head = git(&host.f.repo, &["show", "HEAD:crates/a/src/helper.rs"]);
    assert_eq!(head.trim(), "// fixed");
}

//! Batch O, wave level: review remediation judged per finding, through the
//! production write wave with real Git writes, the host's remediation plan
//! and the host's per-finding reading of each verifier.
//!
//! One finding a verifier leaves open in round 1 and closes in round 2 is
//! closed; its sibling, closed in round 1, is never sent again. A finding
//! every verifier leaves open is still open when its cycle closes nothing,
//! is reported open by its own id, and holds the run.
#[path = "support/acceptance_ran.rs"]
mod acceptance_ran;
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::review_finding_ids::finding_id_of;
use archon_workflow::v2::script::{
    AuthoredRunFacts, AuthoredRunOutcome, authored_call_facts, authored_run_terminal_status_with,
    writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const A: &str = "crates/a/src/lib.rs";
const B: &str = "crates/b/src/lib.rs";

const SCRIPT: &str = r#"export const meta = { name: 'per-finding', description: 'd', phases: [] }
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
        json!({"id": "a-retry", "canonical_task_ids": ["TASK-A"], "severity": "high",
            "claim": "crates/a/src/lib.rs never retries a failed read"}),
        json!({"id": "a-error", "canonical_task_ids": ["TASK-A"], "severity": "low",
            "claim": "crates/a/src/lib.rs drops the error text"}),
        json!({"id": "b-bounds", "canonical_task_ids": ["TASK-B"], "severity": "medium",
            "claim": "crates/b/src/lib.rs accepts an empty range"}),
    ]
}

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
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-A", A), task("TASK-B", B)],
    });
    f
}

/// Every round's fix lands a change of its own, so each is verified.
fn writes(key: &str, round: u64, _escalated: bool) -> Edits {
    let path = if key == "TASK-A" { A } else { B };
    let content = match round {
        1 => "// round 1\n",
        2 => "// round 2\n",
        _ => "// round 3\n",
    };
    Edits {
        report: vec![path],
        files: vec![(path, content)],
        via_adapter: false,
    }
}

fn terminal(host: &Host, result: &Value) -> AuthoredRunOutcome {
    let calls = host.calls.borrow().clone();
    // REM-13: the prelude ran the acceptance stage after the script; the
    // rule is judged on the round it recorded.
    let ran = acceptance_ran::AcceptanceRan::of(&host.store);
    let facts = authored_call_facts(&calls, |id| host.store.load_call_record(id)).unwrap();
    let accounting = json!({"accepted": [], "blocked": [], "adversarial_findings": findings(),
        "uncovered_requirements": [], "review_remediation": result["review"]})
    .to_string();
    let universe = host.f.universe.as_ref().unwrap();
    let universe_tasks: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.clone())
        .collect();
    authored_run_terminal_status_with(
        &AuthoredRunFacts {
            accumulated_status: WorkflowV2Status::NeedsReview,
            host_terminal_failure: None,
            script_result: Some(&accounting),
            acceptance_gate: ran.fact(&facts),
            calls: &facts,
            writable_tasks: &writable_task_ids(Some(universe)),
            universe_tasks: &universe_tasks,
        },
        &BTreeSet::new(),
    )
}

#[tokio::test]
async fn a_refused_finding_closes_in_round_two_and_an_open_one_holds_the_run() {
    let f = fixture();
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(writes)));
    let ids: Vec<String> = findings().iter().map(finding_id_of).collect();
    let (retry, error, bounds) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    host.verdicts(
        "TASK-A",
        vec![
            // Round 1: the error text is fixed; the retry is refused.
            Verdict::Dispose(vec![(retry.clone(), "open"), (error.clone(), "resolved")]),
            // Round 2 is sent the retry alone, and closes it.
            Verdict::Dispose(vec![(retry.clone(), "resolved")]),
        ],
    );
    host.verdicts(
        "TASK-B",
        vec![
            Verdict::Dispose(vec![(bounds.clone(), "open")]),
            Verdict::Dispose(vec![(bounds.clone(), "open")]),
        ],
    );
    let script = SCRIPT.replace("FINDINGS", &Value::from(findings()).to_string());
    let result = run(&script, NEW_PRELUDE, host.clone()).await;

    // The host planned every finding, by the id every later rule reads.
    let review = &result["review"];
    let resolved: Vec<&Value> = review["resolved"].as_array().unwrap().iter().collect();
    assert_eq!(resolved.len(), 1, "{review}");
    assert_eq!(resolved[0]["taskId"], json!("TASK-A"));
    let mut closed: Vec<String> =
        serde_json::from_value(resolved[0]["findingIds"].clone()).unwrap();
    closed.sort();
    let mut want = vec![retry.clone(), error.clone()];
    want.sort();
    assert_eq!(closed, want);
    let unresolved = review["unresolved"].as_array().unwrap();
    assert_eq!(unresolved.len(), 1, "{review}");
    assert_eq!(unresolved[0]["findingId"], json!(bounds));
    assert_eq!(unresolved[0]["outcome"], json!("unverified"));

    // Round 2 of TASK-A carried only the finding round 1 left open.
    let calls = host.calls.borrow().clone();
    let a_fixes: Vec<&WorkflowV2HostCall> = calls
        .iter()
        .filter(|call| {
            call.write_mode.is_some()
                && call.options.extra["remediationContract"]["taskId"] == json!("TASK-A")
        })
        .collect();
    assert_eq!(a_fixes.len(), 2, "two rounds for TASK-A");
    assert_eq!(
        a_fixes[1].options.extra["remediationContract"]["findingIds"],
        json!([retry])
    );
    // TASK-B: a cycle that closed nothing ends the unit (two rounds, no more).
    let b_fixes = calls
        .iter()
        .filter(|call| {
            call.write_mode.is_some()
                && call.options.extra["remediationContract"]["taskId"] == json!("TASK-B")
        })
        .count();
    assert_eq!(b_fixes, 2);
    assert!(
        host.answers
            .borrow()
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "nothing refused at dispatch"
    );

    // The terminal rule, from the host's records alone.
    let outcome = terminal(&host, &result);
    assert_ne!(
        outcome.status,
        WorkflowV2Status::Accepted,
        "{}",
        outcome.explanation()
    );
    let held_on = |id: &str| outcome.blocking.iter().any(|clause| clause.contains(id));
    assert!(held_on(&bounds), "{}", outcome.explanation());
    assert!(!held_on(&retry), "{}", outcome.explanation());
    assert!(!held_on(&error), "{}", outcome.explanation());
}

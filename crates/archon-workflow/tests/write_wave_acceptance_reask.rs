//! Batch H end to end: the prelude's `acceptance()` over the production
//! write wave, resumed. A recorded remediation answer replays only for the
//! question it answered -- the findings AND the acceptance round they were
//! read from, which the host re-runs on every resume.
//!
//! Live on wf-0ddadd81 a check that still failed after its fix landed was
//! re-asked, with the same failure text, on every resume, and every time the
//! first fix's recorded answer was replayed for it: the task never got a
//! second attempt.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const SCRIPT: &str = r#"export const meta = { name: 'reask', description: 'd', phases: [] }
const tasks = [{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] }]
const byId = (id) => tasks.find((t) => t.id === id) || {}
return await acceptance({ maxRounds: 2, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

const FIX: &str = "review-remediate-task-a-1-1";

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
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
            ..Default::default()
        }],
    });
    f
}

/// Each dispatched fix writes a line of its own, so a re-run lands.
fn edits(_key: &str, _round: u64, _escalated: bool) -> Edits {
    let line: &'static str = Box::leak(
        format!(
            "// fixed {}\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
        .into_boxed_str(),
    );
    Edits {
        files: vec![("crates/a/src/lib.rs", line)],
        report: vec!["crates/a/src/lib.rs"],
        via_adapter: false,
    }
}

/// Round 1 of acceptance: check AC-1, owned by TASK-A, failed with `stderr`.
fn failing(stderr: &str) -> Value {
    json!({ "round": 1, "final": false, "passed": [], "operational_errors": [],
        "contract_present": true,
        "failing": [{ "check_id": "AC-1", "criterion": "the gate holds", "kind": "command",
            "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-A"], "stderr_tail": stderr }] })
}

fn session(f: Fixture, rounds: Vec<Value>) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(edits)));
    host.verdicts("TASK-A", vec![Verdict::Accept, Verdict::Accept]);
    host.acceptance.borrow_mut().extend(rounds);
    host
}

fn done(host: Rc<Host>) -> Fixture {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("session still referenced")
    };
    host.f
}

fn answer(host: &Host, id: &str) -> Option<Answer> {
    host.answers
        .borrow()
        .iter()
        .find(|(call, _)| call == id)
        .map(|(_, answer)| answer.clone())
}

/// Session 1: acceptance round 1 fails AC-1 with `stderr`; TASK-A's fix runs
/// and lands, its verdict accepts, round 2 is clean.
async fn first_session(stderr: &str) -> Fixture {
    let first = session(fixture(), vec![failing(stderr)]);
    run(SCRIPT, NEW_PRELUDE, first.clone()).await;
    assert_eq!(answer(&first, FIX), Some(Answer::Ran));
    assert_ne!(at_head(&first.f.repo, "crates/a/src/lib.rs"), "// a");
    done(first)
}

/// The acceptance round, re-run on the resume, observes the SAME failure on
/// a tree that already holds the fix: the fix did not hold, and its answer
/// is no answer to this observation. Under the same call id (the prelude
/// takes the same path) the fix is dispatched again.
#[tokio::test]
async fn the_same_failure_observed_after_its_fix_landed_is_fixed_again() {
    let f = first_session("AssertionError: unregistered dataset ref x").await;
    let landed = at_head(&f.repo, "crates/a/src/lib.rs");
    let second = session(
        f,
        vec![failing("AssertionError: unregistered dataset ref x")],
    );
    run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    assert_eq!(
        answer(&second, FIX),
        Some(Answer::Ran),
        "{:#?}",
        second.answers.borrow()
    );
    assert_ne!(
        at_head(&second.f.repo, "crates/a/src/lib.rs"),
        landed,
        "a real second attempt landed"
    );
    // And the verdict judges that new fix: it is asked again.
    assert_eq!(
        answer(&second, "verification-wave-review-verify-task-a-1-2"),
        Some(Answer::Ran)
    );
}

/// Finding A recorded, finding B asked under the same call id: never the
/// answer to A.
#[tokio::test]
async fn a_fix_for_one_failure_never_answers_another_under_the_same_id() {
    let f = first_session("scratch path collision (environment)").await;
    let second = session(
        f,
        vec![failing("AssertionError: unregistered dataset ref x")],
    );
    run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    assert_eq!(answer(&second, FIX), Some(Answer::Ran));
    let prompts = second.prompts.borrow();
    assert!(
        prompts
            .iter()
            .any(|(id, prompt)| id == FIX && prompt.contains("unregistered dataset ref x")),
        "the new failure is what the coder was asked"
    );
}

/// The contract names the observation, and nothing else about the fix call
/// moved: a review remediation (no acceptance round behind it) still
/// replays on a resume -- see `write_wave_remediation_landing` for the
/// whole review path.
#[tokio::test]
async fn the_acceptance_fix_names_the_round_it_was_observed_in() {
    let first = session(fixture(), vec![failing("boom")]);
    run(SCRIPT, NEW_PRELUDE, first.clone()).await;
    let calls = first.calls.borrow();
    let fix = calls
        .iter()
        .find(|call| call.id == FIX)
        .expect("fix call listed");
    assert_eq!(
        fix.options.extra["remediationContract"]["observedBy"],
        json!(["acceptance-contract-run-1"])
    );
}

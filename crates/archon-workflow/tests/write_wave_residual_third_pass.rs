//! Issue-121 end to end: a second-pass residual round whose verifier
//! REFUSES while it records a HIGH regression in another task's file -- the
//! live wf-0ddadd81 shape -- blocks the final gate, and the bounded third
//! residual pass (`residual-gaps-3`) plans one round of that file's owner
//! through the real prelude, the production write wave, the host's dispatch
//! check and the final gate, and the run goes green; under the deployed
//! prelude (no third pass) it stays blocked; and a resume from the deployed
//! prelude replays every existing call, so only the third pass's round runs.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;
#[path = "support/residual_world.rs"]
mod world;

use std::rc::Rc;

use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::json;
use support::git;
use world::*;

/// The prelude the live binary runs (0bfb2e0ae): two residual passes.
const DEPLOYED: &str = include_str!("fixtures/v3_primitives_0bfb2e0ae.js");

const TESTS: &str = "crates/shared/src/store_tests.rs";
const STALE: &str = "store::tests::stale";
const LIB: &str = "cargo test -p shared --lib";
const MEDIUM_STORE_GAP: (&str, &str, &str) = (
    "gap-store-version",
    "medium",
    "crates/shared/src/store.rs:12 writes no version beside the lane's",
);
const REGRESSION: (&str, &str, &str) = (
    "gap-b-regression",
    "high",
    "crates/b/src/lib.rs:1 no longer derives the lane's version, so TASK-B's declared tests are red",
);

/// The residual world, its store file now a package with a test module.
fn package_host() -> Rc<Host> {
    let f = fixture();
    std::fs::write(
        f.repo.join("crates/shared/Cargo.toml"),
        "[package]\nname = \"shared\"\n",
    )
    .unwrap();
    std::fs::write(f.repo.join(TESTS), "// stale fixture\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "the store's tests"]);
    session(f, 0)
}

/// A session over `f` whose TASK-A rounds so far number `done`: its first
/// residual round fixes the store, its second (the second pass's) the tests.
fn session(f: support::Fixture, done: usize) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let rounds = std::cell::Cell::new(done);
    Rc::new(Host::new(
        f,
        store,
        Box::new(move |key: &str, round: u64, escalated: bool| {
            if key != "TASK-A" {
                return writes(key, round, escalated);
            }
            rounds.set(rounds.get() + 1);
            if rounds.get() == 1 {
                edits(vec![(STORE, "// store: versioned\n")])
            } else {
                edits(vec![(TESTS, "// tests: fixed\n")])
            }
        }),
    ))
}

fn verdicts(host: &Host) {
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![MEDIUM_STORE_GAP])]);
    host.verdicts(
        "TASK-A",
        vec![
            Verdict::RefuseRed(vec![STALE], LIB),
            Verdict::RefuseWith(vec![REGRESSION]),
        ],
    );
    host.verdicts("TASK-B", vec![Verdict::Accept]);
}

#[tokio::test]
async fn a_refused_second_pass_verifiers_regression_gets_a_third_pass_round_of_its_owner() {
    let host = package_host();
    verdicts(&host);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        6,
        "three rounds of fix and verifier: {:#?}",
        answers(&host)
    );
    assert!(
        answers(&host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "{:#?}",
        answers(&host)
    );
    let fix = host.store.load_call_record(&calls[4]).unwrap().unwrap();
    let contract = &fix.call.options.extra["remediationContract"];
    assert_eq!(contract["residual"]["pass"], json!(3), "{contract}");
    assert_eq!(contract["residual"]["files"], json!([]));
    assert_eq!(contract["taskId"], json!("TASK-B"));
    assert!(
        contract["residual"]["key"]
            .as_str()
            .unwrap()
            .ends_with("p3")
    );
    // Implementable inside its grant: the owner's round, its own file.
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-B"]);
    assert_eq!(at_head(&host.f.repo, B), "// b: refreshed");
    let prompts = host.prompts.borrow();
    let (_, prompt) = prompts.iter().find(|(id, _)| *id == calls[4]).unwrap();
    assert!(prompt.contains("gap-b-regression"), "{prompt}");
    drop(prompts);
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(why.contains("resolved `gap-b-regression`"), "{why}");
}

#[tokio::test]
async fn without_the_third_pass_the_refused_verifiers_regression_blocks() {
    let host = package_host();
    verdicts(&host);
    let result = run(&script(), DEPLOYED, host.clone()).await;
    assert_eq!(residual_calls(&host).len(), 4, "{:#?}", answers(&host));
    assert_eq!(at_head(&host.f.repo, B), "// b");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("gap-b-regression") && why.contains("after the second residual pass"),
        "{why}"
    );
}

#[tokio::test]
async fn a_resume_from_the_deployed_prelude_replays_every_call_and_runs_only_the_third_pass() {
    let first = package_host();
    verdicts(&first);
    run(&script(), DEPLOYED, first.clone()).await;
    let recorded = answers(&first);
    let Ok(first) = Rc::try_unwrap(first) else {
        panic!("the session is still referenced")
    };
    let second = session(first.f, 2);
    second.verdicts("TASK-B", vec![Verdict::Accept]);
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    let answered = answers(&second);
    for (id, answer) in &answered {
        if recorded.iter().any(|(seen, _)| seen == id) {
            assert_eq!(*answer, Answer::Replayed, "{id}: {answered:#?}");
        }
    }
    let new: Vec<&String> = answered
        .iter()
        .filter(|(_, answer)| *answer != Answer::Replayed)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        new.len(),
        2,
        "only the third pass's fix and verifier: {answered:#?}"
    );
    assert!(
        new.iter().all(|id| id.contains("residual-")),
        "{answered:#?}"
    );
    assert_eq!(at_head(&second.f.repo, B), "// b: refreshed");
    let (status, why) = terminal(&second, &after);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

//! Issue-118 end to end: a residual round carrying a HIGH gap, whose verdict
//! the host refused over red tests the round could not write, is planned
//! AGAIN by the bounded second residual pass (`residual-gaps-2`) -- its own
//! gap plus those tests, granted the tests' file -- through the real
//! prelude, the production write wave, the host's dispatch check and the
//! final gate, and the run goes green; under the deployed prelude it stays
//! blocked; and a resume from the deployed prelude replays every existing
//! call, so only the retry runs.
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

/// The prelude the live binary runs (7c9f11a6c): one residual pass only.
const DEPLOYED: &str = include_str!("fixtures/v3_primitives_7c9f11a6c.js");

const TESTS: &str = "crates/shared/src/store_tests.rs";
const STALE: &str = "store::tests::stale";
const LIB: &str = "cargo test -p shared --lib";
const HIGH_STORE_GAP: (&str, &str, &str) = (
    "gap-store-version",
    "high",
    "crates/shared/src/store.rs:12 writes no version beside the lane's",
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
/// residual round fixes the store; its second (the second pass's) fixes the
/// tests the first could not write.
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
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_STORE_GAP])]);
    host.verdicts(
        "TASK-A",
        vec![Verdict::RefuseRed(vec![STALE], LIB), Verdict::Accept],
    );
}

/// The resumed session asks only the second pass's verifier.
fn verdicts_resumed(host: &Host) {
    host.verdicts("TASK-A", vec![Verdict::Accept]);
}

#[tokio::test]
async fn a_round_refused_over_tests_it_cannot_write_gets_a_second_pass_round_that_fixes_them() {
    let host = package_host();
    verdicts(&host);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        4,
        "two rounds of fix and verifier: {:#?}",
        answers(&host)
    );
    assert!(
        answers(&host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "{:#?}",
        answers(&host)
    );
    let fix = host.store.load_call_record(&calls[2]).unwrap().unwrap();
    let contract = &fix.call.options.extra["remediationContract"];
    assert_eq!(contract["residual"]["pass"], json!(2), "{contract}");
    assert_eq!(contract["residual"]["files"], json!([STORE, TESTS]));
    assert_eq!(contract["taskId"], json!("TASK-A"));
    assert!(
        contract["residual"]["key"]
            .as_str()
            .unwrap()
            .ends_with("p2")
    );
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    assert_eq!(at_head(&host.f.repo, TESTS), "// tests: fixed");
    let prompts = host.prompts.borrow();
    let (_, prompt) = prompts.iter().find(|(id, _)| *id == calls[2]).unwrap();
    assert!(prompt.contains(STALE), "the red test, by name: {prompt}");
    assert!(
        prompt.contains("gap-store-version"),
        "the round's own gap, again: {prompt}"
    );
    drop(prompts);
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(
        why.contains("resolved `gap-store-version`"),
        "the refused round's HIGH gap is resolved by its retry: {why}"
    );
}

#[tokio::test]
async fn without_the_second_pass_the_red_tests_have_no_round_and_the_high_gap_blocks() {
    // The deployed prelude on the same world: nothing is planned for the
    // tests the round could not write.
    let host = package_host();
    verdicts(&host);
    let result = run(&script(), DEPLOYED, host.clone()).await;
    assert_eq!(residual_calls(&host).len(), 2, "{:#?}", answers(&host));
    assert_eq!(at_head(&host.f.repo, TESTS), "// stale fixture");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(why.contains("gap-store-version"), "{why}");
}

#[tokio::test]
async fn a_resume_from_the_deployed_prelude_replays_every_call_and_runs_only_the_second_pass() {
    let first = package_host();
    verdicts(&first);
    run(&script(), DEPLOYED, first.clone()).await;
    let recorded = answers(&first);
    assert_eq!(
        recorded.len(),
        4,
        "review round and first-pass round: {recorded:#?}"
    );
    let Ok(first) = Rc::try_unwrap(first) else {
        panic!("the session is still referenced")
    };
    let second = session(first.f, 1);
    verdicts_resumed(&second);
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    let answered = answers(&second);
    let new: Vec<&String> = answered
        .iter()
        .filter(|(_, answer)| *answer != Answer::Replayed)
        .map(|(id, _)| id)
        .collect();
    // The refused first-pass round was attempted: it is never asked again,
    // and every answered earlier call replays.
    for (id, answer) in &answered {
        if recorded.iter().any(|(seen, _)| seen == id) {
            assert_eq!(*answer, Answer::Replayed, "{id}: {answered:#?}");
        }
    }
    assert_eq!(
        new.len(),
        2,
        "only the second pass's fix and verifier: {answered:#?}"
    );
    assert!(
        new.iter().all(|id| id.contains("residual-")),
        "{answered:#?}"
    );
    assert_eq!(at_head(&second.f.repo, TESTS), "// tests: fixed");
    let (status, why) = terminal(&second, &after);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

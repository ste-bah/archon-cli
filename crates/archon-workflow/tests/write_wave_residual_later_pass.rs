//! Batch O2 (REM-11/CUT-5) end to end: residual passes follow progress. A
//! round carrying two gaps is left open by its judge in passes 1 and 2; in
//! pass 3 its judge resolves one of the two -- progress -- so the host plans
//! a FOURTH pass retrying it, through the real prelude, the production write
//! wave and the host's dispatch check, and that pass resolves the rest: the
//! run goes green. When the third pass makes no progress, no round is
//! planned after it, the prelude stops, and what stands blocks. A resume
//! replays every call, the fourth pass's included. Batch O2 review (M1, M3):
//! a verifier that rewords its gap every pass cannot keep the passes going
//! -- they stop and the gap blocks, quoted -- and a recording that already
//! moved past its residual passes (an acceptance round under a prelude that
//! asked three) plans nothing in the fourth slot a resume asks.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;
#[path = "support/residual_world.rs"]
mod world;

use std::rc::Rc;

use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, run};
use world::*;

const G1: (&str, &str, &str) = (
    "gap-store-version",
    "high",
    "crates/shared/src/store.rs:12 writes no version beside the lane's",
);
const G2: (&str, &str, &str) = (
    "gap-store-instrument",
    "high",
    "crates/shared/src/store.rs:30 recovers the instrument from the wrong segment",
);

/// A session whose every TASK-A round lands a change of its own.
fn session(f: support::Fixture) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let rounds = std::cell::Cell::new(0usize);
    Rc::new(Host::new(
        f,
        store,
        Box::new(move |key: &str, round: u64, escalated: bool| {
            if key != "TASK-A" {
                return writes(key, round, escalated);
            }
            rounds.set(rounds.get() + 1);
            let text: &'static str =
                Box::leak(format!("// store: attempt {}\n", rounds.get()).into_boxed_str());
            edits(vec![(STORE, text)])
        }),
    ))
}

/// The store round's judges, pass by pass: both open, both open, then
/// `third`, then everything resolved.
fn verdicts(host: &Host, third: Verdict) {
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![G1, G2])]);
    host.verdicts(
        "TASK-A",
        vec![
            Verdict::RefuseWith(vec![]),
            Verdict::RefuseWith(vec![]),
            third,
            Verdict::AcceptDisposing(vec![], vec![(G1.0, "resolved"), (G2.0, "resolved")]),
        ],
    );
}

fn passes(host: &Host) -> Vec<u64> {
    let mut passes: Vec<u64> = residual_calls(host)
        .iter()
        .filter_map(|id| host.store.load_call_record(id).unwrap())
        .map(|record| {
            record.call.options.extra["remediationContract"]["residual"]["pass"]
                .as_u64()
                .unwrap_or(1)
        })
        .collect();
    passes.dedup();
    passes
}

#[tokio::test]
async fn a_third_pass_that_made_progress_buys_a_fourth_which_resolves_the_rest() {
    let host = session(fixture());
    verdicts(
        &host,
        Verdict::AcceptDisposing(vec![], vec![(G1.0, "resolved"), (G2.0, "open")]),
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert!(
        answers(&host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "{:#?}",
        answers(&host)
    );
    assert_eq!(passes(&host), [1, 2, 3, 4], "{:#?}", answers(&host));
    let calls = residual_calls(&host);
    let fourth = host.store.load_call_record(&calls[6]).unwrap().unwrap();
    let key = fourth.call.options.extra["remediationContract"]["residual"]["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(key.ends_with("p4"), "{key}");
    // The slot after the fourth was asked and planned nothing: the end.
    // (Each slot is listed once however often the prelude asks it.)
    let mut slots: Vec<String> = Vec::new();
    for call in host.calls.borrow().iter() {
        if call.id.starts_with("residual-gaps-") && !slots.contains(&call.id) {
            slots.push(call.id.clone());
        }
    }
    assert_eq!(
        slots,
        [
            "residual-gaps-1",
            "residual-gaps-2",
            "residual-gaps-3",
            "residual-gaps-4",
            "residual-gaps-5"
        ]
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(why.contains(&key), "{why}");

    // A resume replays every call, the fourth pass's included.
    let Ok(first) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    let second = session(first.f);
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    let fresh: Vec<(String, Answer)> = answers(&second)
        .into_iter()
        .filter(|(_, answer)| *answer == Answer::Ran)
        .filter(|(id, _)| !harness::asked_again_by_batch_o(id))
        .collect();
    assert!(fresh.is_empty(), "{fresh:#?}");
    let (status, why) = terminal(&second, &after);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_third_pass_without_progress_ends_the_passes_and_what_stands_blocks() {
    let host = session(fixture());
    // The third judge leaves both gaps open, as the second did.
    verdicts(&host, Verdict::RefuseWith(vec![]));
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert_eq!(passes(&host), [1, 2, 3], "{:#?}", answers(&host));
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(why.contains("gap-store-instrument"), "{why}");
    // M1: the fourth slot planned no round -- the open HIGH gaps did not
    // move -- and says so, quoting each gap that stands.
    assert!(
        why.contains("residual passes stopped before pass 4") && why.contains(G2.2),
        "{why}"
    );
}

/// M1: the store round's judge refuses every pass, recording the same gap
/// in new words each time.
#[tokio::test]
async fn a_gap_reworded_every_pass_stops_the_passes_and_blocks_quoted() {
    let host = session(fixture());
    let reworded = |n: usize| -> (&'static str, &'static str, &'static str) {
        let text = format!("crates/shared/src/store.rs:12 drifts from the lane (wording {n})");
        ("gap-store-drift", "high", Box::leak(text.into_boxed_str()))
    };
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![reworded(0)])]);
    host.verdicts(
        "TASK-A",
        (1..=12)
            .map(|n| Verdict::RefuseWith(vec![reworded(n)]))
            .collect(),
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let reached = passes(&host);
    assert!(
        reached.iter().all(|pass| *pass <= 3),
        "no pass after the third: {reached:?} {:#?}",
        answers(&host)
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("residual passes stopped before pass 4") && why.contains("(wording "),
        "{why}"
    );
}

/// M3: a recording that ran its acceptance stage after three passes -- as a
/// prelude that asked no fourth recorded it -- plans nothing in the fourth
/// slot a resume under this prelude asks, though its third pass made
/// progress. The recording is the progress run with what a fourth pass
/// recorded taken back out.
#[tokio::test]
async fn a_recording_that_moved_past_its_passes_plans_no_fourth_on_resume() {
    let first = session(fixture());
    verdicts(
        &first,
        Verdict::AcceptDisposing(vec![], vec![(G1.0, "resolved"), (G2.0, "open")]),
    );
    run(&script(), NEW_PRELUDE, first.clone()).await;
    assert_eq!(passes(&first), [1, 2, 3, 4], "{:#?}", answers(&first));
    // What a three-pass prelude never recorded: the later slots and rounds.
    for record in first.store.load_call_records().unwrap() {
        let later_slot = record.call.id.starts_with("residual-gaps-")
            && !["residual-gaps-1", "residual-gaps-2", "residual-gaps-3"]
                .contains(&record.call.id.as_str());
        let later_round = record
            .call
            .options
            .extra
            .get("remediationContract")
            .and_then(|contract| contract.pointer("/residual/pass"))
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|pass| pass >= 4);
        // A later round's done mark (`<key>-done`, its key ending `pN`).
        let later_done = record
            .call
            .id
            .strip_suffix("-done")
            .and_then(|key| key.rsplit_once('p'))
            .and_then(|(_, pass)| pass.parse::<u64>().ok())
            .is_some_and(|pass| pass >= 4);
        if later_slot || later_round || later_done {
            std::fs::remove_file(first.store.result_path(&record.call.id)).unwrap();
        }
    }
    let Ok(first) = Rc::try_unwrap(first) else {
        panic!("the session is still referenced")
    };
    let second = session(first.f);
    // Were a fourth pass planned, its round would run and be resolved.
    second.verdicts(
        "TASK-A",
        vec![Verdict::AcceptDisposing(
            vec![],
            vec![(G1.0, "resolved"), (G2.0, "resolved")],
        )],
    );
    run(&script(), NEW_PRELUDE, second.clone()).await;
    let reached = passes(&second);
    assert!(!reached.contains(&4), "{reached:?} {:#?}", answers(&second));
}

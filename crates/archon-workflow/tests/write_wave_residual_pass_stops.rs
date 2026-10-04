//! Residual stalls pause resumably; progressing passes have no total limit.
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

type Gap = (&'static str, &'static str, &'static str);

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// The `n`th gap of a churning verifier: a new id, in new words, on the
/// store lane TASK-A's rounds may write.
fn churn(n: usize) -> Gap {
    (
        leak(format!("gap-churn-{n}")),
        "high",
        leak(format!(
            "churn {n}: crates/shared/src/store.rs:{n} writes lane {n} out of order"
        )),
    )
}

/// A churning verifier's verdict in pass `n`: the gap it was asked about
/// resolved, and a new one recorded.
fn churning(n: usize) -> Verdict {
    Verdict::AcceptDisposing(vec![churn(n)], vec![(churn(n - 1).0, "resolved")])
}

const GAP_A: Gap = (
    "gap-cycle-a",
    "high",
    "crates/shared/src/store.rs:12 writes no version beside the lane's",
);
const GAP_B: Gap = (
    "gap-cycle-b",
    "high",
    "crates/shared/src/store.rs:30 recovers the instrument from the wrong segment",
);
const GAP_C: Gap = (
    "gap-cycle-c",
    "high",
    "crates/shared/src/store.rs:44 drops the lane's trailing record on flush",
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
            let text = leak(format!("// store: attempt {}\n", rounds.get()));
            edits(vec![(STORE, text)])
        }),
    ))
}

/// The residual pass of each residual round the host dispatched, in order.
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

/// The run as the live host records and decides it, after one session.
async fn decided(host: &Rc<Host>) -> (WorkflowV2Status, String) {
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert!(
        answers(host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "{:#?}",
        answers(host)
    );
    terminal(host, &result)
}

#[tokio::test]
async fn a_verifier_recording_new_states_runs_past_the_legacy_default_ceiling() {
    let host = session(fixture());
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![churn(0)])]);
    host.verdicts("TASK-A", (1..=12).map(churning).collect());
    let (status, why) = decided(&host).await;
    assert_eq!(
        passes(&host),
        (1..=13).collect::<Vec<_>>(),
        "{:#?}",
        answers(&host)
    );
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

/// Open gaps {a, c} after pass 1 (a carried and refused, c recorded by the
/// refusing verifier), {b} after pass 2 (a resolved, b recorded), then
/// {a, c} again after pass 3 (b resolved and a recorded again; c's own
/// round, a first attempt, left open): pass 4 would retry c's round, but
/// the open set repeats pass 1's.
#[tokio::test]
async fn a_verifier_whose_open_gaps_return_to_an_earlier_set_stops_at_the_repeat() {
    let host = session(fixture());
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![GAP_A])]);
    host.verdicts(
        "TASK-A",
        vec![
            Verdict::RefuseWith(vec![GAP_C]),
            Verdict::AcceptDisposing(vec![GAP_B], vec![(GAP_A.0, "resolved")]),
            // Pass 3 plans b's round, then c's.
            Verdict::AcceptDisposing(vec![GAP_A], vec![(GAP_B.0, "resolved")]),
            Verdict::RefuseWith(vec![]),
        ],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert_eq!(result["paused"], serde_json::json!(true));
    assert_eq!(
        host.f.store.load_state(&host.f.run).unwrap().status,
        RunStatus::Paused
    );
    assert_eq!(passes(&host), [1, 2, 3], "{:#?}", answers(&host));
    let why = std::fs::read_to_string(host.f.store.events_path(&host.f.run)).unwrap();
    assert!(
        why.contains("cycle") && why.contains("pass 3 left open the same gaps as pass 1"),
        "{why}"
    );
    for gap in [GAP_A, GAP_C] {
        assert!(why.contains(gap.0) && why.contains(gap.2), "{why}");
    }
}

/// A verifier that records a new gap for four passes and then none: five
/// passes, exactly the recorded ceiling, and the run is accepted.
#[tokio::test]
async fn a_converging_verifier_is_accepted_at_exactly_the_ceiling() {
    let host = session(fixture());
    host.store.record_max_residual_passes(5).unwrap();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![churn(0)])]);
    let mut judged: Vec<Verdict> = (1..=4).map(churning).collect();
    judged.push(Verdict::AcceptDisposing(
        vec![],
        vec![(churn(4).0, "resolved")],
    ));
    host.verdicts("TASK-A", judged);
    let (status, why) = decided(&host).await;
    assert_eq!(passes(&host), [1, 2, 3, 4, 5], "{:#?}", answers(&host));
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(!why.contains("residual passes stopped"), "{why}");
}

#[tokio::test]
async fn a_legacy_ceiling_of_one_cannot_stop_progressing_passes() {
    let host = session(fixture());
    host.store.record_max_residual_passes(1).unwrap();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![churn(0)])]);
    host.verdicts("TASK-A", (1..=4).map(churning).collect());
    let (status, why) = decided(&host).await;
    assert_eq!(passes(&host), [1, 2, 3, 4, 5], "{:#?}", answers(&host));
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

/// No verifier records a gap: no residual round, nothing to stop, and the
/// run is accepted even under the smallest ceiling.
#[tokio::test]
async fn an_empty_gap_set_is_never_stopped() {
    let host = session(fixture());
    host.store.record_max_residual_passes(1).unwrap();
    host.verdicts(CROSS, vec![Verdict::Accept]);
    let (status, why) = decided(&host).await;
    assert!(passes(&host).is_empty(), "{:#?}", answers(&host));
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(!why.contains("residual passes stopped"), "{why}");
}

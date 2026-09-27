//! A host-planned residual round whose fix accepts having changed nothing
//! (its gap already gone) is judged by one verifier on the tree as it is,
//! and the final gate resolves it only on that verifier's explicit
//! "resolved" for every gap it targets. Under the 3a9cd5aec prelude no
//! verifier ran and the gap stood with no way to resolve it.
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

const AT_3A9CD5AEC: &str = include_str!("fixtures/v3_primitives_3a9cd5aec.js");

/// The residual world whose TASK-A round writes nothing: the store gap is
/// already fixed on the tree.
fn noop_host() -> Rc<Host> {
    noop_host_on(fixture(), false)
}

/// ... with TASK-A declaring a required tool its gap never involves, and the
/// no-op judged by the real adapter.
fn noop_host_on(f: support::Fixture, via_adapter: bool) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(
        f,
        store,
        Box::new(move |key: &str, round: u64, escalated: bool| {
            if key == "TASK-A" {
                let mut none = edits(vec![]);
                none.via_adapter = via_adapter;
                none
            } else {
                writes(key, round, escalated)
            }
        }),
    ))
}

#[tokio::test]
async fn a_no_op_round_its_verifier_confirms_resolves() {
    let host = noop_host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    host.verdicts(
        "TASK-A",
        vec![Verdict::AcceptDisposing(
            vec![],
            vec![(HIGH_GAP.0, "resolved")],
        )],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        2,
        "a fix and its verifier: {:#?}",
        answers(&host)
    );
    assert!(calls[1].starts_with("verification-wave-"), "{calls:?}");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_no_op_round_its_verifier_accepts_without_resolving_each_gap_stands() {
    let host = noop_host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    host.verdicts("TASK-A", vec![Verdict::Accept]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("landed nothing") && why.contains(HIGH_GAP.0),
        "{why}"
    );
}

#[tokio::test]
async fn under_the_3a9cd5aec_prelude_a_no_op_round_is_never_verified() {
    let host = noop_host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    host.verdicts(
        "TASK-A",
        vec![Verdict::AcceptDisposing(
            vec![],
            vec![(HIGH_GAP.0, "resolved")],
        )],
    );
    let result = run(&script(), AT_3A9CD5AEC, host.clone()).await;
    assert!(
        !answers(&host).iter().any(|(id, a)| id.contains("residual")
            && id.starts_with("verification-wave-")
            && *a == Answer::Ran),
        "{:#?}",
        answers(&host)
    );
    let (status, _) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview);
}

/// Batch B item 6: a no-op residual round is not made to exercise a required
/// tool of its task that neither its gap nor any file it touches involves
/// (live: a residual round refused twice for a compile tool). Under the
/// adapter's old rule the no-op was refused and nothing ever verified it.
#[tokio::test]
async fn a_no_op_round_owes_no_required_tool_its_gap_never_involves() {
    let mut f = fixture();
    f.universe.as_mut().unwrap().tasks[0].required_tools = vec!["mcp__feed__quote".into()];
    let host = noop_host_on(f, true);
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    host.verdicts(
        "TASK-A",
        vec![Verdict::AcceptDisposing(
            vec![],
            vec![(HIGH_GAP.0, "resolved")],
        )],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(
        fix.status,
        WorkflowV2Status::Noop,
        "{:?}",
        fix.result.summary
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

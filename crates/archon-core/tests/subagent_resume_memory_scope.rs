//! A resume runs under its stored context and, on top of it, the caller's
//! supervision; state that a stored object can change after spawn refuses it.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use archon_tools::tool::ToolContext;
use harness::*;
use memory_harness::*;

#[tokio::test]
async fn the_resume_callers_cancellation_still_stops_the_resumed_agent() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let store = store(&root);
    let host = Host::new(&root, "memory-cancel", vec![STOP, STOP]);
    host.spawn("child", request(&workspace, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    history(&store, "child");
    let plan = host.plan(&store, "child").await.unwrap();
    let interrupted = tokio_util::sync::CancellationToken::new();
    interrupted.cancel();
    let result = host
        .resume(
            "child",
            plan,
            ToolContext {
                cancel_parent: Some(interrupted),
                ..parent(&root, &[])
            },
        )
        .await;
    assert!(
        result.is_err(),
        "the caller's interrupt did not reach the resumed agent: {result:?}"
    );
}

#[tokio::test]
async fn a_second_resume_of_the_same_agent_is_refused_while_one_is_pending() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let store = store(&root);
    let host = Host::new(&root, "memory-race", vec![STOP]);
    host.spawn("child", request(&workspace, None, vec![]), parent(&root, &[]))
        .await
        .unwrap();
    history(&store, "child");
    let (_, first) = host.plan(&store, "child").await.unwrap().into_pending();
    let (_, second) = host.plan(&store, "child").await.unwrap().into_pending();
    let held = archon_core::agents::transcript::reserve_resume(&host.pending, first)
        .await
        .expect("the first resume reserves the slot");
    let refusal = archon_core::agents::transcript::reserve_resume(&host.pending, second)
        .await
        .err()
        .expect("a second resume replaced the first one's slot");
    assert!(refusal.contains("'child'"), "{refusal}");
    drop(held);
    assert!(
        host.pending.lock().await.is_empty(),
        "an unconsumed reservation outlived its resume"
    );
}

// H2: a site restored after a check's fault keeps the warm build output when
// no other process holds the site or its build output. Its own target, so
// no other test of the same process forks while the observation probes for
// holders (a child between fork and exec briefly holds every descriptor of
// its parent, which reads as a holder and forgets the builds: cold, never
// stale).
#![cfg(unix)]

#[path = "support/isolation_fixture.rs"]
mod support;
use support::*;

/// H2: a check's fault that leaves nothing running keeps the warm build
/// output: the observing process's own handle on its site is no holder, so
/// the check after it builds warm, in the same cache generation.
#[tokio::test]
async fn a_fault_that_leaves_nothing_running_keeps_the_warm_build_output() {
    let (t, result) = observe_with(
        &[
            (
                "printf warm > \"$CARGO_TARGET_DIR/warm\" && test -f data/value",
                "project_root",
            ),
            ("rm -f target && test -f data/value", "project_root"),
            ("test -f \"$CARGO_TARGET_DIR/warm\"", "project_root"),
        ],
        |root, policy| policy.build_cache = Some(root.join("cache")),
    )
    .await;
    assert_siblings_judged(&result, 1);
    assert_eq!(generation(t.path()), 0, "no build was forgotten");
}

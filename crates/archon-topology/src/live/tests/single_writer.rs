//! Invariant 2 — single writer per artifact.

use super::super::*;
use super::*;

#[test]
fn invariant_2_blocks_an_unrelated_live_node_writing_a_claimed_path() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );

    let verdict = live.on_write_intent(SESSION, &write("b", "src/lib.rs"));

    assert_eq!(verdict.invariant(), Some(Invariant::SingleWriter));
    let reason = verdict.reason().expect("blocked carries a reason");
    assert!(reason.contains('a'), "names the conflicting node: {reason}");
    assert!(reason.contains("src/lib.rs"), "names the path: {reason}");
    assert!(
        reason.contains("single_writer"),
        "names the invariant: {reason}"
    );
}

#[test]
fn invariant_2_admits_a_different_path() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &write("b", "src/main.rs"))
            .is_allowed()
    );
}

#[test]
fn invariant_2_admits_a_node_rewriting_its_own_claim() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );
}

#[test]
fn invariant_2_admits_once_the_claim_holder_has_finished() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );

    live.on_node_finished(SESSION, "a");

    assert!(
        live.on_write_intent(SESSION, &write("b", "src/lib.rs"))
            .is_allowed()
    );
}

#[test]
fn invariant_2_admits_two_nodes_joined_by_a_dependency_path() {
    // The near-miss that matters most: the paths overlap exactly, but a
    // dependency orders the two nodes so they cannot race.
    let live = tracker(LiveTopologyConfig::default());
    live.declare_graph(
        SESSION,
        &graph(
            vec![
                TaskNode::new("a", NodeRole::Work),
                TaskNode {
                    depends_on: vec!["a".into()],
                    ..TaskNode::new("b", NodeRole::Work)
                },
            ],
            GraphBudget::default(),
        ),
    );
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");

    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &write("b", "src/lib.rs"))
            .is_allowed()
    );
}

#[test]
fn invariant_2_admits_a_child_writing_what_its_spawning_parent_claims() {
    // The undeclared-session analogue of the test above: the only dependency
    // structure a plain turn has is the spawn chain.
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "root");
    assert!(
        live.on_write_intent(SESSION, &write("root", "src/lib.rs"))
            .is_allowed()
    );

    assert!(
        live.on_spawn(
            SESSION,
            &SpawnIntent {
                node_id: "child".into(),
                parent_id: Some("root".into()),
                agent: "worker".into(),
            }
        )
        .is_allowed()
    );

    assert!(
        live.on_write_intent(SESSION, &write("child", "src/lib.rs"))
            .is_allowed()
    );
}

#[test]
fn invariant_2_uses_the_coordinators_glob_overlap_not_string_equality() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/*.rs"))
            .is_allowed()
    );

    // `src/lib.rs` is not string-equal to `src/*.rs`; the resource-key overlap
    // table says they conflict. `TaskGraph::write_conflicts`, which is exact
    // string, would miss this.
    assert!(
        live.on_write_intent(SESSION, &write("b", "src/lib.rs"))
            .is_blocked()
    );
}

#[test]
fn invariant_2_treats_a_malformed_glob_as_conflicting() {
    // The write coordinator's deliberate fail-safe, preserved. Not in tension
    // with "never fail closed on a bookkeeping bug": this is a present and
    // unreadable claim, not a missing one.
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/[unclosed"))
            .is_allowed()
    );

    assert!(
        live.on_write_intent(SESSION, &write("b", "totally/elsewhere.rs"))
            .is_blocked()
    );
}

#[test]
fn invariant_2_declines_to_conclude_when_no_path_is_declared() {
    // Empty writes mean *unknown*, not *nothing*.
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );

    let unknown = WriteIntent {
        node_id: "b".into(),
        paths: vec![String::new(), "   ".into()],
        shared_append: Vec::new(),
    };
    assert!(live.on_write_intent(SESSION, &unknown).is_allowed());
}

// ---------------------------------------------------------- shared appends

fn append(node: &str, path: &str) -> WriteIntent {
    WriteIntent::exclusive(node, Vec::new()).with_shared_append(vec![path.to_string()])
}

/// Admission must reach the same three answers the write coordinator plans by,
/// because it is the same overlap table. Two nodes both asserting a coordinated
/// append to one path do not race.
#[test]
fn invariant_2_admits_two_unrelated_nodes_appending_to_one_declared_path() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &append("a", ".archon/data/registry.json"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &append("b", ".archon/data/registry.json"))
            .is_allowed()
    );
}

/// One side asserting coordination is not agreement, in either order.
#[test]
fn invariant_2_blocks_an_exclusive_write_against_a_declared_append() {
    for (first, second) in [("append", "exclusive"), ("exclusive", "append")] {
        let live = tracker(LiveTopologyConfig::default());
        live.on_node_started(SESSION, "a");
        live.on_node_started(SESSION, "b");
        let intent = |node: &str, kind: &str| match kind {
            "append" => append(node, ".archon/data/registry.json"),
            _ => write(node, ".archon/data/registry.json"),
        };
        assert!(
            live.on_write_intent(SESSION, &intent("a", first))
                .is_allowed()
        );
        let verdict = live.on_write_intent(SESSION, &intent("b", second));
        assert_eq!(
            verdict.invariant(),
            Some(Invariant::SingleWriter),
            "{first} then {second} must block"
        );
        let reason = verdict.reason().expect("blocked carries a reason");
        assert!(
            reason.contains("both sides must declare the path as a shared append"),
            "the reason must name the way out: {reason}"
        );
    }
}

/// The fail-safe. A shared-append claim naming a pattern rather than a file
/// cannot be checked, so it conflicts — it must not pass by being downgraded.
#[test]
fn invariant_2_treats_an_unresolvable_shared_append_as_conflicting() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &append("b", ".archon/data/*.json"))
            .is_blocked(),
        "a pattern is not a coordinated claim on a file"
    );
}

/// Nothing becomes concurrent by omission: an ordinary write to the same path
/// still blocks, exactly as before this key existed.
#[test]
fn invariant_2_still_blocks_when_neither_side_declares_an_append() {
    let live = tracker(LiveTopologyConfig::default());
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", ".archon/data/registry.json"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &write("b", ".archon/data/registry.json"))
            .is_blocked()
    );
}

#[test]
fn invariant_2_can_be_disabled_alone() {
    let live = tracker(LiveTopologyConfig {
        single_writer: false,
        ..LiveTopologyConfig::default()
    });
    live.on_node_started(SESSION, "a");
    live.on_node_started(SESSION, "b");
    assert!(
        live.on_write_intent(SESSION, &write("a", "src/lib.rs"))
            .is_allowed()
    );
    assert!(
        live.on_write_intent(SESSION, &write("b", "src/lib.rs"))
            .is_allowed()
    );
}

//! Write-claim tests (#184 M2).
//!
//! Claims are keyed by agent id in a process-global map, so every test uses
//! ids unique to itself and releases what it took.
//!
//! Each test registers its agents in `BACKGROUND_AGENTS` for real. A claim only
//! counts while its holder is registered as running, so inventing an id would
//! make every claim read as dead and no overlap could ever be detected — the
//! tests would pass while proving nothing.

use super::*;

use std::sync::Arc;
use std::time::SystemTime;

use crate::background_agents::{self, AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle};

fn declared(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

/// Register a genuinely running agent and return its id.
///
/// Liveness is read from `BACKGROUND_AGENTS`, so a claim only counts while its
/// holder is actually registered as running — a test that invented an id would
/// prove nothing, because every claim would read as dead and no overlap could
/// ever be detected.
fn live_agent(tag: &str) -> String {
    let subagent_id = format!("write-claim-test-{tag}");
    let handle = BackgroundAgentHandle {
        agent_id: uuid::Uuid::new_v4(),
        subagent_id: subagent_id.clone(),
        join_handle: None,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        spawned_at: SystemTime::now(),
        status: Arc::new(std::sync::Mutex::new(AgentStatus::Running)),
        result_slot: background_agents::new_result_slot(),
    };
    let _ = BACKGROUND_AGENTS.register(handle);
    subagent_id
}

/// An id that was never registered — indistinguishable from a finished agent,
/// which is the state that must not block anyone.
fn dead(tag: &str) -> String {
    format!("write-claim-finished-{tag}")
}

#[test]
fn a_first_claim_conflicts_with_nothing() {
    let a = live_agent("first-claim");
    let overlaps = claim(&a, Some("planner"), &declared(&["src/lib.rs"]));
    assert!(overlaps.is_empty());
    release(&a);
}

/// The regression M2 exists for: two agents editing one file was invisible.
#[test]
fn two_agents_declaring_the_same_file_overlap() {
    let a = live_agent("same-file-a");
    let b = live_agent("same-file-b");
    claim(&a, Some("planner"), &declared(&["src/same_file/mod.rs"]));

    let overlaps = claim(
        &b,
        Some("implementer"),
        &declared(&["src/same_file/mod.rs"]),
    );

    assert_eq!(overlaps.len(), 1, "{overlaps:?}");
    assert_eq!(overlaps[0].paths, vec!["src/same_file/mod.rs".to_string()]);
    assert!(overlaps[0].holder.describe().contains("planner"));

    release(&a);
    release(&b);
}

#[test]
fn disjoint_declarations_do_not_overlap() {
    let a = live_agent("disjoint-a");
    let b = live_agent("disjoint-b");
    claim(&a, None, &declared(&["src/one.rs"]));

    let overlaps = claim(&b, None, &declared(&["src/two.rs"]));

    assert!(overlaps.is_empty(), "{overlaps:?}");
    release(&a);
    release(&b);
}

/// A glob against a concrete file is the case a plain string comparison
/// misses, and the reason this defers to the shared resource-key table rather
/// than comparing paths itself.
#[test]
fn a_glob_overlaps_a_file_it_covers() {
    let a = live_agent("glob-a");
    let b = live_agent("glob-b");
    claim(&a, None, &declared(&["crates/archon-core/**"]));

    let overlaps = claim(&b, None, &declared(&["crates/archon-core/src/lib.rs"]));

    assert_eq!(overlaps.len(), 1, "a glob must cover the files under it");
    release(&a);
    release(&b);
}

/// Windows and Unix spellings of one path are the same resource. The shared
/// table unifies separators on every platform; doing it here too would be a
/// second opinion that could disagree.
#[test]
fn separator_differences_still_overlap() {
    let a = live_agent("sep-a");
    let b = live_agent("sep-b");
    claim(&a, None, &declared(&["src\\SepCase\\Mod.rs"]));

    let overlaps = claim(&b, None, &declared(&["src/SepCase/Mod.rs"]));

    assert_eq!(overlaps.len(), 1, "{overlaps:?}");
    release(&a);
    release(&b);
}

/// Case folding follows the filesystem, not the table.
///
/// On Windows and macOS `Mod.rs` and `mod.rs` are one file, so two agents
/// writing them collide. On Linux they are two files and do not. Asserting a
/// single answer everywhere would have to be wrong on one of them — which is
/// exactly what this test did before, passing on Windows and macOS and failing
/// on Linux. `fold_case_for_os` is per-OS for the same reason.
#[test]
fn case_differences_overlap_only_where_the_filesystem_folds_them() {
    let a = live_agent("case-a");
    let b = live_agent("case-b");
    claim(&a, None, &declared(&["src/CaseFold/Mod.rs"]));

    let overlaps = claim(&b, None, &declared(&["src/casefold/mod.rs"]));

    if cfg!(any(target_os = "windows", target_os = "macos")) {
        assert_eq!(
            overlaps.len(),
            1,
            "a case-insensitive filesystem makes these one file: {overlaps:?}"
        );
    } else {
        assert!(
            overlaps.is_empty(),
            "a case-sensitive filesystem makes these two files: {overlaps:?}"
        );
    }
    release(&a);
    release(&b);
}

/// The property that removes the need for a release path at all. Every
/// terminal hook skips auto-backgrounded agents, so a claim that had to be
/// released would leak on exactly the long-running agents most likely to hold
/// one.
#[test]
fn a_dead_agents_claim_cannot_conflict() {
    let ghost = dead("ghost");
    let live = live_agent("outlives-ghost");
    claim(&ghost, Some("abandoned"), &declared(&["src/shared.rs"]));

    let overlaps = claim(&live, None, &declared(&["src/shared.rs"]));

    assert!(
        overlaps.is_empty(),
        "a finished agent must not block a new one: {overlaps:?}"
    );
    release(&live);
}

/// Re-declaring replaces rather than accumulates, and an agent never conflicts
/// with itself — a resumed agent re-stating its intent is not a collision.
#[test]
fn reclaiming_replaces_and_never_self_conflicts() {
    let a = live_agent("reclaim");
    claim(&a, None, &declared(&["src/first.rs"]));

    let overlaps = claim(&a, None, &declared(&["src/first.rs", "src/second.rs"]));
    assert!(overlaps.is_empty(), "{overlaps:?}");

    let held: Vec<_> = live_claims()
        .into_iter()
        .filter(|c| c.agent_id == a)
        .collect();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].declared.len(), 2, "the new declaration replaces");

    release(&a);
}

/// Declaring nothing claims nothing and blocks nobody — an agent that says
/// nothing is unconstrained, which is what makes the field safe to add.
#[test]
fn an_empty_declaration_conflicts_with_nothing() {
    let a = live_agent("empty-a");
    let b = live_agent("empty-b");
    claim(&a, None, &declared(&["src/anything.rs"]));

    let overlaps = claim(&b, None, &[]);

    assert!(overlaps.is_empty(), "{overlaps:?}");
    release(&a);
    release(&b);
}

#[test]
fn the_warning_names_the_holder_and_the_paths() {
    let a = live_agent("warned-a");
    let b = live_agent("warned-b");
    claim(&a, Some("reviewer"), &declared(&["docs/spec.md"]));
    let overlaps = claim(&b, None, &declared(&["docs/spec.md"]));

    let text = describe_overlaps(&overlaps);

    assert!(text.contains("reviewer"), "{text}");
    assert!(text.contains("docs/spec.md"), "{text}");
    assert!(
        text.contains("isolation"),
        "should suggest a remedy: {text}"
    );

    release(&a);
    release(&b);
}

#[test]
fn released_claims_stop_conflicting() {
    let a = live_agent("released-a");
    let b = live_agent("released-b");
    claim(&a, None, &declared(&["src/gone.rs"]));
    release(&a);

    let overlaps = claim(&b, None, &declared(&["src/gone.rs"]));

    assert!(overlaps.is_empty(), "{overlaps:?}");
    release(&b);
}

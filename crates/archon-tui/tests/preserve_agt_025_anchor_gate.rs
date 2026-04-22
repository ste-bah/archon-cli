//! PRESERVE-D5 ANCHOR GATE — TUI-side dead-man's switch for upstream AGT-025 guard.
//!
//! TASK-TUI-904 closure rationale: Post-TASK-AGS-105 the AGT-025 race moved from
//! archon-core/src/agent.rs to archon-tools/src/agent_tool.rs::run_subagent.
//! archon-core/tests/preserve_d5_agt025.rs was updated to scan the new location
//! and provides complete REQ-FOR-PRESERVE-D5 coverage (4 invariants). It runs
//! as part of `cargo test --workspace` which CI invokes at ci.yml:69.
//!
//! TC-TUI-PRESERVE-05 acceptance ("upstream AGT-025 regression test remains green")
//! is satisfied transitively. This anchor gate is the TUI-side dead-man's switch:
//! it does not duplicate coverage — it pins the existence + invariant-set of the
//! upstream guard so a future TUI refactor cannot silently delete or tamper with it.
//!
//! Reference: REQ-FOR-PRESERVE-D5, AC-PRESERVE-05, TC-TUI-PRESERVE-05
//! Reference: archon-core/tests/preserve_d5_agt025.rs (upstream guard)
//! Reference: TASK-AGS-105 (moved AGT-025 race to archon-tools::agent_tool::run_subagent)

/// Pinned snapshot of the upstream D5 guard via compile-time include_str!.
/// If the file is deleted/moved, this fails to compile.
const UPSTREAM_GUARD_SOURCE: &str =
    include_str!("../../archon-core/tests/preserve_d5_agt025.rs");

/// The 4 invariant test functions required by REQ-FOR-PRESERVE-D5.
const REQUIRED_TEST_FUNCTIONS: &[&str] = &[
    "fn test_auto_background_ms_constant_is_120000",
    "fn test_env_gate_archon_auto_background_tasks",
    "fn test_timeout_branch_does_not_reawait_join_handle",
    "fn test_subagent_manager_tracks_running_and_completed",
];

#[test]
fn test_upstream_d5_guard_has_all_four_invariants() {
    let mut missing: Vec<&str> = Vec::new();
    for fn_name in REQUIRED_TEST_FUNCTIONS {
        if !UPSTREAM_GUARD_SOURCE.contains(fn_name) {
            missing.push(fn_name);
        }
    }
    assert!(
        missing.is_empty(),
        "REQ-FOR-PRESERVE-D5 anchor gate violated: \
         archon-core/tests/preserve_d5_agt025.rs is missing {} required invariant \
         test function(s): {:?}. TC-TUI-PRESERVE-05 requires the upstream AGT-025 \
         guard to remain intact. Restore the deleted/renamed functions OR update \
         TASK-TUI-904 with a TUI-side replacement gate.",
        missing.len(),
        missing
    );
}

#[test]
fn test_upstream_d5_guard_scans_post_ags_105_location() {
    // Post-TASK-AGS-105, the AGT-025 race lives in archon-tools/src/agent_tool.rs
    // (not archon-core/src/agent.rs). The upstream guard MUST scan the new location.
    assert!(
        UPSTREAM_GUARD_SOURCE.contains("archon-tools/src/agent_tool.rs"),
        "REQ-FOR-PRESERVE-D5 anchor gate violated: upstream guard no longer scans \
         the post-AGS-105 AGT-025 location (archon-tools/src/agent_tool.rs::run_subagent). \
         If the guard reverted to the pre-AGS-105 path (archon-core/src/agent.rs) it is \
         scanning a dead code path and D5 is silently unguarded."
    );
}

#[test]
fn test_upstream_d5_guard_has_preserve_d5_references() {
    // Sanity: the upstream file must identify itself as the PRESERVE-D5 guard.
    // Prevents drift where the filename stays but the purpose mutates.
    let d5_mentions = UPSTREAM_GUARD_SOURCE.matches("REQ-FOR-PRESERVE-D5").count();
    assert!(
        d5_mentions >= 4,
        "REQ-FOR-PRESERVE-D5 anchor gate violated: upstream guard has only {} \
         REQ-FOR-PRESERVE-D5 references (expected >=4, one per invariant). The file \
         may have been repurposed away from AGT-025 guard duty.",
        d5_mentions
    );
}

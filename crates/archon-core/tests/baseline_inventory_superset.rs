//! Phase-4/6/7 baseline inventory superset guard — PLACEHOLDER.
//!
//! Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-012.md
//! Based on:  tests/fixtures/baseline/{main_rs_loc,slash_commands,providers}.txt
//!            captured by scripts/capture-inventory.sh
//!
//! On phase-0 HEAD this file intentionally does nothing — it is a
//! documented extension point for phase-4, phase-6, and phase-7 tasks
//! to assert:
//!
//!     current_inventory ⊇ baseline_inventory
//!
//! i.e. refactors may ADD slash commands / providers / shrink main.rs,
//! but they must never REMOVE a slash command, drop a provider type,
//! or regress main.rs back above the captured line count.
//!
//! The test passes trivially today so that `cargo test -p archon-core
//! --test baseline_inventory_superset` is always part of CI. Later
//! phases replace `fn test_baseline_inventory_superset_placeholder`
//! with real superset assertions (and MAY rename the test); the
//! placeholder exists to keep that migration low-friction — no new
//! crate metadata, no new test target, no new Cargo.toml entry.

/// REGRESSION-GUARD PLACEHOLDER: DO NOT REMOVE.
///
/// Phase-4/6/7 tasks replace the body with real superset checks. The
/// function signature, the test name, and the fact that it is
/// discovered by `cargo test -p archon-core --test
/// baseline_inventory_superset` MUST be preserved so that the CI gate
/// wrappers (ci-gate.sh step 5, TASK-AGS-007) continue to run it.
#[test]
fn test_baseline_inventory_superset_placeholder() {
    // Intentionally empty. See module-level doc for phase-4/6/7
    // extension contract.
}

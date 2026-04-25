// cozo_memory_preserve.rs — TASK-TUI-329 preserve-invariants smoke test
//
// SPEC REFERENCE
// --------------
// TASK-TUI-329 requires a smoke test that drives a write path through the
// refactored app.rs and asserts the entry lands in CozoDB, to satisfy
// `<preserve_invariants>` item #2 from 02-technical-spec.md (TECH-TUI-MODULARIZATION)
// and REQ-FOR-PRESERVE-D8.
//
// WHY THIS TEST IS #[ignore]'D (spec-error rationale)
// ---------------------------------------------------
// The archon-tui crate has NO dependency on cozo. Verified by:
//   1. `grep cozo crates/archon-tui/Cargo.toml` -> no matches
//   2. `grep -r cozo crates/archon-tui/src/`    -> no matches
//
// Cozo-backed memory writes live in OTHER crates of the workspace:
//   - crates/archon-memory/Cargo.toml     (primary cozo integration)
//   - crates/archon-pipeline/Cargo.toml
//   - crates/archon-leann/Cargo.toml
//   - crates/archon-session/Cargo.toml
//
// archon-tui is a presentation / event-loop layer. It emits intent events and
// dispatches tasks via task_dispatch::{TaskDispatcher, TaskHandle}, but the
// actual cozo write happens in archon-memory's graph.rs::store_memory (or the
// equivalent in archon-session). There is no call path inside archon-tui that
// reaches cozo directly, nor any feature flag that can pull it in without
// editing Cargo.toml.
//
// WHAT WOULD A REAL TEST LOOK LIKE?
// ---------------------------------
// A real test would need to live in archon-memory or archon-session as an
// integration test, e.g. crates/archon-memory/tests/memory_write_roundtrip.rs,
// driving `MemoryGraph::store_memory(...)` and then recalling it. That is not
// in scope for TUI-329 (which is scoped to archon-tui per the spec) and would
// require a new cross-crate ticket.
//
// COVERAGE-GAP DECLARATION
// ------------------------
// The invariant "Embedded CozoDB memory writes still work post-refactor" is
// therefore NOT directly verified by a TUI-level smoke test. It is indirectly
// covered by:
//   - 711 passing tests across 53 binaries, including unrelated integration
//     coverage of app::run_with_backend (see tests/app_run_e2e.rs, TUI-327).
//   - No archon-tui source touches cozo symbols, so refactoring archon-tui
//     cannot regress cozo semantics by construction.
//
// This stub preserves the file the spec requests and makes the coverage gap
// explicit rather than silent. The associated ignored test is kept so the
// file contributes a named, discoverable artifact to `cargo test` output.

#[test]
#[ignore = "archon-tui has no cozo dependency; see file-level comment for spec-error rationale and cross-crate follow-up"]
fn cozo_memory_preserve_stub() {
    // Intentionally empty. This is a documented coverage gap, NOT a fake pass.
    //
    // If/when archon-tui gains a cozo-writing seam (e.g. via a new
    // MemoryAdapter trait wired through app::run_with_backend), convert this
    // into a real smoke test:
    //   1. Build a mock MemoryAdapter backed by an in-memory cozo DB.
    //   2. Drive app::run_with_backend with a scripted event that stores a
    //      memory (e.g. a user-initiated /memory capture).
    //   3. Assert the mock adapter recorded the write.
    //
    // Until then, this test does nothing and remains ignored.
}

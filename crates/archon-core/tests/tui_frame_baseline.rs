//! TUI frame snapshot baseline — phase-0 seed.
//!
//! Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-014.md
//! Based on:  02-technical-spec.md §1009 ("snapshot-test TUI frames
//!            with insta before-and-after each wave")
//! Derived from: REQ-FOR-D4 TUI modularization safety net
//!
//! ## What this test guards
//!
//! On phase-0 HEAD this test drives `DummyApp` — a minimal
//! `AppLike` implementation from `archon-test-support` — through one
//! `render_frame_to_string` call and asserts the result against an
//! `insta` snapshot that lives under
//! `tests/fixtures/snapshots/archon-core/`.
//!
//! Phase-4 tasks (REQ-FOR-D4 modularization) will:
//!   1. implement `AppLike` on the real `archon-cli` `App`, and
//!   2. add new snapshot tests per module wave (`_wave_1`, `_wave_2`,
//!      …) driven by real input sequences. Any frame diff across a
//!      wave is a regression to be reviewed via `cargo insta review`.
//!
//! The phase-0 seed exists so that phase-4 can start with a known-
//! good snapshot file on disk and a working cargo-insta integration;
//! it deliberately does NOT test the real `App`.

use archon_test_support::tui_capture::{DummyApp, render_frame_to_string};
use insta::{assert_snapshot, with_settings};

#[test]
fn test_dummy_app_startup_frame() {
    let mut app = DummyApp::default();
    let rendered = render_frame_to_string(&mut app, 40, 5);

    // Route insta's output to tests/fixtures/snapshots/archon-core/
    // (the workspace-root `insta.yaml` records the convention; the
    // per-test `with_settings!` override makes it actually land
    // there on disk because insta does not interpolate `{crate}`).
    with_settings!({
        snapshot_path => "../../../tests/fixtures/snapshots/archon-core",
        prepend_module_to_snapshot => false,
    }, {
        assert_snapshot!("startup", rendered);
    });
}

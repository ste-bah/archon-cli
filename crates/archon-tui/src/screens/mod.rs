//! Screens module index.
//!
//! Every screen module here is `pub(crate)`. That is deliberate and it is load
//! bearing.
//!
//! `archon-tui` is a library crate, so rustc treats any `pub` item as used by
//! definition — `dead_code` cannot fire on a screen that nothing constructs.
//! Twelve screens were built to spec, tested, and reachable from no code path
//! at all; the lint that should have caught it was disarmed by the visibility,
//! so CI stayed green over a set of features that only looked shipped (#189
//! Phase 0).
//!
//! With the modules crate-private, an unwired screen is a compile-time failure
//! instead. The single item the binary genuinely constructs is re-exported by
//! name from `lib.rs`, and that re-export is a claim that a real caller exists
//! — not that a test constructs it. Widening any of these back to `pub` would
//! reopen the hole.

pub(crate) mod cognitive;
pub(crate) mod docs;
pub(crate) mod evidence_browser;
// TASK-#207 SLASH-FILES: file-picker overlay (3-file sub-module).
pub(crate) mod file_picker;
pub(crate) mod gametheory;
pub(crate) mod hooks_config_menu;
pub(crate) mod learning;
pub(crate) mod memory_file_selector;
pub(crate) mod message_selector;
pub(crate) mod model_picker;
pub(crate) mod permissions_browser;
pub(crate) mod search_results;
pub(crate) mod session_branching;
pub(crate) mod settings_screen;
pub(crate) mod skills_menu;
pub(crate) mod task_overlay;
pub(crate) mod theme_screen;
pub(crate) mod video;
pub(crate) mod voice_capture;
pub(crate) mod workflow;
pub(crate) mod world;

/// Cross-screen coverage that used to live in `tests/`, moved in-crate when the
/// modules above became private (#189 Phase 0).
#[cfg(test)]
#[path = "evidence_engine_screens_tests.rs"]
mod evidence_engine_screens_tests;

/// `/search` overlay render coverage, kept out of `search_results.rs` so that
/// module stays under the 500-line ceiling.
#[cfg(test)]
#[path = "search_results_render_tests.rs"]
mod search_results_render_tests;

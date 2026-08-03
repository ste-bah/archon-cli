//! `archon-test-support` — test-only doubles and helpers for archon-cli.
//!
//! This crate is a `[dev-dependencies]`-only sibling of the production
//! workspace crates. It exists so that phase-specific tests (REQ-FOR-D6
//! provider acceptance, REQ-FOR-PRESERVE-D8 memory regression guard,
//! `.archon/` scoped fixtures, etc.) can share a small set of fakes
//! without duplicating `#[cfg(test)]` modules across every crate.
//!
//! ## Non-dependency rule
//!
//! No production crate (`archon-core`, `archon-tools`, `archon-llm`,
//! `archon-memory`, `archon-session`, the `archon` binary, …) depends
//! on this crate. Consumers opt in via:
//!
//! ```toml
//! [dev-dependencies]
//! archon-test-support = { path = "../archon-test-support" }
//! ```
//!
//! A CI grep (see TASK-AGS-008 validation #4) enforces the rule.
//!
//! ## Modules
//!
//! - [`fixtures`] — the workflow-live JSON corpus, `include_str!`d once and
//!   exposed as `pub const`, so tests in the binary and tests in
//!   `archon-workflow` read the same bytes with no relative path between them.
//! - [`provider`] — `MockProvider` + `spawn_mock_server()` for LLM
//!   acceptance tests.
//! - [`memory`] — `MockMemoryTrait` recording `store_memory` calls for
//!   REQ-FOR-PRESERVE-D8 regression guard.
//! - [`tempdir`] — `ArchonTempDir` RAII wrapper around `tempfile` that
//!   lays out the standard `.archon/` skeleton.
//! - [`tui_capture`] — `FrameBuffer`, `AppLike`, `DummyApp`, and
//!   `render_frame_to_string` for insta-based TUI snapshot tests
//!   (phase-4 REQ-FOR-D4 modularization safety net).

pub mod fixtures;
pub mod memory;
pub mod provider;
pub mod tempdir;
pub mod tui_capture;

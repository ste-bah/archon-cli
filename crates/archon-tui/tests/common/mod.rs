//! TASK-TUI-811: Shared test helpers for load-test integration tests.
//!
//! Per TECH-TUI-OBSERVABILITY lines 1190-1193 the 100-concurrent load test
//! at `subagent_100_concurrent.rs` and the sibling harness planned for
//! TASK-TUI-812 both live under `crates/archon-tui/tests/` and want a
//! shared helper module. Cargo's per-test-binary convention means any
//! `tests/*.rs` file compiles as its own crate; shared code goes in
//! `tests/common/mod.rs` and is pulled in via `mod common;` from each
//! test file (see https://doc.rust-lang.org/book/ch11-03-test-organization.html).
//!
//! This file is intentionally minimal. TASK-TUI-811 inlines its stub
//! executor fixture because it predates TASK-TUI-810's `load-tests`
//! feature gate and the `StubExecutor` pattern is reused verbatim from
//! `crates/archon-tools/tests/task_ags_104.rs`. When TASK-TUI-812
//! lands a second load-test file that wants the same scaffolding, the
//! executor fixture can migrate here.

#![allow(dead_code)]

// TASK-TUI-813: shared RSS + linear-growth assertion helpers.
pub mod memory;

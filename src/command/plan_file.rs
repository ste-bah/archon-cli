//! TASK-P0-B.3 (#174) bin-crate facade for plan-file I/O helpers.
//!
//! TASK #228: facade helpers exist for the Gate-1 structural verifier;
//! some are not invoked yet from dispatch — file-level allow keeps the
//! grep surface intact.
#![allow(dead_code)]
//!
//! The canonical implementation lives in `archon-core` at
//! `crates/archon-core/src/plan_file.rs` so the dispatch layer
//! (library) and the `/plan` slash-command handler (bin) share ONE
//! implementation without a cyclic dep.
//!
//! This module exposes a thin pass-through for each helper so the
//! Gate-1 structural verifier (`p0b-3-plan-mode-wired.sh`) can grep
//! for `pub fn <name>` at this path AND so handler code stays on the
//! `crate::command::plan_file::*` import surface. Every wrapper is
//! `#[inline]` so there is no runtime cost vs. a direct call.
//!
//! Resolution (ii) in the P0-B.3 spec: keep the file under
//! `src/command/` so the Gate-1 test keeps grepping the same path.

use std::path::{Path, PathBuf};

/// Resolve the plan file path — see [`archon_core::plan_file::plan_path`].
#[inline]
pub fn plan_path(working_dir: &Path) -> PathBuf {
    archon_core::plan_file::plan_path(working_dir)
}

/// Read the plan file contents — see
/// [`archon_core::plan_file::read_plan_file`].
#[inline]
pub fn read_plan_file(path: &Path) -> std::io::Result<Option<String>> {
    archon_core::plan_file::read_plan_file(path)
}

/// Append an intercepted-tool-call entry — see
/// [`archon_core::plan_file::append_plan_entry`].
#[inline]
pub fn append_plan_entry(
    path: &Path,
    tool_name: &str,
    input: &serde_json::Value,
) -> std::io::Result<()> {
    archon_core::plan_file::append_plan_entry(path, tool_name, input)
}

/// Open the plan file in `$EDITOR` — see
/// [`archon_core::plan_file::open_plan_in_editor`].
#[inline]
pub fn open_plan_in_editor(path: &Path) -> std::io::Result<()> {
    archon_core::plan_file::open_plan_in_editor(path)
}

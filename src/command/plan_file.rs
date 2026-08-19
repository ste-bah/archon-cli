//! TASK-P0-B.3 (#174) bin-crate facade for plan-file I/O helpers.
//!
//! The canonical implementation lives in `archon-core` at
//! `crates/archon-core/src/plan_file.rs` so the dispatch layer
//! (library) and the `/plan` slash-command handler (bin) share ONE
//! implementation without a cyclic dep. Each wrapper here is `#[inline]`,
//! so routing the handler through them costs nothing over a direct call.
//!
//! Only the wrappers `plan.rs` actually calls remain. `plan_audit_path` and
//! `append_plan_entry` were here solely so a structural verifier could grep
//! for `pub fn <name>` at this path — dispatch reaches the `archon-core`
//! originals directly, so the copies were a text-matching decoy that
//! `#![allow(dead_code)]` was needed to keep quiet. Both are gone, and so is
//! the allow: this module now fails the build if a wrapper stops being used.

use std::path::{Path, PathBuf};

/// Resolve an editable document path for a plan.
#[inline]
pub fn plan_document_path(working_dir: &Path, plan_id: &str) -> std::io::Result<PathBuf> {
    archon_core::plan_file::plan_document_path(working_dir, plan_id)
}

/// Write a structured plan as its editable Markdown document.
#[inline]
pub fn write_plan_document(
    path: &Path,
    plan: &archon_session::plan::PlanDocument,
) -> std::io::Result<()> {
    archon_core::plan_file::write_plan_document(path, plan)
}

/// Read editable plan-document text.
#[inline]
pub fn read_plan_document(path: &Path) -> std::io::Result<Option<String>> {
    archon_core::plan_file::read_plan_document(path)
}

/// Open the plan file in `$EDITOR` — see
/// [`archon_core::plan_file::open_plan_in_editor`].
#[inline]
pub fn open_plan_in_editor(path: &Path) -> std::io::Result<()> {
    archon_core::plan_file::open_plan_in_editor(path)
}

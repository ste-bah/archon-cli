//! `archon-write-plan` — one answer to "do these two write sets collide?".
//!
//! # Why this is its own crate
//!
//! Two layers ask that question. The write coordinator
//! (`archon_workflow::write_coordinator`) asks it at plan time, to schedule a
//! coordinated implementation fan-out into non-conflicting waves. Live
//! admission (`archon_topology::live`) asks it on the synchronous critical path
//! of every non-`Safe` tool call, to block a write that would race a
//! concurrently live node. Both must get the same answer, including the
//! deliberate fail-safes — a malformed glob conflicts with everything, and so
//! does a shared-append claim that names no resolvable file.
//!
//! Sharing the table by having `archon-topology` depend on `archon-workflow`
//! closed a cycle: `archon-core -> archon-workflow -> archon-topology ->
//! archon-core`. Cargo rejects a cyclic package graph outright (`cargo
//! metadata` exits 101), and features cannot break it — Cargo features are
//! additive, so a feature-gated edge is still an edge.
//!
//! So the table lives below both of them, in a crate with **no `archon-*`
//! dependencies at all**. Adding one would recreate the problem this crate
//! exists to remove; the leaf-ness is the point, not an accident of the current
//! contents.
//!
//! `archon-workflow` re-exports both modules under their old paths, so
//! `archon_workflow::write_coordinator::write_plan::…` still resolves.
//!
//! # Modules
//!
//! - [`write_plan`] — path normalization, resource keys, and
//!   [`write_plan::keys_conflict`], the overlap table itself.
//! - [`shared_append`] — the one way a [`write_plan::ResourceKey::SharedAppend`]
//!   gets built, and the item-payload field that declares it.

pub mod shared_append;
pub mod write_plan;

pub use shared_append::{
    SHARED_APPEND_TARGETS_KEY, resolve_shared_append_targets,
    resource_keys_for_targets_with_shared_append, shared_append_key,
    shared_append_key_for_raw_target,
};
pub use write_plan::{
    NormalizedPath, ResourceKey, TargetFilesSource, WritePlan, WritePlanError, fold_resource_case,
    keys_conflict, normalize_target, parse_baseline_id, resolve_target_files,
    resource_key_for_raw_target, resource_keys_for_targets,
};

/// Canonical fan-out item identifier.
///
/// Matches the `id` of the fan-out item a [`WritePlan`] was built for. Kept as
/// a named alias rather than a bare `String` because [`WritePlan::item_id`] is
/// one of three string fields on that struct and the alias is what tells them
/// apart at a glance.
pub type ItemId = String;

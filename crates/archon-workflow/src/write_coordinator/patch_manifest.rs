//! TASK-WC-005 — Patch capture from the isolated workspace + validation manifest.
//!
//! After the agent finishes, capture its patch with a SINGLE `git diff --binary
//! HEAD -- <targets>` (already combines staged + unstaged), validate it against
//! the declared contract before it touches canonical, and persist the durable
//! manifest + patch evidence.

pub(crate) mod code_hygiene;
mod git_changes;
mod scan_note;
mod secret_scan;
mod target_hashes;

pub(crate) use git_changes::workspace_changed_paths;
use git_changes::{
    diff_targets, is_ignored, is_tracked, parse_name_status, run_diff, validated_workspace_changes,
};

pub use scan_note::{COMPLEXITY_SCAN_UNRELIABLE, UnreliableScan};

include!("patch_manifest_a.rs");
include!("patch_manifest_b.rs");

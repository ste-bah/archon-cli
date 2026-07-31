//! TASK-WC-005 — Patch capture from the isolated workspace + validation manifest.
//!
//! After the agent finishes, capture its patch with a SINGLE `git diff --binary
//! HEAD -- <targets>` (already combines staged + unstaged), validate it against
//! the declared contract before it touches canonical, and persist the durable
//! manifest + patch evidence.

mod code_hygiene;
mod secret_scan;
mod target_hashes;

include!("patch_manifest_a.rs");
include!("patch_manifest_b.rs");

//! Declaring a path as coordinated-append rather than exclusive.
//!
//! [`ResourceKey::SharedAppend`] carries the overlap semantics; this module is
//! the only way one gets built, and it exists so that the *declaration* is
//! visible in one place rather than inferred.
//!
//! # Why an explicit field, and not a naming convention
//!
//! The decomposed-PRD surface offers a tempting inference: a deliverable
//! contract whose `kind` ends `_entry`, naming a path some other task declares
//! as a whole artifact, is plausibly one writer appending to a registry another
//! task owns. It is tempting and it is wrong to build on, for three reasons.
//!
//! 1. **It fails open on a typo.** `kind` is free text. An author writing
//!    `audit_entry` for a path that happens to collide with another task's
//!    artifact would silently get concurrent writes to it. The whole point of
//!    this key is that it is an assertion; an assertion nobody realised they
//!    were making is not one.
//! 2. **It is not local.** Whether task A's write is exclusive would depend on
//!    whether some task B exists elsewhere in the corpus. Adding an unrelated
//!    task would change A's conflict semantics without A changing.
//! 3. **It does not generalise.** Two of the three surfaces that build resource
//!    keys — a `WorkflowSpec` fan-out item and a live tool call — have no
//!    `kind` field and never will. A rule only one surface can express is not a
//!    rule this table can rely on.
//!
//! So the declaration is an explicit list of paths, read from the same item
//! payload that already declares `target_files`
//! ([`SHARED_APPEND_TARGETS_KEY`]), and it **defaults to empty**. A path is
//! exclusive unless something names it here. Nothing becomes concurrent by
//! omission, by inference, or by accident.
//!
//! # What declaring it does not do
//!
//! It does not make the write atomic. It stops the coordinator scheduling
//! around the path, and that is all it does. The author is asserting that their
//! writer is already safe — a read-modify-write under a lock, an append-only
//! log, a temp-file-and-rename. Where the surrounding requirement is that
//! interrupted writes leave no half-registered state, that requirement is met
//! by the writer, not by this key.

use std::collections::BTreeSet;

use super::write_plan::{
    NormalizedPath, ResourceKey, WritePlanError, fold_resource_case, resource_keys_for_targets,
};

/// Item-payload key listing paths this item appends to under coordination.
///
/// Sits beside `target_files` / `expected_target_files` in the same payload
/// (PRD-012 §8.1). Absent means every target is exclusive.
pub const SHARED_APPEND_TARGETS_KEY: &str = "shared_append_target_files";

/// Paths one item declares it appends to under coordination.
///
/// Absent, null, or an empty array all mean "none": no path becomes concurrent
/// because a field was left out. A present array with a non-string entry is an
/// error rather than a filtered list — a malformed concurrency declaration must
/// not be read as a smaller one.
pub fn resolve_shared_append_targets(
    item_payload: &serde_json::Value,
) -> Result<Vec<String>, WritePlanError> {
    let Some(value) = item_payload.get(SHARED_APPEND_TARGETS_KEY) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let Some(entries) = value.as_array() else {
        return Err(WritePlanError::InvalidTargetPath(format!(
            "item `{SHARED_APPEND_TARGETS_KEY}` is not an array"
        )));
    };
    let mut paths = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(path) = entry.as_str() else {
            return Err(WritePlanError::InvalidTargetPath(format!(
                "item `{SHARED_APPEND_TARGETS_KEY}` contains a non-string entry"
            )));
        };
        paths.push(path.to_string());
    }
    Ok(paths)
}

/// The shared-append key for an already-normalized target.
#[must_use]
pub fn shared_append_key(target: &NormalizedPath) -> ResourceKey {
    ResourceKey::SharedAppend(target.key_string())
}

/// The shared-append key for a raw declared path, **touching no filesystem**.
///
/// The counterpart of
/// [`resource_key_for_raw_target`](super::write_plan::resource_key_for_raw_target),
/// for milestone 3 admission, which is synchronous on the critical path of
/// every non-`Safe` tool call and cannot canonicalise. Separators unified and
/// case folded the same way, so a key built here and a key built from a
/// [`NormalizedPath`] compare equal.
///
/// A raw target carrying glob metacharacters still becomes a `SharedAppend`
/// key here rather than being silently downgraded — and
/// [`keys_conflict`](super::write_plan::keys_conflict) then treats it as
/// conflicting with everything, because a pattern is not a coordinated claim on
/// a file. Downgrading it to `Glob` would be the fail-open reading.
#[must_use]
pub fn shared_append_key_for_raw_target(raw: &str) -> ResourceKey {
    ResourceKey::SharedAppend(fold_resource_case(&raw.trim().replace('\\', "/")))
}

/// Resource keys for one item, with `shared` declared coordinated-append.
///
/// Every target not named in `shared` goes through
/// [`resource_keys_for_targets`] unchanged, so exclusive targets keep their
/// `File` key and their created-parent `Dir` keys exactly as before.
///
/// A shared target contributes **only** its `SharedAppend` key: no `File` key,
/// which would conflict with the other appenders and defeat the declaration,
/// and no ancestor `Dir` keys either. The directory part is deliberate — you
/// cannot coordinate an append to a file while racing to create the directory
/// holding it, so a shared-append claim on `d/f` is also a claim that `d` is
/// shared. Exclusivity over `d` remains enforceable from the side that asserts
/// it: an item that creates `d` for its own targets still emits `Dir(d)`, and
/// `Dir(d)` against `SharedAppend(d/f)` is a conflict.
pub fn resource_keys_for_targets_with_shared_append(
    targets: &[NormalizedPath],
    canonical_root: &std::path::Path,
    declared_globs: &[String],
    shared: &[NormalizedPath],
) -> Result<BTreeSet<ResourceKey>, WritePlanError> {
    if shared.is_empty() {
        return resource_keys_for_targets(targets, canonical_root, declared_globs);
    }
    let shared_keys: BTreeSet<String> = shared.iter().map(NormalizedPath::key_string).collect();
    let exclusive: Vec<NormalizedPath> = targets
        .iter()
        .filter(|target| !shared_keys.contains(&target.key_string()))
        .cloned()
        .collect();
    let mut keys = resource_keys_for_targets(&exclusive, canonical_root, declared_globs)?;
    keys.extend(shared_keys.into_iter().map(ResourceKey::SharedAppend));
    Ok(keys)
}

#[cfg(test)]
#[path = "shared_append_tests.rs"]
mod tests;

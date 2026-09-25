//! The identity a write branch is reused under: the item as the script
//! authored it, not as the host stamped it (Issue-24).
//!
//! `write::run_write_capable_v2_fanout` prepares every branch before it asks
//! `branch_cache` which stored outcomes may be reused, and that preparation
//! writes host-derived data INTO the item input: the current line count of
//! every declared target (`write::target_budgets`), the repository root
//! (`write::repository_root`), an evidence-bound write scope
//! (`write::scope_discovery`, an agent turn), contract deliverables and the
//! universe's tool binding. The stored `item_input_hash` was the hash of that
//! stamped input, so it moved whenever the tree did.
//!
//! Live: a later wave grew a file an earlier wave's item also declared; on the
//! next resume the earlier item's `current_lines` differed, its hash differed,
//! reuse was refused, and an already-committed task was dispatched to a coder
//! again — 20 to 60 minutes per item, on every resume, for every earlier wave
//! — where it stalled at the read wall with nothing left to write.
//!
//! The fix is one identity, computed from the input BEFORE any stamp touches
//! it, carried on the item under [`REUSE_INPUT_HASH_KEY`] and read by both the
//! save sites and the reuse decision. [`reuse_input_hash`] is the projection
//! behind it: it removes exactly the keys the host stamps, so an input that
//! arrives already stamped (a caller that prepared its own branches, or a
//! test) still hashes to the authored item.
//!
//! Outcomes stored before this identity existed carry the hash of the whole
//! stamped input. [`recorded_hash_matches`] accepts that too, so nothing that
//! reused before stops reusing; a stored hash matching neither is a miss.

use serde_json::Value;

use crate::v2::scheduler::{WorkflowV2FanoutItem, stable_value_hash};

/// Top-level input key the authored identity is carried under. Written by
/// [`stamp_reuse_input_hash`]; the top level of a branch input is built by the
/// host (`call_data::source`), never by an agent, so it cannot be forged from
/// an item value.
pub const REUSE_INPUT_HASH_KEY: &str = "_reuse_input_hash";

/// Top-level input key carrying, for a review-remediation branch, the
/// authored identity rebased to each same-label sibling's ordinal, keyed by
/// the sibling's call id. Written beside [`REUSE_INPUT_HASH_KEY`] and for the
/// same reason: only the authored input can be rebased, and it is gone once
/// the host stamps the branch (`branch_cache::stamp_drift_identities`).
pub const DRIFT_IDENTITIES_KEY: &str = "_reuse_drift_identities";

/// Top-level input keys the host writes. Removed before hashing.
pub const VOLATILE_INPUT_KEYS: &[&str] = &[
    // `write::stamp_project_artifact_policy`: the project's artifact-root
    // policy read from the store. The same for every item, not the item's.
    "_workflow_project_artifact_policy",
    // This module's own stamp, so the projection is idempotent.
    REUSE_INPUT_HASH_KEY,
    // The drift identities written beside it, for the same reason.
    DRIFT_IDENTITIES_KEY,
    // `write::forbidden_paths::stamp`: the paths the item's tasks forbid, for
    // the tool guard. Derived from the task universe by the item's (kept)
    // canonical task ids, exactly as `required_tools` is (Issue-30).
    crate::agent_dispatch_port::FORBIDDEN_PATHS_INPUT_KEY,
    // `write::declared_targets::stamp`: the branch's widened target set, for
    // the tool guard (Issue-64). Host-derived from the plan, never authored.
    crate::agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY,
    // `verification::baseline_rule::stamp_baseline_tests_input`: the task's
    // base-commit test lists, read from the run's own records (Obs-31).
    crate::v2::verification::baseline_rule::BASELINE_TESTS_INPUT_KEY,
];

/// `input.item` keys the host writes. Removed before hashing.
///
/// Kept, because they are the item as authored: `item_id` / `id`,
/// `canonical_task_ids`, the prompt and acceptance text, declared artifacts,
/// `work_type`, and `target_files` as the script declared them — the stamp is
/// taken before `stamp_contract_code_targets` and `scope_discovery` rewrite
/// that key, so their rewrites do not reach the identity either.
pub const VOLATILE_ITEM_KEYS: &[&str] = &[
    // `write::target_budgets`: current line count and remaining room per
    // declared target, measured from the tree at dispatch. Changes whenever
    // any wave touches the file — the live defect.
    "target_file_budgets",
    // `write::target_budgets`: the configured line cap. Host configuration.
    "max_source_file_lines",
    // `write::repository_root`: where the repository is on this host.
    "target_repository_root",
    // `write::stamp_required_tools_from_universe` and
    // `apply_source_graph_targets_to_branches`: derived from the task universe
    // by the item's (kept) canonical task ids. The shared builder strips any
    // agent-authored value before the item arrives, so it is never authored.
    "required_tools",
];

/// Hash of `input` with every host stamp removed: the item as authored.
pub fn reuse_input_hash(input: &Value) -> String {
    let mut projected = input.clone();
    if let Some(object) = projected.as_object_mut() {
        for key in VOLATILE_INPUT_KEYS {
            object.remove(*key);
        }
        if let Some(item) = object.get_mut("item").and_then(Value::as_object_mut) {
            for key in VOLATILE_ITEM_KEYS {
                item.remove(*key);
            }
        }
    }
    stable_value_hash(&projected)
}

/// Carry the authored identity on every branch that does not have one yet.
///
/// Must run before any other stamp in `write::run_write_capable_v2_fanout`:
/// `target_files` is authored AND rewritten later, and only the value seen
/// here is the authored one. An existing stamp is left alone, as
/// `repository_root` leaves an existing root alone — it was given
/// deliberately, and replacing it would silently change what the branch's
/// outcome is filed under.
pub fn stamp_reuse_input_hash(branches: &mut [WorkflowV2FanoutItem]) {
    for branch in branches {
        let hash = reuse_input_hash(&branch.input);
        if let Some(object) = branch.input.as_object_mut() {
            object
                .entry(REUSE_INPUT_HASH_KEY)
                .or_insert_with(|| Value::String(hash));
        }
    }
}

/// The identity `item` is reused under: the carried stamp, or the projection
/// of its input when nothing stamped it.
pub fn reuse_identity(item: &WorkflowV2FanoutItem) -> String {
    item.input
        .get(REUSE_INPUT_HASH_KEY)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| reuse_input_hash(&item.input))
}

/// The hash an outcome stored before this identity existed carries: the whole
/// stamped input, exactly as `WorkflowV2FanoutItem::input_hash` produced it
/// then — which is the input as it is now, less the stamp this module added.
pub fn legacy_input_hash(item: &WorkflowV2FanoutItem) -> String {
    let mut input = item.input.clone();
    if let Some(object) = input.as_object_mut() {
        object.remove(REUSE_INPUT_HASH_KEY);
        object.remove(DRIFT_IDENTITIES_KEY);
    }
    stable_value_hash(&input)
}

/// Whether a stored `item_input_hash` names this item: the authored identity,
/// or the legacy hash of the identically stamped input.
pub fn recorded_hash_matches(recorded: &str, item: &WorkflowV2FanoutItem) -> bool {
    recorded == reuse_identity(item) || recorded == legacy_input_hash(item)
}

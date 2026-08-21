//! Widen a write item's plan to the changed files nothing else in its wave owns.
//!
//! # Why this is here and not only in the adapter
//!
//! A write branch passes THREE independent ownership gates, and relaxing one
//! achieves nothing:
//!
//! 1. `validate_changed_files` — the adapter, judging the agent's result.
//! 2. `validated_workspace_changes` — the patch coordinator at capture time,
//!    raising `UndeclaredWrite` for anything outside `plan.target_files`.
//! 3. `validate_patch` — the same plan again, after capture.
//!
//! Gates 2 and 3 read the coordinator's `WritePlan`, so the plan itself is what
//! has to widen. Extending it here also puts the granted path into
//! `declared_target_files` (the manifest derives them from the hashed declared
//! targets), which matters for safety rather than convenience: a path that is
//! changed but NOT declared is invisible to `assert_no_path_overlap` and to the
//! stale-baseline recheck, so granting it anywhere else would let two items
//! write the same file with no guard between them.
//!
//! # What is granted
//!
//! Only a path no OTHER item in the wave claims. Disjoint ownership is
//! preserved exactly — a file nobody else owns cannot create a conflict by
//! being granted — and a contested path is left to fail as a genuine ownership
//! dispute that belongs in remediation.
//!
//! The candidate paths come from the agent's own `files_changed`. If it
//! under-reports, the extension misses that path and gate 2 rejects it exactly
//! as it does today: the failure mode is the status quo, never a silent pass.
//!
//! # The collision this deliberately makes LOUD
//!
//! Claims are built from DECLARED targets, so if two items in one wave each
//! change a file NEITHER declared, both see it unclaimed and both are granted
//! it. There are only two possible outcomes for that case and no third:
//!
//! - Granted paths become declared, as they do here, so `assert_no_path_overlap`
//!   sees the same path in two manifests and fails the WHOLE wave with
//!   `ConflictGraphViolation`. Harsh, and harsher than today, where each item is
//!   refused individually and the wave survives.
//! - Granted paths stay undeclared, and the overlap guard and the stale-baseline
//!   recheck — both of which filter on `declared_target_files` — cannot see the
//!   path at all. Two patches then write the same file with no guard between
//!   them.
//!
//! The first is a loud failure, the second is silent corruption of the canonical
//! tree. That is why the granted path is declared. Note that a granted path
//! carries no `pre_hash`, because the baseline was captured before the grant
//! existed, so the stale recheck skips it and the overlap guard is the only
//! thing standing there — which is the other reason it must not be bypassed.

use archon_write_plan::{NormalizedPath, WritePlan, normalize_target};

use crate::WorkflowV2Result;
use crate::v2::write_scope_extension::{WaveClaim, resolve_scope_extensions};

/// The plan this branch should actually be judged against.
///
/// Returns the plan unchanged when there is no wave context, when nothing was
/// changed outside it, or when every out-of-scope path is contested — so the
/// pre-existing behaviour is the default in every case that is not a clear
/// grant.
pub(super) fn plan_extended_to_unclaimed_changes(
    plan: &WritePlan,
    result: &WorkflowV2Result,
    wave_claims: Option<&[WaveClaim]>,
) -> WritePlan {
    let Some(wave) = wave_claims else {
        return plan.clone();
    };
    let outside: Vec<&str> = result
        .files_changed
        .iter()
        .map(|file| file.path.as_str())
        .filter(|path| !path_is_planned(plan, path))
        .collect();
    if outside.is_empty() {
        return plan.clone();
    }
    let (granted, _contested) = resolve_scope_extensions(plan.item_id.as_str(), outside, wave);
    let granted: Vec<NormalizedPath> = granted
        .iter()
        .filter_map(|path| normalize_target(path, &plan.canonical_root).ok())
        .collect();
    if granted.is_empty() {
        return plan.clone();
    }
    let mut extended = plan.clone();
    extended.target_files.extend(granted);
    extended.target_files.sort();
    extended.target_files.dedup();
    extended
}

/// Whether the plan already covers `path`, by file or by directory scope.
///
/// Normalisation failure counts as NOT planned, which sends the path to the
/// grant check, where it fails to normalise again and is dropped. A path the
/// coordinator cannot name is never granted.
fn path_is_planned(plan: &WritePlan, path: &str) -> bool {
    let Ok(normalized) = normalize_target(path, &plan.canonical_root) else {
        return false;
    };
    plan.target_files
        .iter()
        .chain(plan.target_dir_scopes.iter())
        .any(|owned| crate::v2::write_mode::paths_overlap(&owned.as_str(), &normalized.as_str()))
}

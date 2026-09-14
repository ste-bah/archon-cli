//! Widen a write item's plan to the changed files nothing else in its wave owns.
//!
//! # Why this is here and not only in the adapter
//!
//! A write branch passes THREE independent ownership gates, and relaxing one
//! achieves nothing:
//!
//! 1. `validate_worktree_branch_result` — the host, judging the agent's
//!    envelope against the plan's targets and scopes (the adapter runs the
//!    same check earlier, from the claims stamped on the call).
//! 2. `validated_workspace_changes` — the patch coordinator at capture time,
//!    raising `UndeclaredWrite` for anything outside `plan.target_files`.
//! 3. `validate_patch` — the same plan again, after capture.
//!
//! All three read ONE [`ScopeGrant`], resolved once per branch in
//! `run_one_worktree_branch` after the envelope is settled and before gate 1.
//! Issue-11 was the alternative: the grant was applied at capture only, but
//! gate 1 had already refused the envelope against the DECLARED targets and
//! replaced it with an empty one — so capture saw no changed files and nothing
//! was ever granted. Live on wf-5979fe15 `agents-5`: one item, no other
//! claimant, two unlisted files, the whole stage failed and eleven dependent
//! waves were skipped in the same second.
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
//! # What is NOT granted: a whitespace-only change
//!
//! A tree-wide formatter touches files the task never meant to own, and each
//! of them is "unclaimed" by the letter of the rule. Granting them would carry
//! formatter noise into the canonical tree under this item's name and, worse,
//! declare paths that a later wave's item may own. So a candidate whose actual
//! diff in the worktree is whitespace-only is not granted — and, since
//! Issue-13, not refused either: it is DROPPED. The worktree copy is restored
//! to the baseline before capture (`drop_whitespace_only_changes`), gate 1
//! ignores the envelope entry for it, and the branch reports the paths as a
//! review gap. Refusing it was the alternative, and live on wf-7db01ce7
//! `agents-3-0` that turned one `cargo fmt --all` into a failed branch with
//! every dependent wave skipped.
//!
//! The candidates here are BOTH the envelope's `files_changed` and the
//! worktree's own changed paths: a formatter touches far more files than an
//! agent reports, and an unreported one would otherwise meet gate 2 as an
//! undeclared write. The comparison itself lives in
//! `write_coordinator::whitespace_only`: bytes minus ASCII whitespace, against
//! the canonical file the worktree was created from. Either side unreadable —
//! a created or deleted file — is a real change, not whitespace, and is
//! granted.
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

/// The plan a branch is judged against, resolved once and read by every gate.
#[derive(Debug, Clone)]
pub(super) struct ScopeGrant {
    /// The coordinator plan widened by every granted path.
    pub(super) plan: WritePlan,
    /// Paths granted beyond the declared targets, repo-relative and sorted.
    pub(super) granted: Vec<String>,
    /// Undeclared paths whose worktree diff is whitespace-only, repo-relative
    /// and sorted: named by the envelope or found in the worktree. Dropped,
    /// not judged.
    pub(super) whitespace_only: Vec<String>,
}

impl ScopeGrant {
    /// The plan unchanged: no wave context, or nothing to widen.
    fn unchanged(plan: &WritePlan) -> Self {
        Self {
            plan: plan.clone(),
            granted: Vec::new(),
            whitespace_only: Vec::new(),
        }
    }

    /// Resolve what this branch may keep beyond its declared targets.
    ///
    /// Returns the plan unchanged when there is no wave context, when nothing
    /// was changed outside it, or when every out-of-scope path is contested or
    /// whitespace-only — so the pre-existing behaviour is the default in every
    /// case that is not a clear grant. The whitespace-only set is resolved
    /// with or without a wave: it is a property of the worktree, not of the
    /// wave, and gate 2 would reject those paths either way.
    pub(super) fn resolve(
        plan: &WritePlan,
        result: &WorkflowV2Result,
        wave_claims: Option<&[WaveClaim]>,
    ) -> Self {
        let mut whitespace_only =
            crate::write_coordinator::whitespace_only::undeclared_whitespace_only_changes(plan);
        let mut outside: Vec<String> = Vec::new();
        for file in &result.files_changed {
            let Some(relative) = repo_relative(plan, &file.path) else {
                // A path the coordinator cannot name is never granted: it
                // meets the ownership check exactly as it does today.
                continue;
            };
            if path_is_planned(plan, &relative) {
                continue;
            }
            if whitespace_only_change(plan, &relative.as_str()) {
                whitespace_only.push(relative.as_str().to_string());
                continue;
            }
            outside.push(relative.as_str().to_string());
        }
        whitespace_only.sort();
        whitespace_only.dedup();
        let Some(wave) = wave_claims else {
            return Self {
                whitespace_only,
                ..Self::unchanged(plan)
            };
        };
        if outside.is_empty() {
            return Self {
                whitespace_only,
                ..Self::unchanged(plan)
            };
        }
        let (granted, _contested) = resolve_scope_extensions(
            plan.item_id.as_str(),
            outside.iter().map(String::as_str),
            wave,
        );
        let granted: Vec<NormalizedPath> = granted
            .iter()
            .filter_map(|path| normalize_target(path, &plan.canonical_root).ok())
            .collect();
        if granted.is_empty() {
            return Self {
                whitespace_only,
                ..Self::unchanged(plan)
            };
        }
        let mut extended = plan.clone();
        extended.target_files.extend(granted.iter().cloned());
        extended.target_files.sort();
        extended.target_files.dedup();
        Self {
            plan: extended,
            granted: granted
                .iter()
                .map(|path| path.as_str().to_string())
                .collect(),
            whitespace_only,
        }
    }

    /// Whether `path` is one this branch was GRANTED beyond its declared
    /// targets — unclaimed by every other item in the wave, and declared in
    /// the manifest because of it.
    pub(super) fn is_granted(&self, path: &str) -> bool {
        repo_relative(&self.plan, path)
            .is_some_and(|relative| self.granted.iter().any(|p| *p == relative.as_str()))
    }

    /// Whether the granted plan covers `path`: a declared target, a path
    /// under a declared directory scope, or a granted path. This is the
    /// ownership the three gates judged the branch by, so anything else that
    /// reads "is this file the branch's to change" after the grant must read
    /// this and not the assignment's declared list (Issue-15).
    pub(super) fn covers(&self, path: &str) -> bool {
        repo_relative(&self.plan, path)
            .is_some_and(|relative| path_is_planned(&self.plan, &relative))
    }

    /// Whether `path` — as an envelope names it, by either root — is one of
    /// the whitespace-only paths this branch drops rather than judges.
    pub(super) fn is_whitespace_only(&self, path: &str) -> bool {
        repo_relative(&self.plan, path)
            .is_some_and(|relative| self.whitespace_only.iter().any(|p| *p == relative.as_str()))
    }

    /// Restore every whitespace-only path in the worktree to the baseline, so
    /// capture never sees it, and return the paths actually restored.
    ///
    /// Runs BEFORE the ownership gates: gate 2 reads the worktree, and the
    /// `patch_landed` marker is answered from it too. One that git cannot
    /// restore is left as it is and meets gate 2 exactly as it does today.
    pub(super) fn drop_whitespace_only_changes(&self) -> Vec<String> {
        if self.whitespace_only.is_empty() {
            return Vec::new();
        }
        crate::write_coordinator::whitespace_only::restore_to_baseline(
            &self.plan.isolated_root,
            &self.whitespace_only,
        )
    }
}

/// The plan this branch should actually be judged against.
///
/// Kept as the single-value form of [`ScopeGrant::resolve`] for callers that
/// need only the plan.
#[cfg(test)]
pub(super) fn plan_extended_to_unclaimed_changes(
    plan: &WritePlan,
    result: &WorkflowV2Result,
    wave_claims: Option<&[WaveClaim]>,
) -> WritePlan {
    ScopeGrant::resolve(plan, result, wave_claims).plan
}

/// `path` as the coordinator names it: relative to the repository root.
///
/// An envelope may name a file by its canonical path or by its worktree path
/// — the same file, from the other checkout — and gate 1 already strips
/// either root. The grant must read both the same way, or a worktree-rooted
/// path is never granted and gate 1 refuses a file capture would have kept.
fn repo_relative(plan: &WritePlan, path: &str) -> Option<NormalizedPath> {
    normalize_target(path, &plan.canonical_root)
        .or_else(|_| normalize_target(path, &plan.isolated_root))
        .ok()
}

/// Whether the plan already covers `path`, by file or by directory scope.
fn path_is_planned(plan: &WritePlan, path: &NormalizedPath) -> bool {
    plan.target_files
        .iter()
        .chain(plan.target_dir_scopes.iter())
        .any(|owned| crate::v2::write_mode::paths_overlap(&owned.as_str(), &path.as_str()))
}

/// Whether the agent's change to `relative` is whitespace-only, by the one
/// comparison every whitespace decision uses (`write_coordinator::whitespace_only`).
fn whitespace_only_change(plan: &WritePlan, relative: &str) -> bool {
    crate::write_coordinator::whitespace_only::whitespace_only_change(
        &plan.canonical_root.join(relative),
        &plan.isolated_root.join(relative),
    )
}

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
//! The candidate paths are what the agent ACTUALLY changed in its worktree —
//! every path that differs from the sealed baseline commit, tracked or
//! untracked-not-ignored, by the one scan capture itself uses
//! (`write_coordinator::whitespace_only::worktree_changes`) — united with the
//! envelope's `files_changed`. Not the envelope alone: an agent reports what it
//! remembers, and gates 2 and 3 judge what is on disk. Issue-16, live on
//! wf-719ff3b0 `agents-5-0`: twenty-two files changed, fourteen reported, the
//! nine unreported ones claimed by nobody, and gate 3 refused the first of them
//! as an undeclared write — the branch failed on a bookkeeping gap, not an
//! ownership one. Under-reporting is now a review finding: the unlisted paths
//! are granted by the same rules as a listed one and named in a residual gap
//! ([`ScopeGrant::unreported`]) for the reviewer. Over-reporting — a listed
//! path with no diff — stays harmless: it is a candidate like any other, and
//! there is nothing to capture for it.
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
//! The comparison itself lives in `write_coordinator::whitespace_only`: bytes
//! minus ASCII whitespace, against the canonical file the worktree was created
//! from. Either side unreadable — a created or deleted file — is a real
//! change, not whitespace, and is granted.
//!
//! # What is NOT granted: a change outside the plan's scope roots
//!
//! "Unclaimed" has no ceiling, and a single-item wave contests nothing.
//! Issue-27, live on wf-719ff3b0 `agents-11`: one item declared targets in
//! `crates/archon-trading/`, `crates/archon-tui/` and `src/`; the coder ran
//! clippy on an unrelated crate and edited twenty files under
//! `crates/archon-workflow/` and `crates/archon-knowledge/`, and all twenty
//! were granted, declared and committed under the task. So a real change
//! outside the plan's scope roots (`scope_roots`: the declared targets'
//! packages, or their top-level directories) is partitioned out of the
//! candidates BEFORE the wave contest — never granted, never declared, never
//! contested — and DROPPED the way a whitespace-only change is: restored to
//! the baseline, or removed when the agent created it, before capture
//! (`drop_out_of_scope_changes`), ignored by gate 1, and reported as a review
//! gap naming the roots. A path directly at the repository root is exempt:
//! workspace manifests and lockfiles are shared build files, and they still
//! go through the contest. Dropping rather than refusing, for the same
//! reason as Issue-13: the branch's real work must still land.
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
//!
//! # What is REJECTED: a change to a path the task forbids
//!
//! Issue-30, live on wf-719ff3b0 `agents-14-1`: the task's `Files Forbidden
//! to Change` list named the crate's gate and coverage modules, the coder
//! changed both, and every rule above admitted them — in scope, unclaimed,
//! real. So after the whitespace-only and out-of-scope partitions, every
//! REMAINING changed path the worktree scan reports — declared, granted or
//! contested alike — that matches the branch's [`ForbiddenPaths`] goes to
//! [`ScopeGrant::forbidden`], and `run_one_worktree_branch` rejects the
//! branch outright when that set is non-empty. Not dropped like the other
//! two partitions: restoring half an edit set leaves a crate that no later
//! verification can build, and the change the coder made there may be the
//! very thing the reviewer needs to see. The set is a property of the
//! worktree, resolved with or without a wave, and the judgement is on the
//! scan, not the envelope: an over-reported forbidden path with no diff is
//! not a change.

use archon_write_plan::{ForbiddenPaths, NormalizedPath, WritePlan, normalize_target};

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
    /// Real changes outside the plan's scope roots, repo-relative and sorted:
    /// named by the envelope or found in the worktree. Dropped before the
    /// wave contest, never granted (Issue-27).
    pub(super) out_of_scope: Vec<String>,
    /// The scope roots the partition was made against, for the preamble and
    /// the gap that names them.
    pub(super) roots: super::scope_roots::ScopeRoots,
    /// Paths changed in the worktree — declared, granted, contested or out
    /// of scope, but not whitespace-only — that the envelope's
    /// `files_changed` did not name. Repo-relative and sorted. A review
    /// finding, never a verdict.
    pub(super) unreported: Vec<String>,
    /// Paths changed in the worktree — after the whitespace-only and
    /// out-of-scope partitions — that the task's forbidden list matches,
    /// repo-relative and sorted. Non-empty means the branch is rejected
    /// before any ownership gate runs (Issue-30).
    pub(super) forbidden: Vec<String>,
}

impl ScopeGrant {
    /// The plan unchanged: no wave context, or nothing to widen.
    fn unchanged(plan: &WritePlan) -> Self {
        Self {
            plan: plan.clone(),
            granted: Vec::new(),
            whitespace_only: Vec::new(),
            out_of_scope: Vec::new(),
            roots: super::scope_roots::ScopeRoots::default(),
            unreported: Vec::new(),
            forbidden: Vec::new(),
        }
    }

    /// Resolve what this branch may keep beyond its declared targets.
    ///
    /// Candidates are the worktree's real changes outside the plan united with
    /// the envelope's out-of-plan entries. Returns the plan unchanged when
    /// there is no wave context, when nothing was changed outside it, or when
    /// every out-of-scope path is contested or whitespace-only — so the
    /// pre-existing behaviour is the default in every case that is not a clear
    /// grant. The whitespace-only, out-of-scope, unreported and forbidden
    /// sets are resolved with or without a wave: they are properties of the
    /// worktree and the plan, not of the wave.
    pub(super) fn resolve(
        plan: &WritePlan,
        result: &WorkflowV2Result,
        wave_claims: Option<&[WaveClaim]>,
        forbidden_paths: &ForbiddenPaths,
    ) -> Self {
        let scan = crate::write_coordinator::whitespace_only::worktree_changes(plan);
        let mut whitespace_only = scan.whitespace_only;
        let mut outside = scan.undeclared;
        // Issue-30: judged on the scan alone, before the envelope's entries
        // join `outside` — a reported path with no diff was not changed.
        let mut forbidden: Vec<String> = scan
            .declared
            .iter()
            .chain(outside.iter())
            .filter(|path| forbidden_paths.matches(path))
            .cloned()
            .collect();
        let mut reported: Vec<String> = Vec::new();
        for file in &result.files_changed {
            let Some(relative) = repo_relative(plan, &file.path) else {
                // A path the coordinator cannot name is never granted: it
                // meets the ownership check exactly as it does today.
                continue;
            };
            let relative = relative.as_str().to_string();
            reported.push(relative.clone());
            if path_is_planned(plan, &relative) {
                continue;
            }
            if whitespace_only_change(plan, &relative) {
                whitespace_only.push(relative);
                continue;
            }
            outside.push(relative);
        }
        whitespace_only.sort();
        whitespace_only.dedup();
        outside.sort();
        outside.dedup();
        let mut unreported: Vec<String> = scan
            .declared
            .into_iter()
            .chain(outside.iter().cloned())
            .filter(|path| !reported.contains(path))
            .collect();
        unreported.sort();
        unreported.dedup();
        // Issue-27: the ceiling. Partitioned AFTER `unreported` is counted —
        // an unlisted out-of-scope change is still under-reporting — and
        // BEFORE the wave contest, so an out-of-scope path is never a
        // candidate for a grant, contested or not.
        let roots = super::scope_roots::scope_roots(plan);
        let (outside, out_of_scope): (Vec<String>, Vec<String>) =
            outside.into_iter().partition(|path| roots.covers(path));
        // An out-of-scope change is dropped, so it is not a change the
        // forbidden verdict has to answer for; the in-scope remainder is.
        // And a forbidden path is never a candidate for a grant: the branch
        // is rejected, but the plan it is judged under must not declare it.
        forbidden.retain(|path| !out_of_scope.contains(path));
        forbidden.sort();
        forbidden.dedup();
        let outside: Vec<String> = outside
            .into_iter()
            .filter(|path| !forbidden.contains(path))
            .collect();
        let unchanged = Self {
            whitespace_only,
            out_of_scope,
            roots,
            unreported,
            forbidden,
            ..Self::unchanged(plan)
        };
        let Some(wave) = wave_claims else {
            return unchanged;
        };
        if outside.is_empty() {
            return unchanged;
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
            return unchanged;
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
            ..unchanged
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
            .is_some_and(|relative| path_is_planned(&self.plan, &relative.as_str()))
    }

    /// Whether `path` — as an envelope names it, by either root — is one of
    /// the whitespace-only paths this branch drops rather than judges.
    pub(super) fn is_whitespace_only(&self, path: &str) -> bool {
        repo_relative(&self.plan, path)
            .is_some_and(|relative| self.whitespace_only.iter().any(|p| *p == relative.as_str()))
    }

    /// Whether `path` — as an envelope names it, by either root — is one of
    /// the out-of-scope paths this branch drops rather than judges (Issue-27).
    pub(super) fn is_out_of_scope(&self, path: &str) -> bool {
        repo_relative(&self.plan, path)
            .is_some_and(|relative| self.out_of_scope.iter().any(|p| *p == relative.as_str()))
    }

    /// Restore every out-of-scope path in the worktree to the baseline — or
    /// remove it, when the agent created it — so capture never sees it, and
    /// return the paths actually dropped. Runs beside
    /// [`Self::drop_whitespace_only_changes`], before the ownership gates,
    /// for the same reason. Unlike a whitespace-only change, an out-of-scope
    /// one may be a created file, which `git checkout HEAD` cannot restore;
    /// `restore_or_remove` handles both. Only paths the worktree actually
    /// holds changed are dropped: an over-reported out-of-scope path with no
    /// diff has nothing to restore and must not be reported as if it had.
    pub(super) fn drop_out_of_scope_changes(&self) -> Vec<String> {
        if self.out_of_scope.is_empty() {
            return Vec::new();
        }
        let changed = crate::write_coordinator::patch_manifest::workspace_changed_paths(
            &self.plan.isolated_root,
        )
        .unwrap_or_default();
        let candidates: Vec<String> = self
            .out_of_scope
            .iter()
            .filter(|path| changed.contains(path))
            .cloned()
            .collect();
        crate::write_coordinator::whitespace_only::restore_or_remove(
            &self.plan.isolated_root,
            &candidates,
        )
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

#[cfg(test)]
impl ScopeGrant {
    /// [`Self::resolve`] for a branch whose tasks forbid nothing: the shape
    /// every pre-Issue-30 fixture exercises.
    pub(super) fn resolve_unforbidden(
        plan: &WritePlan,
        result: &WorkflowV2Result,
        wave_claims: Option<&[WaveClaim]>,
    ) -> Self {
        Self::resolve(plan, result, wave_claims, &ForbiddenPaths::default())
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
    ScopeGrant::resolve_unforbidden(plan, result, wave_claims).plan
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
fn path_is_planned(plan: &WritePlan, path: &str) -> bool {
    plan.target_files
        .iter()
        .chain(plan.target_dir_scopes.iter())
        .any(|owned| crate::v2::write_mode::paths_overlap(&owned.as_str(), path))
}

/// Whether the agent's change to `relative` is whitespace-only, by the one
/// comparison every whitespace decision uses (`write_coordinator::whitespace_only`).
fn whitespace_only_change(plan: &WritePlan, relative: &str) -> bool {
    crate::write_coordinator::whitespace_only::whitespace_only_change(
        &plan.canonical_root.join(relative),
        &plan.isolated_root.join(relative),
    )
}

#[cfg(test)]
#[path = "worktree_scope_grant_worktree_tests.rs"]
mod worktree_tests;

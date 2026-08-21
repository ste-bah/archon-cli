//! Grant a write item a file it did not declare, when nothing else claims it.
//!
//! # The failure this replaces
//!
//! `validate_changed_files_for_repository` rejects any changed file outside an
//! item's declared ownership, and the write coordinator then DISCARDS the whole
//! patch. Observed live on wf-3d7efd28, `implementation-wave-1-impl-tdl-020`:
//!
//! ```text
//! write branch 'impl-tdl-020' required write access to path(s) outside its
//! declared target_files: .../data_store/ahdm_test_support_a.rs. The change was
//! rejected and discarded, so this task cannot be completed until the declared
//! write scope includes those path(s)
//! ```
//!
//! The agent did the work, the work was correct, and an hour of it was thrown
//! away because a list written before the code was read did not name one file.
//! The remediation that followed did nothing cleverer than widen the list.
//!
//! # Why widening is safe here, and why prediction is not the answer
//!
//! The declared scope cannot be right in advance. A task's true write set is
//! discovered by reading the code, and files that do not exist yet cannot be
//! claimed at all — `ruah` lists exactly this among its stated non-guarantees:
//! "perfect prediction for brand-new files that do not exist when locks are
//! taken". Tonight proved both directions fail: TDL-040 declared 69 files and
//! collided, TDL-020 declared too few and was discarded.
//!
//! So the scope is treated as a claim to be extended, not a prophecy to be
//! graded. An extension is granted only when the file is unclaimed by every
//! OTHER item in the wave — the disjoint-ownership invariant that
//! `write_mode::plan` already enforces is preserved exactly, because a file no
//! one else owns cannot create a conflict by being granted here.
//!
//! Contested files are still refused. That case is a genuine ownership dispute
//! and belongs in remediation, where a human-legible gap explains which two
//! items want the same path.

use std::collections::BTreeSet;

/// The outcome of asking for a file outside the declared scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteScopeExtension {
    /// Nothing else claims the path: the item may keep the change.
    Granted { path: String },
    /// Another item in the wave owns the path. Refuse, and name the holder so
    /// the resulting gap says who to talk to rather than just "unsafe".
    Contested { path: String, holder: String },
}

impl WriteScopeExtension {
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted { .. })
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Granted { path } | Self::Contested { path, .. } => path,
        }
    }
}

/// One item's claim on the paths it owns, as the wave planner assigned them.
///
/// Serialisable because the claim list crosses the `call.options.extra`
/// boundary to reach the adapter that validates a result, exactly as
/// `target_ownership_scopes` already does.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WaveClaim {
    pub item_id: String,
    pub owned: BTreeSet<String>,
}

impl WaveClaim {
    pub fn new(item_id: impl Into<String>, owned: impl IntoIterator<Item = String>) -> Self {
        Self {
            item_id: item_id.into(),
            owned: owned.into_iter().collect(),
        }
    }

    fn claims(&self, path: &str) -> bool {
        self.owned
            .iter()
            .any(|owned| crate::v2::write_mode::paths_overlap(owned, path))
    }
}

/// Every claim in a planned wave, in the form the adapter compares against.
///
/// Both write paths that run items concurrently — worktree and coordinated —
/// build this, and they build it HERE so the two cannot drift into disagreeing
/// about what a wave claims. Targets and scopes are unioned: a directory scope
/// is as real a claim as a named file, and `paths_overlap` already treats it
/// as one.
pub fn wave_claims_for(wave: &crate::v2::write_mode::WorkflowV2WriteWave) -> Vec<WaveClaim> {
    wave.assignments
        .iter()
        .map(|assignment| {
            WaveClaim::new(
                assignment.item_id.clone(),
                assignment
                    .owned_targets
                    .iter()
                    .chain(assignment.owned_scopes.iter())
                    .cloned(),
            )
        })
        .collect()
}

/// Decide whether `item_id` may extend its scope to cover `path`.
///
/// `wave` is every claim in the SAME wave, including the requesting item's own
/// (which is skipped). Items in other waves are irrelevant: waves run
/// sequentially, so a path owned by a later wave is not concurrently held.
pub fn resolve_scope_extension(
    item_id: &str,
    path: &str,
    wave: &[WaveClaim],
) -> WriteScopeExtension {
    let holder = wave
        .iter()
        .find(|claim| claim.item_id != item_id && claim.claims(path));
    match holder {
        Some(claim) => WriteScopeExtension::Contested {
            path: path.to_string(),
            holder: claim.item_id.clone(),
        },
        None => WriteScopeExtension::Granted {
            path: path.to_string(),
        },
    }
}

/// Resolve every out-of-scope path an item changed, in one pass.
///
/// Returns the grants and the contests separately: a caller extends ownership
/// by the former and raises a gap for the latter. An empty `contested` means the
/// whole patch may be kept.
pub fn resolve_scope_extensions<'a>(
    item_id: &str,
    paths: impl IntoIterator<Item = &'a str>,
    wave: &[WaveClaim],
) -> (Vec<String>, Vec<WriteScopeExtension>) {
    let mut granted = Vec::new();
    let mut contested = Vec::new();
    for path in paths {
        match resolve_scope_extension(item_id, path, wave) {
            WriteScopeExtension::Granted { path } => granted.push(path),
            other => contested.push(other),
        }
    }
    granted.sort();
    granted.dedup();
    (granted, contested)
}

#[cfg(test)]
#[path = "write_scope_extension_tests.rs"]
mod tests;

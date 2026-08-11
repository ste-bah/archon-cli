//! The unattended pass: what a scheduler is allowed to ask for.
//!
//! Everything here exists so the scheduling seam in the binary has one function
//! to call and no policy decisions of its own to make. The seam owns *when*; the
//! rules about what an unattended pass may do live beside the code that enforces
//! them, where they can be tested without a timer.
//!
//! A scheduled pass differs from `/garden` in three ways, all of them
//! restrictions:
//!
//! 1. It holds the single-run lock, so it cannot overlap another pass — its own
//!    previous tick, a session start, or a `/garden` someone is watching.
//! 2. It runs under a work and time ceiling, and stops at a unit boundary.
//! 3. It cannot delete. Pruning becomes a [`super::RetirementCandidate`].
//!
//! It is never *more* capable than the manual command. That asymmetry is the
//! design: the thing nobody is watching should be able to do less, not more.

use std::path::Path;

use tracing::info;

use super::run_lock::{RunLockOutcome, run_lock_path, with_run_lock};
use super::{
    GardenBudget, GardenConfig, GardenReport, PrunePolicy, consolidate_with_policy,
    should_auto_consolidate,
};
use crate::access::MemoryTrait;
use crate::types::MemoryError;

/// What one pass is permitted to do.
///
/// A pair rather than an enum of modes, because the ceiling and the prune rule
/// are independent questions and a future caller may want a bounded pass that
/// still deletes, or an unbounded one that only proposes. Constructors cover the
/// two combinations that exist today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GardenRunPolicy {
    pub budget: GardenBudget,
    pub prune: PrunePolicy,
}

impl GardenRunPolicy {
    /// Unbounded, and permitted to delete.
    ///
    /// What `/garden` and session-start consolidation have always been. A person
    /// asked for it, is present, and reads the report; the reversibility comes
    /// from them, not from the code.
    pub const fn interactive() -> Self {
        Self {
            budget: GardenBudget::unbounded(),
            prune: PrunePolicy::Delete,
        }
    }

    /// Bounded, and structurally unable to delete.
    ///
    /// Two independent mechanisms enforce the second half, and both are
    /// deliberate. [`PrunePolicy::Propose`] diverts the decision before a
    /// deletion is attempted; `max_deletions: Some(0)` inside
    /// [`GardenBudget::scheduled`] refuses it if some later edit reaches
    /// `delete_memory` by a route the policy does not cover. One of them is
    /// redundant today. Which one is not knowable in advance, which is the
    /// argument for keeping both.
    pub fn scheduled(config: &GardenConfig) -> Self {
        Self {
            budget: GardenBudget::scheduled(
                config.scheduled_max_reversible_ops,
                config.scheduled_max_retirement_candidates,
                std::time::Duration::from_secs(config.scheduled_max_seconds),
            ),
            prune: PrunePolicy::Propose,
        }
    }
}

/// Outcome of asking for a scheduled pass.
///
/// Three outcomes rather than an `Option<GardenReport>`, because "another pass
/// holds the lock" and "the throttle has not elapsed" are different facts about
/// the system and only one of them is worth investigating if it persists.
#[derive(Debug)]
pub enum ScheduledRun {
    /// The pass ran to completion or to its budget. Read
    /// [`GardenReport::budget_exhausted`] to tell which.
    Ran(Box<GardenReport>),
    /// Another consolidation holds the run lock. Nothing was read or written.
    Declined,
    /// The interval since the last pass has not elapsed. Nothing ran.
    TooRecent,
}

impl ScheduledRun {
    /// The report, if a pass actually ran.
    pub fn report(self) -> Option<GardenReport> {
        match self {
            Self::Ran(report) => Some(*report),
            Self::Declined | Self::TooRecent => None,
        }
    }
}

/// Whether the scheduler is switched on at all.
///
/// A predicate rather than an `if` at the call site, so the default-off
/// guarantee is a testable property of this crate rather than a line in the
/// binary that a refactor can move.
pub fn should_run_scheduled(config: &GardenConfig) -> bool {
    config.scheduled_consolidation
}

/// Run one scheduled consolidation pass over `graph`, holding the single-run
/// lock for the store whose coordination files live in `data_dir`.
///
/// Synchronous and blocking: it performs database work and, on every process but
/// the one that owns the store, does so over TCP. Callers on an async runtime
/// must put it on a blocking thread.
///
/// # What an interruption leaves behind
///
/// Nothing that needs cleaning up. The lock is an OS advisory lock, so a killed
/// process releases it without help. The pass writes only whole units — one
/// importance delta, one supersession — and takes its budget decision before
/// each rather than during, so the store at any interruption point is the store
/// a pass with fewer candidates would have produced. The run timestamp is
/// written last but written unconditionally; a pass killed before it simply has
/// its span billed by the next run, which is the same behaviour as a pass that
/// never started.
///
/// # Errors
///
/// A lock file that cannot be opened, or a store error from the pass itself.
/// Contention is [`ScheduledRun::Declined`], not an error.
pub fn run_scheduled_consolidation(
    graph: &dyn MemoryTrait,
    config: &GardenConfig,
    data_dir: &Path,
    run_id: &str,
) -> Result<ScheduledRun, MemoryError> {
    let lock_path = run_lock_path(data_dir);
    let outcome = with_run_lock(&lock_path, || {
        // Inside the lock, not before it. Checked outside, two ticks could both
        // read a stale timestamp, both decide the interval had elapsed, and both
        // queue up behind the lock to run back to back.
        if !should_auto_consolidate(graph, config.scheduled_interval_hours)? {
            return Ok(ScheduledRun::TooRecent);
        }
        info!(
            run_id,
            interval_hours = config.scheduled_interval_hours,
            "garden: starting scheduled consolidation"
        );
        let report =
            consolidate_with_policy(graph, config, run_id, GardenRunPolicy::scheduled(config))?;
        info!(
            run_id,
            decayed = report.importance_decayed,
            merged = report.duplicates_merged + report.fragments_merged,
            proposed_for_retirement = report.retirement_candidates.len(),
            deleted = report.stale_pruned + report.overflow_pruned,
            budget_exhausted = report.budget_exhausted,
            "garden: scheduled consolidation complete"
        );
        Ok(ScheduledRun::Ran(Box::new(report)))
    })?;

    match outcome {
        RunLockOutcome::Ran(result) => result,
        RunLockOutcome::Busy => {
            super::run_lock::log_declined(&lock_path);
            Ok(ScheduledRun::Declined)
        }
    }
}

#[cfg(test)]
#[path = "scheduling_tests.rs"]
mod scheduling_tests;

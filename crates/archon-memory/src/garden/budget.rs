//! The work budget: a hard ceiling on what one consolidation pass may change.
//!
//! Consolidation was unbounded. On a small store that is invisible; on a store
//! that has been accumulating for months, one pass can decay every row, merge
//! dozens of pairs and delete hundreds, each as a separate round trip — and
//! every Archon process after the first reaches memory over TCP, so "a round
//! trip" is a socket, not a function call. An unattended job doing that at 3am
//! with nobody watching is the shape of the problem this bounds.
//!
//! # What "interrupted mid-way" has to mean
//!
//! The budget's real job is not to be fast. It is to make the answer to "what if
//! this stops half-way" boring.
//!
//! Every unit of work consolidation performs is independent of every other:
//! one importance delta, one supersession, one deletion. None of them is half of
//! a larger invariant — there is no state that is only valid once N of them have
//! all happened. So the store after k units is exactly the store a pass with
//! only k candidates would have produced, and the remaining candidates are still
//! candidates on the next run.
//!
//! That is only true if the budget is consulted BETWEEN units and never inside
//! one. [`BudgetLedger::take_reversible`] and [`BudgetLedger::take_deletion`]
//! are therefore called immediately before a unit begins, and a refusal breaks
//! the loop rather than skipping to the next candidate. A budget checked in the
//! middle of a merge would leave the survivor tagged and the loser unmarked,
//! which is exactly the half-consolidated state this exists to prevent.
//!
//! # Exhaustion is reported, not hidden
//!
//! A pass that stops early still records its run timestamp, and that is
//! deliberate rather than an oversight. Importance decay bills the shorter of
//! "since last access" and "since the previous run"; a pass that skipped
//! recording would leave the next one billing the same span twice, which is the
//! compounding-decay bug documented on `phase_importance_decay`. Losing a little
//! pruning to the next run is recoverable. Double-charging decay silently
//! deletes memories early, and is not.
//!
//! So exhaustion surfaces as `GardenReport::budget_exhausted` and a log line,
//! and the leftover work waits for the next scheduled pass. Stale memories are
//! still stale tomorrow; duplicates are still duplicates.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Ceiling on one consolidation pass.
///
/// Each field is `None` for "unbounded", which is what the interactive paths
/// use: `/garden` and session start are started by a person, are visible while
/// they run, and have always been unbounded. Changing that under them would be a
/// behaviour change nobody asked for. The bound is for the unattended path,
/// where nobody is watching and nobody chose the moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GardenBudget {
    /// Most reversible mutations: importance deltas and supersessions.
    ///
    /// Counted together rather than per phase because the cost being bounded is
    /// round trips to the store, and the store cannot tell them apart.
    pub max_reversible_ops: Option<usize>,
    /// Most rows this pass may destroy.
    ///
    /// `Some(0)` is the meaningful setting for an unattended pass: it cannot
    /// delete at all. That is enforced separately and earlier by
    /// [`super::PrunePolicy`]; this is the arithmetic backstop, so a future edit
    /// that reaches a deletion by some other route still cannot exceed it.
    pub max_deletions: Option<usize>,
    /// Most retirement candidates one pass may propose.
    ///
    /// Counted apart from the two above because a proposal writes nothing to the
    /// memory store; what it bounds is the size of the review pile. A store ten
    /// thousand rows over its cap would otherwise hand a reviewer ten thousand
    /// decisions, which is the same as handing them none.
    pub max_retirement_candidates: Option<usize>,
    /// Wall-clock ceiling, checked at unit boundaries.
    ///
    /// Not a timeout in the usual sense: nothing is cancelled and nothing is
    /// rolled back. It stops the pass from starting further work, which is the
    /// only kind of stopping that is safe here.
    pub max_duration: Option<Duration>,
}

impl GardenBudget {
    /// No ceiling at all — today's behaviour, preserved for the paths a person
    /// started and is watching.
    pub const fn unbounded() -> Self {
        Self {
            max_reversible_ops: None,
            max_deletions: None,
            max_retirement_candidates: None,
            max_duration: None,
        }
    }

    /// The ceiling an unattended pass runs under.
    ///
    /// `max_deletions` is fixed at zero and is not a parameter. A scheduled pass
    /// proposes retirements for review; it does not perform them. Making that a
    /// caller's choice would put "may the background job delete your memories"
    /// one argument away from being true.
    pub const fn scheduled(
        max_reversible_ops: usize,
        max_retirement_candidates: usize,
        max_duration: Duration,
    ) -> Self {
        Self {
            max_reversible_ops: Some(max_reversible_ops),
            max_deletions: Some(0),
            max_retirement_candidates: Some(max_retirement_candidates),
            max_duration: Some(max_duration),
        }
    }
}

impl Default for GardenBudget {
    fn default() -> Self {
        Self::unbounded()
    }
}

/// Running account of what a pass has spent against its [`GardenBudget`].
///
/// Threaded through the phases by `&mut` rather than consulted from a shared
/// counter, so the ceiling is per pass and two passes cannot pool one allowance.
#[derive(Debug)]
pub struct BudgetLedger {
    budget: GardenBudget,
    started: Instant,
    reversible: usize,
    deletions: usize,
    proposals: usize,
    exhausted: bool,
}

impl BudgetLedger {
    pub fn new(budget: GardenBudget) -> Self {
        Self {
            budget,
            started: Instant::now(),
            reversible: 0,
            deletions: 0,
            proposals: 0,
            exhausted: false,
        }
    }

    /// Claim one reversible mutation, or refuse and mark the pass exhausted.
    ///
    /// Callers MUST stop on `false` rather than continue to the next candidate.
    /// Skipping onward would let a pass keep scanning after its ceiling, and the
    /// wall-clock limit would then bound nothing.
    #[must_use]
    pub fn take_reversible(&mut self) -> bool {
        if self.out_of_time() {
            return false;
        }
        match self.budget.max_reversible_ops {
            Some(max) if self.reversible >= max => {
                self.exhausted = true;
                false
            }
            _ => {
                self.reversible += 1;
                true
            }
        }
    }

    /// Claim one destructive deletion, or refuse and mark the pass exhausted.
    ///
    /// Kept separate from [`Self::take_reversible`] because the two are not
    /// interchangeable: spending the reversible allowance costs a little tidying
    /// on the next run, and spending this one costs a memory that is not coming
    /// back. A single pooled counter would let a merge-heavy pass consume the
    /// allowance that was meant to bound deletion, or the reverse.
    #[must_use]
    pub fn take_deletion(&mut self) -> bool {
        if self.out_of_time() {
            return false;
        }
        match self.budget.max_deletions {
            Some(max) if self.deletions >= max => {
                self.exhausted = true;
                false
            }
            _ => {
                self.deletions += 1;
                true
            }
        }
    }

    /// Claim room for one retirement candidate, or refuse and mark the pass
    /// exhausted.
    ///
    /// Refusing costs nothing but a smaller review pile: the memory is untouched
    /// either way, and the next pass re-derives the same candidates from the
    /// same store. That is the whole reason proposing is safe to bound and
    /// deleting is not.
    #[must_use]
    pub fn take_proposal(&mut self) -> bool {
        if self.out_of_time() {
            return false;
        }
        match self.budget.max_retirement_candidates {
            Some(max) if self.proposals >= max => {
                self.exhausted = true;
                false
            }
            _ => {
                self.proposals += 1;
                true
            }
        }
    }

    /// Whether the pass stopped short of finishing its candidates.
    ///
    /// Read into the report so a persistently over-budget store is visible.
    /// Without it, a pass that bailed after ten of four hundred merges reports
    /// the same shape of success as one that had ten to do.
    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub fn spent_reversible(&self) -> usize {
        self.reversible
    }

    pub fn spent_deletions(&self) -> usize {
        self.deletions
    }

    pub fn spent_proposals(&self) -> usize {
        self.proposals
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Whether the wall-clock ceiling has passed, latching exhaustion if so.
    ///
    /// Latched rather than recomputed by callers, because time only moves one
    /// way: once a pass is over its deadline every subsequent unit is refused,
    /// and the report must say why even though no count was exceeded.
    fn out_of_time(&mut self) -> bool {
        let Some(max) = self.budget.max_duration else {
            return false;
        };
        if self.started.elapsed() >= max {
            self.exhausted = true;
            return true;
        }
        false
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod budget_tests;

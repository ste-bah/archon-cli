//! The staged acceptance freeze's own time budget and its resumable
//! outcome (Issue 255).
//!
//! The host kills `freeze-acceptance` at its catalog wall clock and the
//! work in flight is lost. The freeze therefore keeps a deadline of its own,
//! read from that same catalog entry (never a second copy of the number)
//! less [`FREEZE_SAFETY_MARGIN_SECS`] for teardown and reporting. Before each
//! probe check it asks the budget how long the check may run; when too
//! little is left it stops, with every finished probe result and the
//! judge's verdicts already saved, and reports [`FreezeIncomplete`]: an
//! operational outcome whose text starts with
//! [`FREEZE_INCOMPLETE_RESUMABLE`]. A retry of the same freeze resumes from
//! the saved results.
//!
//! The staged freeze ends it with the host's operational contract
//! (`workflow_host_command_operational`): the reason on stderr, then the
//! progress line, then exit status `EXIT_INCOMPLETE_RESUMABLE` (75), which
//! the executor retries while progress grows and otherwise pauses the run.
//! While it runs it writes a progress line after every saved probe verdict
//! and after the saved judge verdicts, so even a freeze the host kills
//! reports how far it got. Everything it saves lives outside the call's
//! staging directory, which the executor clears before each retry. An
//! unstaged freeze has no host deadline and never produces it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
use std::time::{Duration, Instant};

use archon_workflow::acceptance_scratch::CheckAllowance;

/// Kept back from the catalog wall clock: the last check's integrity
/// audit, the observation's teardown and live-root audit, and the report.
pub(crate) const FREEZE_SAFETY_MARGIN_SECS: u64 = 600;
/// No check starts with less than this left: it could not finish.
pub(crate) const MIN_CHECK_WINDOW_SECS: u64 = 120;
/// No observation starts with less than this beyond one check's window:
/// preparing a scratch slot alone takes most of a minute.
pub(crate) const OBSERVATION_SETUP_SECS: u64 = 120;
/// How every [`FreezeIncomplete`] text starts, for whoever must tell an
/// incomplete, resumable freeze from a failed one.
pub(crate) const FREEZE_INCOMPLETE_RESUMABLE: &str = "operational: freeze incomplete, resumable";

/// Under the project root: what a staged freeze saves for its retry that
/// has no build cache to live in (judge verdicts; a hermetic probe's
/// verdicts). Never copied into a hermetic probe copy, nor digested as its
/// project data.
pub(crate) const FREEZE_CACHE_DIR: &str = ".archon/freeze-cache";

/// The clock a budget reads; injected by tests.
pub(crate) type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// A freeze's remaining time. Unlimited unless the freeze runs under a
/// host wall clock.
#[derive(Clone)]
pub(crate) struct FreezeBudget {
    deadline: Option<Instant>,
    clock: Clock,
    /// The wall clock it was derived from, for the report.
    outer_secs: u64,
}

impl FreezeBudget {
    pub(crate) fn unlimited() -> Self {
        Self {
            deadline: None,
            clock: Arc::new(Instant::now),
            outer_secs: 0,
        }
    }

    /// A budget of `outer_secs` from now on `clock`, less the margin.
    pub(crate) fn within(outer_secs: u64, clock: Clock) -> Self {
        let usable = outer_secs.saturating_sub(FREEZE_SAFETY_MARGIN_SECS);
        Self {
            deadline: Some(clock() + Duration::from_secs(usable)),
            clock,
            outer_secs,
        }
    }

    /// The budget of the host command `command_id`, from now: its catalog
    /// wall clock (`workflow_host_command_catalog`), read and never
    /// restated. Unlimited when the catalog has no such command.
    pub(crate) fn for_host_command(command_id: &str) -> Self {
        let timeout =
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("")
                .ok()
                .and_then(|catalog| catalog.capabilities.get(command_id).map(|c| c.timeout_secs));
        match timeout {
            Some(secs) => Self::within(secs, Arc::new(Instant::now)),
            None => Self::unlimited(),
        }
    }

    fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since((self.clock)()))
    }

    /// How long the next check may run under a per-check `cap_secs`.
    pub(crate) fn allowance(&self, cap_secs: u64) -> CheckAllowance {
        let Some(remaining) = self.remaining() else {
            return CheckAllowance::Run {
                timeout_secs: cap_secs,
                cut: false,
            };
        };
        let left = remaining.as_secs();
        if left < MIN_CHECK_WINDOW_SECS {
            return CheckAllowance::Defer;
        }
        CheckAllowance::Run {
            timeout_secs: cap_secs.min(left),
            cut: left < cap_secs,
        }
    }

    /// Whether a new observation (or copy) may still be prepared.
    pub(crate) fn allows_observation(&self) -> bool {
        self.remaining()
            .is_none_or(|left| left.as_secs() >= MIN_CHECK_WINDOW_SECS + OBSERVATION_SETUP_SECS)
    }

    fn describe(&self) -> String {
        format!(
            "the freeze stops starting checks {FREEZE_SAFETY_MARGIN_SECS}s before its {}s host wall clock",
            self.outer_secs
        )
    }
}

/// What a freeze has saved for its retry, this attempt and earlier ones:
/// the units the host's progress line counts.
#[derive(Debug, Default)]
pub(crate) struct FreezeProgress {
    saved: AtomicU64,
    reused: AtomicU64,
    judged: AtomicU64,
    /// Whether each saved unit is reported on stderr (a staged freeze).
    report: bool,
}

impl FreezeProgress {
    /// Probe verdicts or judge verdicts saved by this attempt; reported.
    pub(crate) fn saved(&self, judge: bool) {
        let counter = if judge { &self.judged } else { &self.saved };
        counter.fetch_add(1, SeqCst);
        self.report_line();
    }

    /// A unit an earlier attempt saved, found again.
    pub(crate) fn reused(&self, judge: bool) {
        let counter = if judge { &self.judged } else { &self.reused };
        counter.fetch_add(1, SeqCst);
        // Reported too: after a kill, the last line must count every unit on
        // disk, or the executor reads too little progress and pauses early.
        self.report_line();
    }

    /// Exercise counter rollback in progress tests.
    #[cfg(test)]
    pub(crate) fn withdrawn(&self) {
        let _ = (self.saved).fetch_update(SeqCst, SeqCst, |n| n.checked_sub(1));
    }

    /// Every unit saved for this call so far, by any attempt.
    pub(crate) fn total(&self) -> u64 {
        self.saved.load(SeqCst) + self.reused.load(SeqCst) + self.judged.load(SeqCst)
    }

    /// The host's progress line for [`Self::total`].
    pub(crate) fn line(&self) -> String {
        crate::command::workflow_host_command_operational::progress_line(self.total())
    }

    fn report_line(&self) {
        if self.report {
            eprintln!("{}", self.line());
        }
    }
}

/// The freeze stopped for its time budget, with its progress saved.
#[derive(Debug)]
pub(crate) struct FreezeIncomplete {
    budget: String,
    saved: u64,
    reused: u64,
    deferred: Vec<String>,
    progress: String,
}

impl FreezeIncomplete {
    pub(crate) fn new(
        budget: &FreezeBudget,
        progress: &FreezeProgress,
        deferred: Vec<String>,
    ) -> Self {
        Self {
            budget: budget.describe(),
            saved: progress.saved.load(SeqCst),
            reused: progress.reused.load(SeqCst),
            deferred,
            progress: progress.line(),
        }
    }

    /// What the staged freeze writes to stderr before it exits
    /// `EXIT_INCOMPLETE_RESUMABLE`: the reason, then the progress line,
    /// last, as the host reads it.
    pub(crate) fn report(&self) -> String {
        format!("{self}\n{}", self.progress)
    }

    /// The incomplete freeze `error` carries, if any.
    pub(crate) fn caused(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for FreezeIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{FREEZE_INCOMPLETE_RESUMABLE}: {}, and too little time was left; {} probe result(s) were saved by this attempt and {} reused from earlier ones, and the judge's verdicts are saved; {} check(s) still have no probe verdict ({}). Retry the freeze with the same candidate (resume the run): it continues from the saved results",
            self.budget,
            self.saved,
            self.reused,
            self.deferred.len(),
            self.deferred.join(", ")
        )
    }
}

impl std::error::Error for FreezeIncomplete {}

/// How a freeze may span attempts: its budget, and whether it saves probe
/// results and judge verdicts for a retry. [`FreezeResume::none`] is the
/// unstaged freeze's: unlimited, nothing saved.
#[derive(Clone)]
pub(crate) struct FreezeResume {
    pub(crate) budget: FreezeBudget,
    pub(crate) persist: bool,
    pub(crate) progress: Arc<FreezeProgress>,
}

impl FreezeResume {
    pub(crate) fn none() -> Self {
        Self::saving(FreezeBudget::unlimited(), false)
    }

    /// The staged `command_id` freeze: its host budget, results saved and
    /// reported as they are.
    pub(crate) fn staged(command_id: &str) -> Self {
        let mut resume = Self::saving(FreezeBudget::for_host_command(command_id), true);
        resume.progress = Arc::new(FreezeProgress {
            report: true,
            ..FreezeProgress::default()
        });
        resume
    }

    /// Under `budget`, saving results when `persist`; reporting nothing.
    pub(crate) fn saving(budget: FreezeBudget, persist: bool) -> Self {
        Self {
            budget,
            persist,
            progress: Arc::new(FreezeProgress::default()),
        }
    }
}

#[cfg(test)]
#[path = "workflow_freeze_budget_tests.rs"]
mod tests;

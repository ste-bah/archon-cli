//! The regression search itself (Batch J, J2), split from
//! `acceptance_regression` for size: one search per failure signature,
//! over the run's probe points (the base, every landing in order, the tip),
//! sharing observations and a budget (`acceptance_regression_drive`
//! schedules them).
//!
//! Batch J2: a SIGNATURE-AWARE BISECTION for the first landing where the
//! check fails the way it fails now. The rule, per member at a point:
//!
//! - BAD: it fails with the same failure signature as at the tip
//!   ([`Verdict::fails_as`]);
//! - GOOD: it passes, or fails with a DIFFERENT signature -- the feature
//!   is absent at the base, say, so a check the run itself built is good
//!   there;
//! - no verdict: the observation failed, or the landing did not build
//!   (`could not compile`): skipped, never read either way.
//!
//! The base is observed first, for every member at once. From the first
//! member GOOD there (the lead), the search bisects the run's landings
//! between the last point known GOOD and the first known BAD: about log2
//! of the landings. The members' owners' landings are only a tie-break for
//! the midpoint -- and, should every member already be BAD at the base,
//! the first guesses at a GOOD point, newest first -- never a linear walk.
//! The driver confirms every member at the settled bisection: GOOD at the
//! landing before the break and BAD at the break, or it is searched on its
//! own. A history that turns BAD, GOOD and BAD again yields one of its
//! breaks, not necessarily the last.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AcceptanceRegressionV1, FailingCheck, RegressionSearchV1, SearchBudget, Timeline, Verdict,
};

/// A member's reading at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// Passes, or fails another way.
    Good,
    /// Fails as at the tip.
    Bad,
}

/// How the search reads `verdict` for `check`; `None` is no verdict.
pub(super) fn mark(check: &FailingCheck, verdict: &Verdict) -> Option<Mark> {
    match verdict {
        _ if verdict.passed() => Some(Mark::Good),
        Verdict::Failed(seen) if seen.contains("could not compile") => None,
        _ if verdict.fails_as(&check.signature) => Some(Mark::Bad),
        _ => Some(Mark::Good),
    }
}

pub(super) enum Phase {
    /// Waiting for the base's verdicts.
    Base,
    /// Every member was BAD at the base: its owners' landings, newest
    /// first, are tried for a GOOD point (`order[next..]` remain).
    Guess {
        order: Vec<usize>,
        next: usize,
    },
    /// Its lead is GOOD at `lo` and BAD at `hi`.
    Bisect {
        lo: usize,
        hi: usize,
    },
    /// Pinned (`settled` says where).
    Found(AcceptanceRegressionV1),
    Stopped(RegressionSearchV1),
}

pub(super) struct Search<'a> {
    /// The member leading the search (the one bisected and confirmed
    /// against), and every member sharing its outcome, the lead first.
    pub(super) probe: &'a FailingCheck,
    pub(super) members: Vec<&'a FailingCheck>,
    pub(super) phase: Phase,
    /// Every reading, by point and member id; the tip is BAD for all.
    seen: BTreeMap<usize, BTreeMap<String, Mark>>,
    /// Points probed with no verdict for the members asked.
    unknown: BTreeSet<usize>,
    /// Its members' owners' landings.
    owned: BTreeSet<usize>,
    /// Its members were checked against its settled bisection (`confirm`).
    pub(super) confirmed: bool,
    /// Where its bisection settled: its lead GOOD at `.0`, BAD at `.1`.
    pub(super) settled: Option<(usize, usize)>,
    /// It found a GOOD point and bisected for its lead alone; until then
    /// every member was observed wherever it was.
    pub(super) led: bool,
}

impl<'a> Search<'a> {
    /// A search of its own for `members` (the first leads), seeded by
    /// [`Search::start`].
    pub(super) fn new(timeline: &Timeline, members: Vec<&'a FailingCheck>) -> Self {
        let last = timeline.points.len() - 1;
        let at_tip = members.iter().map(|m| (m.id.clone(), Mark::Bad)).collect();
        Search {
            probe: members[0],
            members,
            phase: Phase::Base,
            seen: BTreeMap::from([(last, at_tip)]),
            unknown: BTreeSet::new(),
            owned: BTreeSet::new(),
            confirmed: false,
            settled: None,
            led: false,
        }
    }

    /// Note every member's owners' landings; start at the base.
    pub(super) fn start(&mut self, timeline: &Timeline) {
        let owners: BTreeSet<&String> = (self.members.iter())
            .flat_map(|check| check.owners.iter())
            .collect();
        let last = timeline.points.len() - 1;
        self.owned = (1..=timeline.landings.len())
            .filter(|p| *p != last)
            .filter(|p| timeline.tasks[p - 1].iter().any(|t| owners.contains(t)))
            .collect();
        self.phase = Phase::Base;
        if last == 0 {
            self.stop(timeline, "the run landed nothing before the tip".into());
        }
    }

    pub(super) fn open(&self) -> bool {
        matches!(
            self.phase,
            Phase::Base | Phase::Guess { .. } | Phase::Bisect { .. }
        )
    }

    /// The point this search wants observed next and the members to
    /// observe there: all of them at the base and at a guess, the lead
    /// while bisecting.
    pub(super) fn wants(&self) -> Option<(usize, Vec<&'a FailingCheck>)> {
        match &self.phase {
            Phase::Base => Some((0, self.members.clone())),
            Phase::Guess { order, next } => {
                order.get(*next).map(|point| (*point, self.members.clone()))
            }
            Phase::Bisect { lo, hi } => {
                let middle = (lo + hi) as f64 / 2.0;
                (lo + 1..*hi)
                    .filter(|point| !self.unknown.contains(point))
                    .min_by(|a, b| {
                        let (da, db) = ((*a as f64 - middle).abs(), (*b as f64 - middle).abs());
                        // Equally central: an owner's landing first.
                        (da.total_cmp(&db))
                            .then(self.owned.contains(b).cmp(&self.owned.contains(a)))
                            .then(a.cmp(b))
                    })
                    .map(|point| (point, vec![self.probe]))
            }
            _ => None,
        }
    }

    fn reading(&self, point: usize, id: &str) -> Option<Mark> {
        self.seen.get(&point).and_then(|at| at.get(id)).copied()
    }

    /// Lead with the first member (by id) GOOD at `point`, bisecting up to
    /// the first point it is known BAD at.
    fn lead_from(&mut self, timeline: &Timeline, point: usize) -> bool {
        let Some(at) =
            (self.members.iter()).position(|m| self.reading(point, &m.id) == Some(Mark::Good))
        else {
            return false;
        };
        let lead = self.members.remove(at);
        self.members.insert(0, lead);
        self.probe = lead;
        let hi = (self.seen.keys().copied())
            .filter(|p| *p > point)
            .find(|p| self.reading(*p, &lead.id) == Some(Mark::Bad))
            .unwrap_or(timeline.points.len() - 1);
        self.phase = Phase::Bisect { lo: point, hi };
        self.led = true;
        true
    }

    /// Take the verdicts at `point` (of the members asked; a missing one
    /// had no verdict there).
    pub(super) fn take(
        &mut self,
        timeline: &Timeline,
        point: usize,
        verdicts: &BTreeMap<String, Verdict>,
    ) {
        let mine: BTreeMap<String, Mark> = (self.members.iter())
            .filter_map(|m| Some((m.id.clone(), mark(m, verdicts.get(&m.id)?)?)))
            .collect();
        let asked = match &self.phase {
            Phase::Bisect { .. } => mine.contains_key(&self.probe.id),
            _ => !mine.is_empty(),
        };
        if !asked {
            self.unknown.insert(point);
        }
        self.seen.entry(point).or_default().extend(mine);
        match &mut self.phase {
            Phase::Base => {
                if !self.lead_from(timeline, point) {
                    // Already BAD (or unobservable) at the base: guess.
                    let order = self.owned.iter().rev().copied().collect();
                    self.phase = Phase::Guess { order, next: 0 };
                }
            }
            Phase::Guess { next, .. } => {
                *next += 1;
                self.lead_from(timeline, point);
            }
            Phase::Bisect { lo, hi } => {
                match self.seen.get(&point).and_then(|at| at.get(&self.probe.id)) {
                    Some(Mark::Good) => *lo = point,
                    Some(Mark::Bad) => *hi = point,
                    None => {}
                }
            }
            _ => return,
        }
        self.advance(timeline);
    }

    /// Points it (any member) was observed at with a verdict.
    pub(super) fn observed(&self) -> usize {
        self.seen.len()
    }

    pub(super) fn stop(&mut self, timeline: &Timeline, note: String) {
        self.phase = Phase::Stopped(RegressionSearchV1 {
            never_held: false,
            observed: self.observed(),
            points: timeline.points.len(),
            note,
            probed_as: None,
        });
    }

    /// Settle a bisection with nothing left to probe between its bounds.
    fn settle(&mut self, timeline: &Timeline, lo: usize, hi: usize) {
        self.settled = Some((lo, hi));
        if hi - lo > 1 {
            // The break is one of these landings: name them all.
            let suspects: Vec<String> = (lo + 1..=hi)
                .filter_map(|point| {
                    let landing = timeline.landing_at(point)?;
                    let tasks: Vec<String> = timeline.tasks[point - 1].iter().cloned().collect();
                    Some(format!(
                        "{} ({}, {})",
                        timeline.short(point),
                        landing.stage,
                        tasks.join("+")
                    ))
                })
                .collect();
            let note = format!(
                "it did not fail this way at {} and does at {}; the landing(s) between could not be observed or did not build, so the break is one of: {}",
                timeline.short(lo),
                timeline.short(hi),
                suspects.join("; ")
            );
            return self.stop(timeline, note);
        }
        let Some(landing) = timeline.landing_at(hi) else {
            let note = format!(
                "it did not fail this way at {}, the run's last landing, and does at the tip {}, a commit the run did not land: no task of the run broke it",
                timeline.short(lo),
                timeline.short(hi)
            );
            return self.stop(timeline, note);
        };
        let tasks: Vec<String> = timeline.tasks[hi - 1].iter().cloned().collect();
        if tasks.is_empty() {
            // Pinned, but to no task: say where, never route to nobody.
            let note = format!(
                "it did not fail this way at {} and first did at run landing {} ({}), which changed {}, but no task of that landing could be read from the run's records",
                timeline.short(lo),
                landing.commit,
                landing.stage,
                landing.paths.join(", ")
            );
            return self.stop(timeline, note);
        }
        self.phase = Phase::Found(AcceptanceRegressionV1 {
            held_at: timeline.points[lo].clone(),
            landing_commit: landing.commit.clone(),
            landing_stage: landing.stage.clone(),
            tasks,
            changed_files: landing.paths.clone(),
            probed_as: None,
        });
    }

    /// Close a search that has nothing left to probe.
    pub(super) fn advance(&mut self, timeline: &Timeline) {
        if self.wants().is_some() {
            return;
        }
        match self.phase {
            Phase::Bisect { lo, hi } => self.settle(timeline, lo, hi),
            Phase::Guess { .. } => {
                let base = if self.unknown.contains(&0) {
                    format!("the run base {} could not be observed", timeline.short(0))
                } else {
                    format!(
                        "it already fails this way at the run base {}",
                        timeline.short(0)
                    )
                };
                let note = format!(
                    "{base}, and at each of its owners' {} landing(s) tried: no point where it did not was found, so no landing of the run is shown to have broken it",
                    self.owned.len()
                );
                self.stop(timeline, note);
            }
            _ => {}
        }
    }

    pub(super) fn cut_short(&mut self, timeline: &Timeline, budget: SearchBudget) {
        let spent = format!(
            "the regression search budget ({} observations, {} min) ran out",
            budget.observations,
            budget.time.as_secs() / 60
        );
        let state = match self.phase {
            Phase::Bisect { lo, hi } => format!(
                "; it did not fail this way at {} and does at {}: one of the landings after the first, up to the second, broke it",
                timeline.short(lo),
                timeline.short(hi)
            ),
            _ => format!(
                " before a point where it did not fail this way was found ({} of the run's {} point(s) observed)",
                self.observed(),
                timeline.points.len()
            ),
        };
        self.stop(timeline, format!("{spent}{state}"));
    }
}

//! The regression search itself (Batch J), split from
//! `acceptance_regression` for size: one search per failure signature,
//! over the run's probe points, sharing observations and a budget.
//!
//! A search SEEKS a point where its check held -- the base, then its
//! owners' landings latest first (the primary points), then every other
//! landing latest first (the sweep) -- and, from the first it holds at,
//! BISECTS up to the nearest point it is known to fail at (the tip, where
//! the round just saw it fail, or a point the seek saw it fail at). Each
//! pass observes ONE commit, for every search wanting it at once: the one
//! serving the most failing checks (a signature shared by three checks
//! first), then a bisection's, then the earliest. A sweep is scheduled only
//! when no search is bisecting or on its primary points, so a check that
//! never held cannot starve one about to be pinned. A point with no verdict
//! (the observation failed there) is skipped, never read as a pass or a
//! fail. Whatever the budget leaves open is closed with a note saying so.
//!
//! Checks failing identically share their probe's search, but a pinned
//! landing is shared only with a member seen to hold where the probe held
//! and to fail at that landing; any other member is searched on its own.
//! A member never inherits `never_held`: that is proven only by probing.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AcceptanceRegressionV1, Attribution, FailingCheck, Observations, RegressionSearchV1,
    SearchBudget, Timeline,
};

enum Phase {
    /// Looking for a point it held at: `order[next..]` remain, the first
    /// `primary` of `order` being the primary points.
    Seek {
        order: Vec<usize>,
        next: usize,
        primary: usize,
    },
    /// It held at `lo` and fails at `hi`.
    Bisect {
        lo: usize,
        hi: usize,
    },
    /// Pinned: it held at `lo` and the landing at `hi` broke it.
    Found(AcceptanceRegressionV1, usize, usize),
    Stopped(RegressionSearchV1),
}

struct Search<'a> {
    /// The check probed, and those sharing its outcome.
    probe: &'a FailingCheck,
    members: Vec<&'a FailingCheck>,
    phase: Phase,
    fails: BTreeSet<usize>,
    unknown: BTreeSet<usize>,
    observed: BTreeSet<usize>,
    /// Its members were checked against its pinned landing (`confirm`).
    confirmed: bool,
}

impl Search<'_> {
    fn open(&self) -> bool {
        matches!(self.phase, Phase::Seek { .. } | Phase::Bisect { .. })
    }

    /// The point this search wants observed next, and whether it is a
    /// sweep point.
    fn wants(&self) -> Option<(usize, bool)> {
        match &self.phase {
            Phase::Seek {
                order,
                next,
                primary,
            } => order.get(*next).map(|point| (*point, *next >= *primary)),
            Phase::Bisect { lo, hi } => {
                let middle = (lo + hi) as f64 / 2.0;
                (lo + 1..*hi)
                    .filter(|point| !self.unknown.contains(point))
                    .min_by(|a, b| {
                        let (da, db) = ((*a as f64 - middle).abs(), (*b as f64 - middle).abs());
                        da.total_cmp(&db).then(a.cmp(b))
                    })
                    .map(|point| (point, false))
            }
            _ => None,
        }
    }

    fn stop(&mut self, timeline: &Timeline, note: String) {
        self.phase = Phase::Stopped(RegressionSearchV1 {
            never_held: false,
            observed: self.observed.len(),
            points: timeline.points.len(),
            note,
            probed_as: None,
        });
    }

    /// Settle a bisection with nothing left to probe between its bounds.
    fn settle(&mut self, timeline: &Timeline, lo: usize, hi: usize) {
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
                "it held at {} and fails at {}; the landing(s) between could not be observed, so the break is one of: {}",
                timeline.short(lo),
                timeline.short(hi),
                suspects.join("; ")
            );
            return self.stop(timeline, note);
        }
        let Some(landing) = timeline.landing_at(hi) else {
            let note = format!(
                "it held at {}, the run's last landing, and fails at the tip {}, a commit the run did not land: no task of the run broke it",
                timeline.short(lo),
                timeline.short(hi)
            );
            return self.stop(timeline, note);
        };
        let tasks: Vec<String> = timeline.tasks[hi - 1].iter().cloned().collect();
        if tasks.is_empty() {
            // Pinned, but to no task: say where, never route to nobody.
            let note = format!(
                "it held at {} and first failed at run landing {} ({}), which changed {}, but no task of that landing could be read from the run's records",
                timeline.short(lo),
                landing.commit,
                landing.stage,
                landing.paths.join(", ")
            );
            return self.stop(timeline, note);
        }
        self.phase = Phase::Found(
            AcceptanceRegressionV1 {
                held_at: timeline.points[lo].clone(),
                landing_commit: landing.commit.clone(),
                landing_stage: landing.stage.clone(),
                tasks,
                changed_files: landing.paths.clone(),
                probed_as: None,
            },
            lo,
            hi,
        );
    }

    /// Take a verdict at `point`; `None` is no verdict there.
    fn take(&mut self, timeline: &Timeline, point: usize, verdict: Option<bool>) {
        match verdict {
            Some(passed) => {
                self.observed.insert(point);
                if !passed {
                    self.fails.insert(point);
                }
            }
            None => {
                self.unknown.insert(point);
            }
        }
        match &mut self.phase {
            Phase::Seek { next, .. } => {
                *next += 1;
                if verdict == Some(true) {
                    let hi = self
                        .fails
                        .iter()
                        .copied()
                        .find(|fail| *fail > point)
                        .unwrap_or(timeline.points.len() - 1);
                    self.phase = Phase::Bisect { lo: point, hi };
                }
            }
            Phase::Bisect { lo, hi } => match verdict {
                Some(true) => *lo = point,
                Some(false) => *hi = point,
                None => {}
            },
            _ => return,
        }
        self.advance(timeline);
    }

    /// Close a search that has nothing left to probe.
    fn advance(&mut self, timeline: &Timeline) {
        if self.wants().is_some() {
            return;
        }
        match self.phase {
            Phase::Bisect { lo, hi } => self.settle(timeline, lo, hi),
            Phase::Seek { .. } => {
                let points = timeline.points.len();
                let never_held = self.unknown.is_empty();
                let note = if never_held {
                    format!(
                        "it never held in this run: it fails at the run base and at every one of the run's {} landing(s)",
                        timeline.landings.len()
                    )
                } else {
                    format!(
                        "it held at none of the {} point(s) of the run observed; {} could not be observed",
                        self.observed.len(),
                        self.unknown.len()
                    )
                };
                self.phase = Phase::Stopped(RegressionSearchV1 {
                    never_held,
                    observed: if never_held {
                        points
                    } else {
                        self.observed.len()
                    },
                    points,
                    note,
                    probed_as: None,
                });
            }
            _ => {}
        }
    }

    fn cut_short(&mut self, timeline: &Timeline, budget: SearchBudget) {
        let spent = format!(
            "the regression search budget ({} observations, {} min) ran out",
            budget.observations,
            budget.time.as_secs() / 60
        );
        let state = match self.phase {
            Phase::Bisect { lo, hi } => format!(
                "; it held at {} and fails at {}: one of the landings after the first, up to the second, broke it",
                timeline.short(lo),
                timeline.short(hi)
            ),
            _ => format!(
                " before a point it held at was found ({} of the run's {} point(s) observed)",
                self.observed.len(),
                timeline.points.len()
            ),
        };
        self.stop(timeline, format!("{spent}{state}"));
    }
}

/// The seek order for checks owned by `owners`: the base and the owners'
/// landings latest first (the primary points), then the other landings
/// latest first; never the last point, where the check is known to fail.
fn seek_order(timeline: &Timeline, owners: &BTreeSet<String>) -> (Vec<usize>, usize) {
    let last = timeline.points.len() - 1;
    let landing_points = (1..=timeline.landings.len()).rev().filter(|p| *p != last);
    let owned = |point: &usize| timeline.tasks[point - 1].iter().any(|t| owners.contains(t));
    let mut order: Vec<usize> = (last > 0).then_some(0).into_iter().collect();
    order.extend(landing_points.clone().filter(owned));
    let primary = order.len();
    order.extend(landing_points.filter(|point| !owned(point)));
    (order, primary)
}

/// A search of its own for `check`.
fn search_for<'a>(timeline: &Timeline, check: &'a FailingCheck) -> Search<'a> {
    let last = timeline.points.len() - 1;
    Search {
        probe: check,
        members: vec![check],
        phase: Phase::Seek {
            order: Vec::new(),
            next: 0,
            primary: 0,
        },
        fails: BTreeSet::from([last]),
        unknown: BTreeSet::new(),
        observed: BTreeSet::from([last]),
        confirmed: false,
    }
}

/// Seed each fresh search's seek order from its members' owners.
fn start(timeline: &Timeline, searches: &mut [Search]) {
    for search in searches {
        if !matches!(&search.phase, Phase::Seek { order, .. } if order.is_empty()) {
            continue;
        }
        let owners: BTreeSet<String> = (search.members.iter())
            .flat_map(|check| check.owners.iter().cloned())
            .collect();
        let (order, primary) = seek_order(timeline, &owners);
        search.phase = Phase::Seek {
            order,
            next: 0,
            primary,
        };
        search.advance(timeline);
    }
}

/// Observe until every search is closed or nothing affordable is left.
async fn drive<'a>(
    timeline: &Timeline,
    searches: &mut Vec<Search<'a>>,
    observations: &mut Observations<'_>,
) {
    // Commits the spent budget cannot pay for; a cached one still serves.
    let mut unaffordable: BTreeSet<usize> = BTreeSet::new();
    loop {
        // A landing just pinned is confirmed for the members sharing it
        // at once, before any other search spends the budget.
        let mut split = Vec::new();
        for search in searches.iter_mut() {
            if !search.confirmed && matches!(search.phase, Phase::Found(..)) {
                search.confirmed = true;
                split.extend(confirm(timeline, search, observations).await);
            }
        }
        if !split.is_empty() {
            let mut fresh: Vec<Search> = (split.into_iter())
                .map(|member| search_for(timeline, member))
                .collect();
            start(timeline, &mut fresh);
            searches.extend(fresh);
            continue;
        }
        let wanted: Vec<(usize, usize, bool)> = (searches.iter().enumerate())
            .filter_map(|(at, search)| search.wants().map(|(point, sweep)| (at, point, sweep)))
            .filter(|(_, point, _)| !unaffordable.contains(point))
            .collect();
        let focused = wanted.iter().any(|(_, _, sweep)| !sweep);
        let mut by_point: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (at, point, sweep) in wanted {
            if !(focused && sweep) {
                by_point.entry(point).or_default().push(at);
            }
        }
        // One commit per pass: the one serving the most failing checks,
        // then the one a bisection wants (it is closest to an answer),
        // then the earliest. A search that shares no commit with a larger
        // one waits for it, so the budget goes where it attributes most.
        let served =
            |ats: &Vec<usize>| -> usize { ats.iter().map(|at| searches[*at].members.len()).sum() };
        let bisecting = |ats: &Vec<usize>| {
            ats.iter()
                .any(|at| matches!(searches[*at].phase, Phase::Bisect { .. }))
        };
        let Some((point, ats)) = by_point.into_iter().min_by(|a, b| {
            (served(&b.1).cmp(&served(&a.1)))
                .then(bisecting(&b.1).cmp(&bisecting(&a.1)))
                .then(a.0.cmp(&b.0))
        }) else {
            break;
        };
        let checks: Vec<(String, String)> = (ats.iter())
            .map(|at| {
                (
                    searches[*at].probe.id.clone(),
                    searches[*at].probe.cache_key(),
                )
            })
            .collect();
        let Some(verdicts) = observations.at(&timeline.points[point], &checks).await else {
            unaffordable.insert(point);
            continue;
        };
        for at in ats {
            let verdict = verdicts.get(&searches[at].probe.id).copied();
            searches[at].take(timeline, point, verdict);
        }
    }
}

/// A check shares a pinned landing only once it is seen to hold where the
/// probe held and to fail at the landing that broke the probe: a matching
/// last line is a hint, not proof. Returns the members that disagree, or
/// could not be observed there, for searches of their own.
async fn confirm<'a>(
    timeline: &Timeline,
    search: &mut Search<'a>,
    observations: &mut Observations<'_>,
) -> Vec<&'a FailingCheck> {
    let (lo, hi) = match &search.phase {
        Phase::Found(_, lo, hi) => (*lo, *hi),
        _ => return Vec::new(),
    };
    let others: Vec<(String, String)> = (search.members.iter())
        .filter(|member| member.id != search.probe.id)
        .map(|member| (member.id.clone(), member.cache_key()))
        .collect();
    if others.is_empty() {
        return Vec::new();
    }
    let held = observations.at(&timeline.points[lo], &others).await;
    let broke = observations.at(&timeline.points[hi], &others).await;
    let verdict = |verdicts: &Option<BTreeMap<String, bool>>, id: &String| {
        verdicts.as_ref().and_then(|v| v.get(id).copied())
    };
    let probe = search.probe.id.clone();
    let (kept, apart): (Vec<&FailingCheck>, Vec<&FailingCheck>) =
        search.members.iter().partition(|member| {
            member.id == probe
                || (verdict(&held, &member.id) == Some(true)
                    && verdict(&broke, &member.id) == Some(false))
        });
    search.members = kept;
    apart
}

pub(super) async fn run(
    timeline: &Timeline,
    failing: &[FailingCheck],
    observations: &mut Observations<'_>,
    budget: SearchBudget,
) -> Attribution {
    // One search per signature; a check with none is a search of its own.
    let mut ordered: Vec<&FailingCheck> = failing.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let mut by_signature: BTreeMap<String, usize> = BTreeMap::new();
    let mut searches: Vec<Search> = Vec::new();
    for check in ordered {
        if !check.signature.is_empty()
            && let Some(&at) = by_signature.get(&check.signature)
        {
            searches[at].members.push(check);
            continue;
        }
        if !check.signature.is_empty() {
            by_signature.insert(check.signature.clone(), searches.len());
        }
        searches.push(search_for(timeline, check));
    }
    start(timeline, &mut searches);
    drive(timeline, &mut searches, observations).await;
    let mut attribution = Attribution::default();
    for mut search in searches {
        if search.open() {
            search.cut_short(timeline, budget);
        }
        let probe = search.probe.id.clone();
        for member in &search.members {
            let shared = (member.id != probe).then(|| probe.clone());
            match &search.phase {
                Phase::Found(found, _, _) => {
                    let found = AcceptanceRegressionV1 {
                        probed_as: shared,
                        ..found.clone()
                    };
                    attribution.regressions.insert(member.id.clone(), found);
                }
                Phase::Stopped(stopped) => {
                    // Proven only for the check probed: a member that was
                    // never observed itself is never `never_held`.
                    let stopped = match shared {
                        None => stopped.clone(),
                        Some(probe) => RegressionSearchV1 {
                            never_held: false,
                            observed: 0,
                            points: stopped.points,
                            note: format!(
                                "it fails identically to {probe}, whose search it shares without being probed itself: {}",
                                stopped.note
                            ),
                            probed_as: Some(probe),
                        },
                    };
                    attribution.searches.insert(member.id.clone(), stopped);
                }
                // Every search is closed above; never drop a check.
                _ => {
                    let note = "the regression search ended without an outcome";
                    (attribution.searches)
                        .insert(member.id.clone(), RegressionSearchV1::not_searched(note));
                }
            }
        }
    }
    attribution
}

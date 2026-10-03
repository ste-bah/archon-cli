//! Scheduling the regression searches (Batch J, J2; the searches are in
//! `acceptance_regression_search`). Each pass observes ONE commit, for every
//! search wanting it at once: the one serving the most failing checks (a
//! signature shared by three checks first), then a bisection's, then the
//! earliest. Searches bisecting the same range want the same midpoint, so
//! one observation serves them all. Once the search stops (its no-progress
//! bound, or a cap a caller set), a search still takes whatever it needs
//! that is cached, and whatever is left open is closed with a note saying
//! why.
//!
//! Checks failing identically share one search, but where its bisection
//! settles -- a pinned landing, or a stop between two points -- it is
//! shared only with a member seen GOOD where the lead was and BAD at the
//! upper point (the search's rule); any other member is searched on its
//! own.

use std::collections::{BTreeMap, BTreeSet};

use super::search::{Mark, Phase, Search, mark};
use super::{
    AcceptanceRegressionV1, Attribution, FailingCheck, Observations, RegressionSearchV1, Timeline,
    Verdict,
};

fn key(check: &FailingCheck) -> (String, String) {
    (check.id.clone(), check.cache_key())
}

/// Observe until every search is closed or nothing affordable is left.
async fn drive<'a>(
    timeline: &Timeline,
    searches: &mut Vec<Search<'a>>,
    observations: &mut Observations<'_>,
) {
    // (search, point) pairs the spent budget cannot pay for.
    let mut unaffordable: BTreeSet<(usize, usize)> = BTreeSet::new();
    loop {
        // A bisection just settled is confirmed for the members sharing
        // it at once, before any other search spends the budget.
        let mut split = Vec::new();
        for search in searches.iter_mut() {
            if !search.confirmed && search.settled.is_some() && !search.open() {
                search.confirmed = true;
                split.extend(confirm(timeline, search, observations).await);
            }
        }
        if !split.is_empty() {
            for member in split {
                let mut fresh = Search::new(timeline, vec![member]);
                fresh.start(timeline);
                searches.push(fresh);
            }
            continue;
        }
        let mut by_point: BTreeMap<usize, Vec<(usize, Vec<&FailingCheck>)>> = BTreeMap::new();
        for (at, search) in searches.iter().enumerate() {
            if let Some((point, members)) = search.wants()
                && !unaffordable.contains(&(at, point))
            {
                by_point.entry(point).or_default().push((at, members));
            }
        }
        // One commit per pass: the one serving the most failing checks,
        // then the one a bisection wants (it is closest to an answer),
        // then the earliest. A search that shares no commit with a larger
        // one waits for it, so the budget goes where it attributes most.
        let served = |wants: &Vec<(usize, Vec<&FailingCheck>)>| -> usize {
            wants
                .iter()
                .map(|(at, _)| searches[*at].members.len())
                .sum()
        };
        let bisecting = |wants: &Vec<(usize, Vec<&FailingCheck>)>| {
            (wants.iter()).any(|(at, _)| matches!(searches[*at].phase, Phase::Bisect { .. }))
        };
        let Some((point, wants)) = by_point.into_iter().min_by(|a, b| {
            (served(&b.1).cmp(&served(&a.1)))
                .then(bisecting(&b.1).cmp(&bisecting(&a.1)))
                .then(a.0.cmp(&b.0))
        }) else {
            break;
        };
        let mut checks: Vec<(String, String)> = (wants.iter())
            .flat_map(|(_, members)| members.iter().map(|m| key(m)))
            .collect();
        checks.sort();
        checks.dedup();
        let commit = &timeline.points[point];
        let Some(verdicts) = observations.at(commit, &checks).await else {
            // Spent: a search whose verdicts here are all cached takes them.
            let cached = observations.cached(commit);
            for (at, members) in wants {
                let keys: Vec<(String, String)> = members.iter().map(|m| key(m)).collect();
                if keys.iter().all(|(_, key)| cached.contains_key(key)) {
                    let verdicts: BTreeMap<String, Verdict> = (keys.into_iter())
                        .map(|(id, key)| (id, cached[&key].clone()))
                        .collect();
                    searches[at].take(timeline, point, &verdicts);
                } else {
                    unaffordable.insert((at, point));
                }
            }
            continue;
        };
        for (at, _) in wants {
            searches[at].take(timeline, point, &verdicts);
        }
    }
}

/// A check shares a settled bisection only once it is seen GOOD where the
/// lead was and BAD at the upper point: a matching last line is a hint,
/// not proof. Returns the members that
/// disagree, or could not be observed there, for searches of their own.
async fn confirm<'a>(
    timeline: &Timeline,
    search: &mut Search<'a>,
    observations: &mut Observations<'_>,
) -> Vec<&'a FailingCheck> {
    let Some((lo, hi)) = search.settled else {
        return Vec::new();
    };
    let others: Vec<(String, String)> = (search.members.iter())
        .filter(|member| member.id != search.probe.id)
        .map(|member| key(member))
        .collect();
    if others.is_empty() {
        return Vec::new();
    }
    let held = observations.at(&timeline.points[lo], &others).await;
    let broke = observations.at(&timeline.points[hi], &others).await;
    let verdict = |verdicts: &Option<BTreeMap<String, Verdict>>, id: &String| {
        verdicts.as_ref().and_then(|v| v.get(id).cloned())
    };
    let probe = search.probe.id.clone();
    let (kept, apart): (Vec<&FailingCheck>, Vec<&FailingCheck>) =
        search.members.iter().partition(|member| {
            member.id == probe
                || (verdict(&held, &member.id).and_then(|v| mark(member, &v)) == Some(Mark::Good)
                    && verdict(&broke, &member.id).and_then(|v| mark(member, &v))
                        == Some(Mark::Bad))
        });
    search.members = kept;
    apart
}

pub(super) async fn run(
    timeline: &Timeline,
    failing: &[FailingCheck],
    observations: &mut Observations<'_>,
) -> Attribution {
    // One search per signature; a check with none is a search of its own.
    let mut ordered: Vec<&FailingCheck> = failing.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let mut groups: Vec<Vec<&FailingCheck>> = Vec::new();
    let mut by_signature: BTreeMap<&str, usize> = BTreeMap::new();
    for check in ordered {
        match by_signature.get(check.signature.as_str()) {
            Some(&at) if !check.signature.is_empty() => groups[at].push(check),
            _ => {
                if !check.signature.is_empty() {
                    by_signature.insert(&check.signature, groups.len());
                }
                groups.push(vec![check]);
            }
        }
    }
    let mut searches: Vec<Search> = (groups.into_iter())
        .map(|members| Search::new(timeline, members))
        .collect();
    for search in &mut searches {
        search.start(timeline);
    }
    drive(timeline, &mut searches, observations).await;
    let mut attribution = Attribution::default();
    let spent = observations
        .stop_reason()
        .unwrap_or_else(|| "the regression search stopped".to_string());
    for mut search in searches {
        if search.open() {
            search.cut_short(timeline, &spent);
        }
        let probe = search.probe.id.clone();
        for member in &search.members {
            let shared = (member.id != probe).then(|| probe.clone());
            match &search.phase {
                Phase::Found(found) => {
                    let found = AcceptanceRegressionV1 {
                        probed_as: shared,
                        ..found.clone()
                    };
                    attribution.regressions.insert(member.id.clone(), found);
                }
                Phase::Stopped(stopped) => {
                    // Until a lead bisected, every member was observed
                    // itself: the note is its own.
                    let stopped = match shared {
                        None => stopped.clone(),
                        Some(_) if !search.led => stopped.clone(),
                        Some(probe) => RegressionSearchV1 {
                            never_held: false,
                            note: format!(
                                "it fails identically to {probe}, whose search it shares: {}",
                                stopped.note
                            ),
                            probed_as: Some(probe),
                            ..stopped.clone()
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

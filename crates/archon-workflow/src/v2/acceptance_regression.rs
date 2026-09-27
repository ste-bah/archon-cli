//! Which landing broke a frozen acceptance check, found before the
//! acceptance stage's own remediation runs (Issue-114 follow-up).
//!
//! Live on wf-0ddadd81 one task's remediation tightened a shared command's
//! argument check, and three frozen acceptance checks OTHER tasks own began
//! failing. Nothing ran those checks between that landing and the
//! acceptance stage, which routes a failing check only to the tasks whose
//! `implements` names it: the owners, who can neither see what broke it nor
//! write the file that did.
//!
//! For each failing COMMAND check the host asks where it last held: the
//! commit of the latest run landing by one of its owning tasks (the owner's
//! own delivery), or the run base when no owner landed. If the check passes
//! there, it regressed after, and the host bisects the run's landings
//! between that commit and the tip (`branch_cache::landing`, the host's own
//! commits on the first-parent chain) for the first landing it fails at.
//! That landing's tasks -- the branches whose landed manifest changed what
//! the commit changed, else every task its call dispatched -- are named on
//! the check (`regressed_by`), and the acceptance stage's bounded
//! remediation is routed to them together with the owners, as one unit. A
//! commit the run did not land is no probe point: a break it made is laid
//! at the next landing.
//!
//! Checks run only through the caller's observer -- the acceptance stage's
//! hermetic scratch executor, never the live checkout and never the
//! test-runner path -- at an exact commit. Bounded: at most
//! [`MAX_ATTRIBUTED`] checks are attributed and at most
//! [`MAX_OBSERVATIONS`] observations are made per round, checks sharing a
//! probe commit are observed together, and every verdict is cached per
//! commit under `v2/acceptance/observations/`, so a later round or a resume
//! re-observes nothing. A check whose bound fails too, or whose bisection
//! the budget cut short, names no landing: it is routed as before.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::WorkflowV2ResultStore;
use super::branch_cache::landing::{RunLanding, run_landings_between};

/// Most failing checks one round attributes.
pub const MAX_ATTRIBUTED: usize = 3;
/// Most observations (one per distinct probe commit) one round makes.
pub const MAX_OBSERVATIONS: usize = 8;

/// The landing a check regressed at, as recorded on the check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceRegressionV1 {
    /// The last commit the search saw the check hold at (at or after its
    /// owner's landing, or the run base).
    pub held_at: String,
    /// The first landing after it where the check fails.
    pub landing_commit: String,
    pub landing_stage: String,
    /// That landing's tasks: who the remediation is routed to.
    pub tasks: Vec<String>,
    /// The paths that landing changed, created or deleted: what the
    /// remediation may need to write (`acceptance_routing`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_files: Vec<String>,
}

/// Runs acceptance checks at an exact commit: `id -> passed`, `None` when
/// the observation itself failed (an id missing from the map has no
/// verdict either).
#[async_trait::async_trait]
pub trait CheckObserver: Sync {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, bool>>;
}

/// The run's base commit (its first `repository_bound` event).
pub fn run_base_commit(store: &WorkflowV2ResultStore) -> Option<String> {
    super::write::test_baseline_run_base::run_base_commit(store)
}

fn cache_path(run_dir: &Path, commit: &str) -> PathBuf {
    // A commit id: only its hex digits name the file.
    let sha: String = commit
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(40)
        .collect();
    run_dir
        .join(super::acceptance_stage::ACCEPTANCE_RECORDS_DIR)
        .join("observations")
        .join(format!("{sha}.json"))
}

struct Observations<'a> {
    run_dir: &'a Path,
    observer: &'a dyn CheckObserver,
    budget: usize,
}

impl Observations<'_> {
    fn cached(&self, commit: &str) -> BTreeMap<String, bool> {
        std::fs::read(cache_path(self.run_dir, commit))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// `ids`' verdicts at `commit`, observing the uncached ones in one go
    /// while the budget lasts.
    async fn at(&mut self, commit: &str, ids: &[String]) -> BTreeMap<String, bool> {
        let mut known = self.cached(commit);
        let missing: Vec<String> = ids
            .iter()
            .filter(|id| !known.contains_key(*id))
            .cloned()
            .collect();
        if !missing.is_empty() && self.budget > 0 {
            self.budget -= 1;
            if let Some(seen) = self.observer.observe(commit, &missing).await {
                known.extend(seen.into_iter().filter(|(id, _)| missing.contains(id)));
                let path = cache_path(self.run_dir, commit);
                if let Some(parent) = path.parent()
                    && std::fs::create_dir_all(parent).is_ok()
                    && let Ok(bytes) = serde_json::to_vec_pretty(&known)
                {
                    let _ = std::fs::write(&path, bytes);
                }
            }
        }
        known.retain(|id, _| ids.contains(id));
        known
    }
}

/// The tasks of a landing: those of the stage's branches whose landed
/// manifest changed a path the landing commit changed, or -- when no
/// manifest says -- every task the stage dispatched.
fn landing_tasks(store: &WorkflowV2ResultStore, landing: &RunLanding) -> BTreeSet<String> {
    let mut tasks = BTreeSet::new();
    if let Some(run_dir) = store.root().parent()
        && let Ok(entries) = std::fs::read_dir(
            run_dir
                .join("write-coordination")
                .join("stages")
                .join(&landing.stage)
                .join("manifests"),
        )
    {
        for entry in entries.flatten() {
            let Some(manifest) = std::fs::read(entry.path()).ok().and_then(|bytes| {
                serde_json::from_slice::<crate::write_coordinator::PatchManifest>(&bytes).ok()
            }) else {
                continue;
            };
            let touched = manifest
                .changed_files
                .iter()
                .chain(&manifest.created_files)
                .chain(&manifest.deleted_files)
                .any(|path| landing.paths.contains(path));
            if !touched {
                continue;
            }
            let item = manifest.item_id.to_string();
            if let Ok(Some(outcome)) = store.load_branch_outcome(&manifest.stage_id, &item)
                && let Some(ids) = outcome
                    .result
                    .as_ref()
                    .and_then(|result| result.data.get("canonical_task_ids"))
                    .and_then(serde_json::Value::as_array)
            {
                tasks.extend(
                    ids.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string),
                );
            }
        }
    }
    if !tasks.is_empty() {
        return tasks;
    }
    store
        .load_call_record(&landing.stage)
        .ok()
        .flatten()
        .map(|record| {
            record
                .dispatched_items
                .iter()
                .flat_map(|item| item.canonical_task_ids.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// One failing check to attribute: its id and its owning tasks.
pub struct FailingCheck {
    pub id: String,
    pub owners: Vec<String>,
}

/// The landing each of `failing` regressed at, where one can be shown.
pub async fn attribute_regressions(
    store: &WorkflowV2ResultStore,
    repository: &Path,
    tip: &str,
    failing: &[FailingCheck],
    observer: &dyn CheckObserver,
) -> BTreeMap<String, AcceptanceRegressionV1> {
    let mut found = BTreeMap::new();
    let (Some(run_dir), Some(base)) = (store.root().parent(), run_base_commit(store)) else {
        return found;
    };
    let Some(run_id) = store.load_call_records().ok().and_then(|records| {
        records
            .into_iter()
            .map(|r| r.run_id)
            .find(|id| !id.is_empty())
    }) else {
        return found;
    };
    let Ok(landings) = run_landings_between(repository, &run_id, &base, tip) else {
        return found;
    };
    if landings.is_empty() {
        return found;
    }
    let tasks: Vec<BTreeSet<String>> = landings
        .iter()
        .map(|landing| landing_tasks(store, landing))
        .collect();
    // The probe points: the base, every landing, and the tip.
    let mut points: Vec<String> = std::iter::once(base.clone())
        .chain(landings.iter().map(|landing| landing.commit.clone()))
        .collect();
    if points.last().map(String::as_str) != Some(tip) {
        points.push(tip.to_string());
    }
    let landing_at = |point: usize| -> Option<&RunLanding> {
        point.checked_sub(1).and_then(|at| landings.get(at))
    };
    let mut observations = Observations {
        run_dir,
        observer,
        budget: MAX_OBSERVATIONS,
    };
    // Each check's search: (lo holds, hi fails), over `points`.
    let mut open: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut ordered: Vec<&FailingCheck> = failing.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let mut bounds: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for check in ordered.into_iter().take(MAX_ATTRIBUTED) {
        // The owner's latest landing, else the base.
        let held = (1..=landings.len())
            .rev()
            .find(|&point| {
                check
                    .owners
                    .iter()
                    .any(|owner| tasks[point - 1].contains(owner))
            })
            .unwrap_or(0);
        bounds.entry(held).or_default().push(check.id.clone());
    }
    for (held, ids) in bounds {
        let verdicts = observations.at(&points[held], &ids).await;
        for id in ids {
            if verdicts.get(&id) == Some(&true) {
                open.insert(id, (held, points.len() - 1));
            }
        }
    }
    // Bisect: checks sharing a probe commit are observed together.
    loop {
        let mut probes: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (id, (lo, hi)) in &open {
            if hi - lo > 1 {
                probes.entry((lo + hi) / 2).or_default().push(id.clone());
            }
        }
        if probes.is_empty() || observations.budget == 0 {
            break;
        }
        for (mid, ids) in probes {
            let verdicts = observations.at(&points[mid], &ids).await;
            for id in ids {
                let Some(passed) = verdicts.get(&id) else {
                    // No verdict there: this search stops, unattributed.
                    open.remove(&id);
                    continue;
                };
                if let Some(bound) = open.get_mut(&id) {
                    if *passed {
                        bound.0 = mid;
                    } else {
                        bound.1 = mid;
                    }
                }
            }
        }
    }
    for (id, (lo, hi)) in open {
        if hi - lo != 1 {
            continue;
        }
        let Some(landing) = landing_at(hi) else {
            continue;
        };
        found.insert(
            id,
            AcceptanceRegressionV1 {
                held_at: points[lo].clone(),
                landing_commit: landing.commit.clone(),
                landing_stage: landing.stage.clone(),
                tasks: tasks[hi - 1].iter().cloned().collect(),
                changed_files: landing.paths.clone(),
            },
        );
    }
    found
}

#[cfg(test)]
#[path = "acceptance_regression_tests.rs"]
mod tests;

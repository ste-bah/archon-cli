//! Which landing broke a frozen acceptance check, found before the
//! acceptance stage's own remediation runs (Issue-114 follow-up, Batch J).
//!
//! Live on wf-0ddadd81 one task's remediation tightened a shared command's
//! argument check, and three frozen acceptance checks OTHER tasks own began
//! failing with a location-less `Error: ...`. Nothing ran those checks
//! between that landing and the acceptance stage, which routes a failing
//! check to the tasks whose `implements` names it: the owners, who can
//! neither see what broke it nor write the file that did.
//!
//! Batch J: the search is no longer anchored on the owner's latest landing
//! alone, and no failing check is left unsearched. Batch J2: for each
//! failing COMMAND check the host bisects the run's landings
//! (`branch_cache::landing`, the host's own commits on the first-parent
//! chain) for the first one where the check fails with the signature it
//! fails with now; passing, or failing another way (the feature absent at
//! the base), is GOOD (`acceptance_regression_search`). That landing's
//! tasks -- the branches whose landed manifest changed what the commit
//! changed, else every task its call dispatched -- are named on the check
//! (`regressed_by`) with every path the landing changed: the acceptance
//! stage routes the remediation to them and grants them those paths
//! (`acceptance_routing`). A commit the run did not land is no probe point:
//! a break it made is laid at the next landing, or, after the last one,
//! named in the note.
//!
//! Every failing check handed in comes back with an outcome: a regression,
//! or a [`RegressionSearchV1`] saying what the search established (it
//! already failed this way at the base, or why it stopped). Nothing is
//! dropped.
//!
//! Bounded (`acceptance_regression_drive`): checks whose failure reads the
//! same (their [`FailingCheck::signature`]) share one search and are
//! confirmed member by member at its break; one observation serves every
//! check probed at the same commit; and the whole search stops at a
//! [`SearchBudget`] of observations and wall time. Every verdict (with its
//! failure signature) is cached per (commit, check command) under
//! `v2/acceptance/observations/`, so a later round or a resume re-observes
//! nothing and a re-authored check is never judged by its old command's
//! verdict. Checks run only through the caller's observer -- the acceptance
//! stage's hermetic scratch executor, never the live checkout -- at an exact
//! commit, building from the run's persistent build cache.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::WorkflowV2ResultStore;
use super::branch_cache::landing::{RunLanding, run_landings_between};

#[path = "acceptance_regression_drive.rs"]
mod drive;
#[path = "acceptance_regression_verdict.rs"]
mod verdict;
pub use verdict::{Verdict, failure_signature};
#[path = "acceptance_regression_search.rs"]
mod search;

/// Most observations (one per probe commit; a commit probed again for
/// other checks counts again) one round makes. Batch J2: a search costs the
/// base (shared by every search), a bisection of about log2 of the run's
/// landings (6 for wf-0ddadd81's 57) and a confirmation or two; the four
/// searches of its attempt 6 need about 28, fewer where their midpoints
/// coincide.
pub const MAX_OBSERVATIONS: usize = 32;
/// Most wall time one round's search spends; checked before each
/// observation, so one already started finishes. Batch J2: observations
/// build from the run's build cache (`acceptance_scratch` `cache`), so only
/// the first is a cold build (15-25 min on wf-0ddadd81's copy, by load); a
/// later one reuses every dependency and rebuilds the crates its commit
/// changed or stamps its hash into (2-5 min), then runs its checks (about
/// 2 min each with the scratch's integrity audits): about 9 min for three.
pub const MAX_SEARCH_TIME: Duration = Duration::from_secs(240 * 60);

/// What one round's search may spend.
#[derive(Debug, Clone, Copy)]
pub struct SearchBudget {
    pub observations: usize,
    pub time: Duration,
}

impl Default for SearchBudget {
    fn default() -> Self {
        Self {
            observations: MAX_OBSERVATIONS,
            time: MAX_SEARCH_TIME,
        }
    }
}

/// The landing a check regressed at, as recorded on the check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceRegressionV1 {
    /// The last commit the search saw the check hold at.
    pub held_at: String,
    /// The first landing after it where the check fails.
    pub landing_commit: String,
    pub landing_stage: String,
    /// That landing's tasks: who the remediation is routed to.
    pub tasks: Vec<String>,
    /// The paths that landing changed, created or deleted: what the
    /// remediation is granted (`acceptance_routing`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_files: Vec<String>,
    /// The check whose search this one shares, when it failed identically
    /// and was not probed itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probed_as: Option<String>,
}

/// What the search established for a failing check it could not attribute.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionSearchV1 {
    /// Proven: it failed at the run base and at every run landing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub never_held: bool,
    /// Points it was observed at with a verdict, of the run's `points`.
    #[serde(default)]
    pub observed: usize,
    #[serde(default)]
    pub points: usize,
    /// The finding in words: what the remediation's tasks are told.
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probed_as: Option<String>,
}

impl RegressionSearchV1 {
    /// A check the search could not start on, and why.
    pub fn not_searched(note: impl Into<String>) -> Self {
        Self {
            note: note.into(),
            ..Self::default()
        }
    }
}

/// Every failing check's outcome: exactly one of the two maps names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribution {
    pub regressions: BTreeMap<String, AcceptanceRegressionV1>,
    pub searches: BTreeMap<String, RegressionSearchV1>,
}

/// Runs acceptance checks at an exact commit: each id's [`Verdict`],
/// `None` when the observation itself failed (an id missing from the map
/// has no verdict either).
#[async_trait::async_trait]
pub trait CheckObserver: Sync {
    async fn observe(&self, commit: &str, ids: &[String]) -> Option<BTreeMap<String, Verdict>>;
}

/// The run's base commit (its first `repository_bound` event).
pub fn run_base_commit(store: &WorkflowV2ResultStore) -> Option<String> {
    super::write::test_baseline_run_base::run_base_commit(store)
}

/// A frozen command's fingerprint: what a cached verdict is keyed by.
pub fn command_fingerprint(command: &str) -> String {
    blake3::hash(command.as_bytes()).to_hex()[..16].to_string()
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

pub(crate) struct Observations<'a> {
    run_dir: &'a Path,
    observer: &'a dyn CheckObserver,
    left: usize,
    deadline: Instant,
    pub(crate) made: usize,
}

impl<'a> Observations<'a> {
    pub(crate) fn new(
        run_dir: &'a Path,
        observer: &'a dyn CheckObserver,
        budget: SearchBudget,
    ) -> Self {
        Self {
            run_dir,
            observer,
            left: budget.observations,
            deadline: Instant::now() + budget.time,
            made: 0,
        }
    }

    pub(crate) fn exhausted(&self) -> bool {
        self.left == 0 || Instant::now() >= self.deadline
    }

    fn cached(&self, commit: &str) -> BTreeMap<String, Verdict> {
        std::fs::read(cache_path(self.run_dir, commit))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// The verdicts at `commit` of `checks` (`(id, cache key)`), observing
    /// the uncached ones in one go. `None` when one was needed and the
    /// budget is spent; an id missing from the map had no verdict there.
    pub(crate) async fn at(
        &mut self,
        commit: &str,
        checks: &[(String, String)],
    ) -> Option<BTreeMap<String, Verdict>> {
        let mut known = self.cached(commit);
        let missing: Vec<&(String, String)> = checks
            .iter()
            .filter(|(_, key)| !known.contains_key(key))
            .collect();
        if !missing.is_empty() {
            if self.exhausted() {
                return None;
            }
            self.left -= 1;
            self.made += 1;
            let ids: Vec<String> = missing.iter().map(|(id, _)| id.clone()).collect();
            if let Some(seen) = self.observer.observe(commit, &ids).await {
                for (id, key) in &missing {
                    if let Some(verdict) = seen.get(id) {
                        known.insert(key.clone(), verdict.clone());
                    }
                }
                let path = cache_path(self.run_dir, commit);
                if let Some(parent) = path.parent()
                    && std::fs::create_dir_all(parent).is_ok()
                    && let Ok(bytes) = serde_json::to_vec_pretty(&known)
                {
                    let _ = std::fs::write(&path, bytes);
                }
            }
        }
        Some(
            checks
                .iter()
                .filter_map(|(id, key)| known.get(key).map(|verdict| (id.clone(), verdict.clone())))
                .collect(),
        )
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

/// One failing check to attribute.
#[derive(Debug, Clone, Default)]
pub struct FailingCheck {
    pub id: String,
    /// Its owning tasks: their landings are searched first.
    pub owners: Vec<String>,
    /// [`failure_signature`]: checks sharing a non-empty one are probed
    /// once and share the outcome.
    pub signature: String,
    /// [`command_fingerprint`] of its frozen command: its cached verdicts
    /// are this command's only. Empty keys the cache by id alone.
    pub fingerprint: String,
}

impl FailingCheck {
    fn cache_key(&self) -> String {
        if self.fingerprint.is_empty() {
            self.id.clone()
        } else {
            format!("{}@{}", self.id, self.fingerprint)
        }
    }
}

/// The run's probe points: the base, every run landing, and the tip.
pub(crate) struct Timeline {
    pub(crate) points: Vec<String>,
    pub(crate) landings: Vec<RunLanding>,
    /// Each landing's tasks, by landing index.
    pub(crate) tasks: Vec<BTreeSet<String>>,
}

impl Timeline {
    /// The landing at probe point `point` (point 0 is the base; a trailing
    /// tip the run did not land is no landing).
    pub(crate) fn landing_at(&self, point: usize) -> Option<&RunLanding> {
        point.checked_sub(1).and_then(|at| self.landings.get(at))
    }

    pub(crate) fn short(&self, point: usize) -> String {
        self.points[point].chars().take(12).collect()
    }
}

/// The outcome of every check of `failing` (see the module docs).
pub async fn attribute_regressions(
    store: &WorkflowV2ResultStore,
    repository: &Path,
    tip: &str,
    failing: &[FailingCheck],
    observer: &dyn CheckObserver,
    budget: SearchBudget,
) -> Attribution {
    let unsearched = |note: &str| Attribution {
        regressions: BTreeMap::new(),
        searches: failing
            .iter()
            .map(|check| (check.id.clone(), RegressionSearchV1::not_searched(note)))
            .collect(),
    };
    let (Some(run_dir), Some(base)) = (store.root().parent(), run_base_commit(store)) else {
        return unsearched(
            "the run's base commit is not recorded, so no earlier point of the run could be probed",
        );
    };
    let Some(run_id) = store.load_call_records().ok().and_then(|records| {
        records
            .into_iter()
            .map(|r| r.run_id)
            .find(|id| !id.is_empty())
    }) else {
        return unsearched("the run's id is not recorded, so its landings could not be read");
    };
    let landings = match run_landings_between(repository, &run_id, &base, tip) {
        Ok(landings) => landings,
        Err(error) => {
            return unsearched(&format!(
                "the run's landings could not be read ({error}), so no earlier point could be probed"
            ));
        }
    };
    let tasks = landings
        .iter()
        .map(|landing| landing_tasks(store, landing))
        .collect();
    let mut points: Vec<String> = std::iter::once(base)
        .chain(landings.iter().map(|landing| landing.commit.clone()))
        .collect();
    if points.last().map(String::as_str) != Some(tip) {
        points.push(tip.to_string());
    }
    let timeline = Timeline {
        points,
        landings,
        tasks,
    };
    let mut observations = Observations::new(run_dir, observer, budget);
    drive::run(&timeline, failing, &mut observations, budget).await
}

#[cfg(test)]
#[path = "acceptance_regression_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "acceptance_regression_e2e_tests.rs"]
mod e2e_tests;

#[cfg(test)]
#[path = "acceptance_regression_budget_tests.rs"]
mod budget_tests;

#[cfg(test)]
#[path = "acceptance_regression_group_tests.rs"]
mod group_tests;

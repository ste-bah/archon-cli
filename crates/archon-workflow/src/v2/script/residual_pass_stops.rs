//! Residual passes stop only on a repeated open state, never a pass total.
//! The stored legacy ceiling remains readable for old run metadata, but
//! cannot terminate work. A cycle reports resumable pause evidence.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::super::dispositions::bare_id;
use super::super::{Residual, ResidualPlan};
use super::{left_open, open_ids};
use crate::v2::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use crate::{WorkflowError, WorkflowResult};

impl WorkflowV2ResultStore {
    /// Record that the run paused on the no-progress stop of residual pass
    /// `pass`. Called by the host only after the pause took effect, so the
    /// only way back to the planner is a resume: there the pass plans its
    /// rounds again, and a pass after it that still revisits a state pauses
    /// again.
    pub fn waive_residual_stall(&self, pass: u64) -> WorkflowResult<()> {
        let mut waived = self.residual_stall_waivers();
        if !waived.insert(pass) {
            return Ok(());
        }
        let path = self.root().join(WAIVER_FILE);
        std::fs::create_dir_all(self.root()).map_err(|err| WorkflowError::io(self.root(), err))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&waived)?)
            .map_err(|err| WorkflowError::io(&tmp, err))?;
        std::fs::rename(&tmp, &path).map_err(|err| WorkflowError::io(&path, err))
    }

    fn residual_stall_waivers(&self) -> BTreeSet<u64> {
        std::fs::read(self.root().join(WAIVER_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Whether a resumed pause waived the no-progress stop of pass `pass`.
    pub(in crate::v2::script) fn residual_stall_waived(&self, pass: u64) -> bool {
        self.residual_stall_waivers().contains(&pass)
    }
}

/// Legacy metadata default, retained for reading earlier runs. It is not
/// an execution limit.
pub const DEFAULT_MAX_RESIDUAL_PASSES: u64 = 6;

const CEILING_FILE: &str = "residual-passes.json";
/// The residual passes whose no-progress stop a pause recorded; a resume
/// is the new chance it asked for (rule A), so that stop is waived once.
const WAIVER_FILE: &str = "residual-stall-waivers.json";
const CEILING_KEY: &str = "max_residual_passes";

impl WorkflowV2ResultStore {
    fn residual_ceiling_path(&self) -> PathBuf {
        self.root().join(CEILING_FILE)
    }

    /// Preserve legacy configuration in run metadata. Execution ignores
    /// this total ceiling and stops only on no progress.
    pub fn record_max_residual_passes(&self, ceiling: u64) -> WorkflowResult<()> {
        let path = self.residual_ceiling_path();
        std::fs::create_dir_all(self.root()).map_err(|err| WorkflowError::io(self.root(), err))?;
        let bytes = serde_json::to_vec_pretty(&json!({ CEILING_KEY: ceiling.max(1) }))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
        std::fs::rename(&tmp, &path).map_err(|err| WorkflowError::io(&path, err))
    }

    /// The run's legacy ceiling metadata, with a default for older runs.
    /// This value cannot stop a progressing run.
    pub fn max_residual_passes(&self) -> u64 {
        std::fs::read_to_string(self.residual_ceiling_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|value| value.get(CEILING_KEY).and_then(Value::as_u64))
            .unwrap_or(DEFAULT_MAX_RESIDUAL_PASSES)
            .max(1)
    }
}

/// Why a pass was stopped.
enum Stop {
    /// The pass before it left the open set this earlier pass left.
    Cycle { repeats: u64 },
}

/// Compatibility with earlier planners: a total ceiling never stops work.
pub(in crate::v2::script) fn capped(
    _n: u64,
    would: ResidualPlan,
    _previous: &ResidualPlan,
    _store: &WorkflowV2ResultStore,
    _stored: &[WorkflowV2CallRecord],
) -> ResidualPlan {
    would
}

/// Pass `n` (4 on) after the repeated-state stop: `plans` are passes 1..n-1, as each
/// was finally planned.
pub(super) fn checked(
    n: u64,
    would: ResidualPlan,
    plans: &[ResidualPlan],
    store: &WorkflowV2ResultStore,
    stored: &[WorkflowV2CallRecord],
) -> ResidualPlan {
    let Some(previous) = plans.last() else {
        return would;
    };
    if would.rounds.is_empty() {
        return would;
    }
    let open = open_ids(previous, stored);
    let earlier = &plans[..plans.len().saturating_sub(2)];
    let repeats = (!open.is_empty())
        .then(|| {
            earlier
                .iter()
                .position(|plan| open_ids(plan, stored) == open)
        })
        .flatten();
    match repeats {
        Some(at) if !store.residual_stall_waived(n) => stopped(
            n,
            Stop::Cycle {
                repeats: at as u64 + 1,
            },
            previous,
            would,
            stored,
        ),
        _ => capped(n, would, previous, store, stored),
    }
}

/// Pass `n` stopped: no round; `would`'s own reports kept; every gap
/// `previous` left open and every gap `would` carried that neither already
/// reports reported, once each, quoted whole, with the stop's reason and
/// every open id.
fn stopped(
    n: u64,
    stop: Stop,
    previous: &ResidualPlan,
    would: ResidualPlan,
    stored: &[WorkflowV2CallRecord],
) -> ResidualPlan {
    let mut plan = ResidualPlan {
        rounds: Vec::new(),
        reported: would.reported,
    };
    // A gap the pass before reported already stands (a pass past the
    // ceiling after a stopped one would only report it again).
    let mut seen: BTreeSet<String> = plan
        .reported
        .iter()
        .chain(&previous.reported)
        .map(|(gap, _)| gap.key())
        .collect();
    let gaps: Vec<Residual> = left_open(previous, stored)
        .into_iter()
        .chain(would.rounds.into_iter().flat_map(|round| round.residuals))
        .filter(|gap| seen.insert(gap.key()))
        .collect();
    let ids: BTreeSet<String> = gaps.iter().map(|gap| bare_id(&gap.id)).collect();
    let ids = ids.into_iter().collect::<Vec<_>>().join(", ");
    let reason = match stop {
        Stop::Cycle { repeats } => format!(
            "cycle: pass {} left open the same gaps as pass {repeats}, so no further pass breaks the cycle",
            n - 1
        ),
    };
    for gap in gaps {
        let why = format!(
            "no_progress: the residual passes paused before pass {n} on their {reason}; every gap still open ({ids}) stands for the next attempt to route; it stands as recorded: {:?}",
            gap.description
        );
        plan.reported.push((gap, why));
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_is_what_the_run_recorded_else_the_default_and_never_below_one() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        assert_eq!(store.max_residual_passes(), DEFAULT_MAX_RESIDUAL_PASSES);
        store.record_max_residual_passes(3).unwrap();
        assert_eq!(store.max_residual_passes(), 3);
        // A reader over the same directory sees the same bound.
        let other = WorkflowV2ResultStore::new(temp.path().join("v2"));
        assert_eq!(other.max_residual_passes(), 3);
        store.record_max_residual_passes(0).unwrap();
        assert_eq!(store.max_residual_passes(), 1);
        std::fs::write(store.residual_ceiling_path(), "not json").unwrap();
        assert_eq!(store.max_residual_passes(), DEFAULT_MAX_RESIDUAL_PASSES);
    }
}

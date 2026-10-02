//! Issue-225: the two stops that bound the residual passes.
//!
//! Progress alone (`residual_later_pass`) does not end the passes: a
//! verifier that records a NEW gap every pass keeps the open set moving,
//! and one whose open set returns to an earlier pass's keeps it moving in
//! a circle. Either is a run that never ends. So a pass that would run a
//! round is stopped when
//!
//! - CYCLE: the open gaps (any severity, by id) the pass before it left
//!   are a non-empty set some EARLIER pass also left -- the pass just
//!   before that is the stall rule's, already a stop; or
//! - CEILING: its number is past the run's recorded ceiling
//!   ([`WorkflowV2ResultStore::max_residual_passes`], the host's
//!   `workflow.generated.max_residual_passes`).
//!
//! A stopped pass plans no round. Every gap the pass before it left open,
//! and every gap the stopped pass would have carried, is reported by id
//! with the stop's reason, so the final gate blocks on each one (the run
//! is not accepted) and the next attempt routes every one of them. Nothing
//! is dropped: what the stopped pass would itself have reported is kept.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::super::dispositions::bare_id;
use super::super::{Residual, ResidualPlan};
use super::{left_open, open_ids};
use crate::v2::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use crate::{WorkflowError, WorkflowResult};

/// The ceiling a run that recorded none is held to. The host's own default
/// for `workflow.generated.max_residual_passes` is the same number.
pub const DEFAULT_MAX_RESIDUAL_PASSES: u64 = 6;

const CEILING_FILE: &str = "residual-passes.json";
const CEILING_KEY: &str = "max_residual_passes";

impl WorkflowV2ResultStore {
    fn residual_ceiling_path(&self) -> PathBuf {
        self.root().join(CEILING_FILE)
    }

    /// Record the run's residual pass ceiling beside its records, so every
    /// reader of this store -- the slots' views, the dispatch check, the
    /// final gate, a resume -- plans the passes under the same bound. A
    /// value below 1 is recorded as 1: the first pass always plans.
    pub fn record_max_residual_passes(&self, ceiling: u64) -> WorkflowResult<()> {
        let path = self.residual_ceiling_path();
        std::fs::create_dir_all(self.root()).map_err(|err| WorkflowError::io(self.root(), err))?;
        let bytes = serde_json::to_vec_pretty(&json!({ CEILING_KEY: ceiling.max(1) }))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
        std::fs::rename(&tmp, &path).map_err(|err| WorkflowError::io(&path, err))
    }

    /// How many residual passes may run rounds in this run: the recorded
    /// ceiling, else [`DEFAULT_MAX_RESIDUAL_PASSES`] (a record that cannot
    /// be read is no record -- the default still bounds the passes).
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
    /// It is past the run's ceiling.
    Ceiling(u64),
}

/// Pass `n` (2 on) as planned, unless it is past the run's ceiling and
/// would run a round: then stopped, every gap reported.
pub(in crate::v2::script) fn capped(
    n: u64,
    would: ResidualPlan,
    previous: &ResidualPlan,
    store: &WorkflowV2ResultStore,
    stored: &[WorkflowV2CallRecord],
) -> ResidualPlan {
    let ceiling = store.max_residual_passes();
    if n <= ceiling || would.rounds.is_empty() {
        return would;
    }
    stopped(n, Stop::Ceiling(ceiling), previous, would, stored)
}

/// Pass `n` (4 on) after both stops: `plans` are passes 1..n-1, as each
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
        Some(at) => stopped(
            n,
            Stop::Cycle {
                repeats: at as u64 + 1,
            },
            previous,
            would,
            stored,
        ),
        None => capped(n, would, previous, store, stored),
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
        Stop::Ceiling(ceiling) => format!(
            "ceiling: the run's ceiling of {ceiling} residual pass(es) (max_residual_passes) is reached"
        ),
    };
    for gap in gaps {
        let why = format!(
            "the residual passes stopped before pass {n} on their {reason}; every gap still open ({ids}) stands for the next attempt to route; it stands as recorded: {:?}",
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

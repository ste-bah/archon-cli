//! Persist in-memory `Trajectory` structs to the CozoDB `trajectories` relation.
//!
//! Provides single-row and batch `:put` — CozoDB upserts by key, so re-putting
//! the same `trajectory_id` overwrites (useful for feedback-triggered updates).

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::sona::Trajectory;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// One `:put` taking its rows from `$rows`, shared by the single-row and batch
/// writers.
///
/// Both go through the same script so a batch cannot drift from what the
/// one-row path would have written: the row shape is built in exactly one
/// place, [`row_values`], and the column list appears exactly once.
const PUT_TRAJECTORIES: &str = "\
     ?[trajectory_id, route, agent_key, session_id, patterns, context, \
     embedding, quality, reward, feedback_score, weights_path, created_at, \
     updated_at] <- $rows \
     :put trajectories { trajectory_id => route, agent_key, session_id, \
     patterns, context, embedding, quality, reward, feedback_score, \
     weights_path, created_at, updated_at }";

/// Persist a single trajectory to CozoDB. Upserts by `trajectory_id`.
pub fn store_trajectory(db: &DbInstance, trajectory: &Trajectory) -> Result<()> {
    put_rows(db, vec![row_values(trajectory)])
        .map_err(|e| anyhow::anyhow!("trajectory_store::store_trajectory: {e}"))
}

/// One trajectory as the positional row `PUT_TRAJECTORIES` binds.
///
/// Order is the column order in that script and nothing else may reorder it:
/// the relation's columns are all strings, lists and numbers in runs, so a
/// transposition of two same-typed columns would store silently swapped values
/// rather than failing.
fn row_values(trajectory: &Trajectory) -> DataValue {
    let strings = |values: &[String]| {
        DataValue::List(
            values
                .iter()
                .map(|value| DataValue::Str(value.clone().into()))
                .collect(),
        )
    };
    DataValue::List(vec![
        DataValue::Str(trajectory.trajectory_id.clone().into()),
        DataValue::Str(trajectory.route.clone().into()),
        DataValue::Str(trajectory.agent_key.clone().into()),
        DataValue::Str(trajectory.session_id.clone().into()),
        strings(&trajectory.patterns),
        strings(&trajectory.context),
        DataValue::List(
            trajectory
                .embedding
                .iter()
                .map(|&f| DataValue::from(f as f64))
                .collect(),
        ),
        DataValue::from(trajectory.quality),
        DataValue::from(trajectory.reward),
        DataValue::from(trajectory.feedback_score),
        DataValue::Str(trajectory.weights_path.clone().into()),
        DataValue::from(trajectory.created_at as i64),
        DataValue::from(trajectory.updated_at as i64),
    ])
}

fn put_rows(db: &DbInstance, rows: Vec<DataValue>) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    super::run_script_guarded(
        db,
        PUT_TRAJECTORIES,
        params,
        ScriptMutability::Mutable,
        "store pipeline learning trajectories",
    )
    .map(|_| ())
}

/// Read back every trajectory recorded on one route as `(quality, created_at)`.
///
/// The projection is deliberately narrow. Its only caller is the parameter
/// tuner, which replays recorded outcomes through `SonaEngine` to rebuild
/// weights; pulling embeddings and context back for that would move megabytes
/// to compute two floats. Returned unordered — the tuner sorts by `created_at`,
/// because relying on a store's row order to define replay order is how a
/// learner silently changes its answer when the store is compacted.
pub fn load_route_outcomes(db: &DbInstance, route: &str) -> Result<Vec<(f64, i64)>> {
    let mut params = BTreeMap::new();
    params.insert("route".to_string(), DataValue::Str(route.into()));

    let rows = super::run_script_guarded(
        db,
        "?[quality, created_at] := *trajectories{ route, quality, created_at }, route = $route",
        params,
        ScriptMutability::Immutable,
        "load pipeline learning trajectories for one route",
    )
    .map_err(|e| anyhow::anyhow!("trajectory_store::load_route_outcomes: {e}"))?;

    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            // A row missing either column is a schema the tuner does not
            // understand; dropping it keeps the replay honest rather than
            // substituting a zero that would read as "no pressure".
            Some((row.first()?.get_float()?, row.get(1)?.get_int()?))
        })
        .collect())
}

/// Persist multiple trajectories in a single CozoDB transaction. All rows land
/// or none do.
///
/// This used to be a loop over [`store_trajectory`], which is not what the name
/// promises and is not free: every guarded mutable script canonicalises the
/// store path, takes the cross-process write lock file, and commits its own
/// transaction. That round trip costs roughly a millisecond on Linux and an
/// order of magnitude more on Windows, where opening and byte-range locking a
/// file is a far heavier syscall — so a caller persisting a few hundred
/// trajectories paid seconds of pure overhead on one platform and not the
/// other. One script for the whole slice pays it once.
pub fn store_trajectory_batch(db: &DbInstance, trajectories: &[Trajectory]) -> Result<()> {
    if trajectories.is_empty() {
        return Ok(());
    }
    put_rows(db, trajectories.iter().map(row_values).collect())
        .map_err(|e| anyhow::anyhow!("trajectory_store::store_trajectory_batch: {e}"))
}

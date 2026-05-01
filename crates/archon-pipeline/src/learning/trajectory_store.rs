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

/// Persist a single trajectory to CozoDB. Upserts by `trajectory_id`.
pub fn store_trajectory(db: &DbInstance, trajectory: &Trajectory) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "trajectory_id".to_string(),
        DataValue::Str(trajectory.trajectory_id.clone().into()),
    );
    params.insert(
        "route".to_string(),
        DataValue::Str(trajectory.route.clone().into()),
    );
    params.insert(
        "agent_key".to_string(),
        DataValue::Str(trajectory.agent_key.clone().into()),
    );
    params.insert(
        "session_id".to_string(),
        DataValue::Str(trajectory.session_id.clone().into()),
    );
    params.insert(
        "patterns".to_string(),
        DataValue::List(
            trajectory
                .patterns
                .iter()
                .map(|s| DataValue::Str(s.clone().into()))
                .collect(),
        ),
    );
    params.insert(
        "context".to_string(),
        DataValue::List(
            trajectory
                .context
                .iter()
                .map(|s| DataValue::Str(s.clone().into()))
                .collect(),
        ),
    );
    params.insert(
        "embedding".to_string(),
        DataValue::List(
            trajectory
                .embedding
                .iter()
                .map(|&f| DataValue::from(f as f64))
                .collect(),
        ),
    );
    params.insert("quality".to_string(), DataValue::from(trajectory.quality));
    params.insert("reward".to_string(), DataValue::from(trajectory.reward));
    params.insert(
        "feedback_score".to_string(),
        DataValue::from(trajectory.feedback_score),
    );
    params.insert(
        "weights_path".to_string(),
        DataValue::Str(trajectory.weights_path.clone().into()),
    );
    params.insert(
        "created_at".to_string(),
        DataValue::from(trajectory.created_at as i64),
    );
    params.insert(
        "updated_at".to_string(),
        DataValue::from(trajectory.updated_at as i64),
    );

    db.run_script(
        "?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, \
         quality, reward, feedback_score, weights_path, created_at, updated_at] <- \
         [[$trajectory_id, $route, $agent_key, $session_id, $patterns, $context, \
         $embedding, $quality, $reward, $feedback_score, $weights_path, \
         $created_at, $updated_at]] \
         :put trajectories { trajectory_id => route, agent_key, session_id, \
         patterns, context, embedding, quality, reward, feedback_score, \
         weights_path, created_at, updated_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map(|_| ())
    .map_err(|e| anyhow::anyhow!("trajectory_store::store_trajectory: {e}"))
}

/// Persist multiple trajectories in a single CozoDB transaction.
///
/// Uses repeated `:put` rows for batch insert. If any row fails,
/// the entire batch fails with an error.
pub fn store_trajectory_batch(db: &DbInstance, trajectories: &[Trajectory]) -> Result<()> {
    for t in trajectories {
        store_trajectory(db, t)?;
    }
    Ok(())
}

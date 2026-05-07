use super::helpers::now_epoch;
use super::store::DescEpisodeStore;
use anyhow::Result;
use cozo::{DataValue, ScriptMutability};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// TrajectoryLinker
// ---------------------------------------------------------------------------

/// Link DESC episodes to SONA trajectory records.
pub struct TrajectoryLinker;

impl TrajectoryLinker {
    /// Attach a SONA `trajectory_id` to the given episode.
    ///
    /// Preserves existing `task_type`, `solution`, and `quality_score`.
    pub fn link_episode_to_trajectory(
        store: &DescEpisodeStore,
        episode_id: &str,
        trajectory_id: &str,
    ) -> Result<()> {
        let now = now_epoch();

        // Read current metadata.
        let mut p = BTreeMap::new();
        p.insert("eid".to_string(), DataValue::from(episode_id));

        let cur = store
            .db
            .run_script(
                "?[task_type, solution, quality_score] := \
                 *desc_episode_metadata{ episode_id, task_type, solution, quality_score }, \
                 episode_id = $eid",
                p,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("link_trajectory read failed: {}", e))?;

        if cur.rows.is_empty() {
            return Err(anyhow::anyhow!(
                "episode '{}' not found in desc_episode_metadata",
                episode_id
            ));
        }

        let task_type = cur.rows[0][0].get_str().unwrap_or("").to_string();
        let solution = cur.rows[0][1].get_str().unwrap_or("").to_string();
        let quality_score = cur.rows[0][2].get_float().unwrap_or(0.0);

        let mut p2 = BTreeMap::new();
        p2.insert("eid".to_string(), DataValue::from(episode_id));
        p2.insert("tt".to_string(), DataValue::from(task_type.as_str()));
        p2.insert("sol".to_string(), DataValue::from(solution.as_str()));
        p2.insert("qs".to_string(), DataValue::from(quality_score));
        p2.insert("tid".to_string(), DataValue::from(trajectory_id));
        p2.insert("ts".to_string(), DataValue::from(now as i64));

        store
            .db
            .run_script(
                "?[episode_id, task_type, solution, quality_score, trajectory_id, updated_at] \
                 <- [[$eid, $tt, $sol, $qs, $tid, $ts]] \
                 :put desc_episode_metadata { \
                     episode_id => task_type, solution, quality_score, trajectory_id, updated_at \
                 }",
                p2,
                ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("link_trajectory write failed: {}", e))?;

        Ok(())
    }

    /// Return all trajectory IDs linked to the given episode.
    ///
    /// Filters out empty strings that represent "not linked".
    pub fn get_linked_trajectories(
        store: &DescEpisodeStore,
        episode_id: &str,
    ) -> Result<Vec<String>> {
        let mut p = BTreeMap::new();
        p.insert("eid".to_string(), DataValue::from(episode_id));

        let result = store
            .db
            .run_script(
                "?[trajectory_id] := \
                 *desc_episode_metadata{ episode_id, trajectory_id }, \
                 episode_id = $eid, trajectory_id != ''",
                p,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("get_linked_trajectories query failed: {}", e))?;

        Ok(result
            .rows
            .iter()
            .filter_map(|row| row[0].get_str().map(|s| s.to_string()))
            .collect())
    }
}

use super::helpers::{now_epoch, row_to_episode};
use super::types::{DescEpisode, EpisodeQuery};
use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use std::collections::BTreeMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// DescEpisodeStore
// ---------------------------------------------------------------------------

/// DESC Episode Store - direct CozoDB access, no UCM daemon.
pub struct DescEpisodeStore {
    pub(crate) db: Arc<DbInstance>,
}

impl DescEpisodeStore {
    /// Create a new store wrapping an already-initialised `DbInstance`.
    ///
    /// Call `schema::initialize_learning_schemas(&db)` before constructing.
    pub fn new(db: DbInstance) -> Self {
        Self { db: Arc::new(db) }
    }

    /// Create a store backed by a shared project learning database.
    pub fn from_arc(db: Arc<DbInstance>) -> Self {
        Self { db }
    }

    // -----------------------------------------------------------------------
    // CRUD
    // -----------------------------------------------------------------------

    /// Store a new episode. Returns the `episode_id`.
    ///
    /// Writes to both `desc_episodes` (base fields) and
    /// `desc_episode_metadata` (extended fields). Both are upserted so
    /// calling this twice with the same `episode_id` is idempotent.
    pub fn store_episode(&self, episode: &DescEpisode) -> Result<String> {
        let now = now_epoch();

        // --- desc_episodes ---
        let tags: Vec<DataValue> = episode
            .tags
            .iter()
            .map(|t| DataValue::from(t.as_str()))
            .collect();

        let mut p = BTreeMap::new();
        p.insert(
            "eid".to_string(),
            DataValue::from(episode.episode_id.as_str()),
        );
        p.insert(
            "sid".to_string(),
            DataValue::from(episode.session_id.as_str()),
        );
        p.insert(
            "desc".to_string(),
            DataValue::from(episode.description.as_str()),
        );
        p.insert(
            "outcome".to_string(),
            DataValue::from(episode.outcome.as_str()),
        );
        p.insert("reward".to_string(), DataValue::from(episode.reward));
        p.insert("tags".to_string(), DataValue::List(tags));
        p.insert("ts".to_string(), DataValue::from(now as i64));

        self.db
            .run_script(
                "?[episode_id, session_id, description, outcome, reward, tags, created_at] \
                 <- [[$eid, $sid, $desc, $outcome, $reward, $tags, $ts]] \
                 :put desc_episodes { \
                     episode_id => session_id, description, outcome, reward, tags, created_at \
                 }",
                p,
                ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("store desc_episodes failed: {}", e))?;

        // --- desc_episode_metadata ---
        let traj = episode.trajectory_id.as_deref().unwrap_or("");

        let mut m = BTreeMap::new();
        m.insert(
            "eid".to_string(),
            DataValue::from(episode.episode_id.as_str()),
        );
        m.insert(
            "tt".to_string(),
            DataValue::from(episode.task_type.as_str()),
        );
        m.insert(
            "sol".to_string(),
            DataValue::from(episode.solution.as_str()),
        );
        m.insert("qs".to_string(), DataValue::from(episode.quality_score));
        m.insert("tid".to_string(), DataValue::from(traj));
        m.insert("ts".to_string(), DataValue::from(now as i64));

        self.db
            .run_script(
                "?[episode_id, task_type, solution, quality_score, trajectory_id, updated_at] \
                 <- [[$eid, $tt, $sol, $qs, $tid, $ts]] \
                 :put desc_episode_metadata { \
                     episode_id => task_type, solution, quality_score, trajectory_id, updated_at \
                 }",
                m,
                ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("store desc_episode_metadata failed: {}", e))?;

        Ok(episode.episode_id.clone())
    }

    /// Get an episode by ID. Returns `None` if not found.
    pub fn get_episode(&self, episode_id: &str) -> Result<Option<DescEpisode>> {
        let mut p = BTreeMap::new();
        p.insert("eid".to_string(), DataValue::from(episode_id));

        let result = self
            .db
            .run_script(
                "?[episode_id, session_id, description, outcome, reward, tags, created_at, \
                   task_type, solution, quality_score, trajectory_id, updated_at] := \
                 *desc_episodes{ episode_id, session_id, description, outcome, reward, tags, created_at }, \
                 *desc_episode_metadata{ episode_id, task_type, solution, quality_score, trajectory_id, updated_at }, \
                 episode_id = $eid",
                p,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("get_episode query failed: {}", e))?;

        if result.rows.is_empty() {
            return Ok(None);
        }

        Ok(Some(row_to_episode(&result.rows[0])))
    }

    /// Find episodes matching query criteria, ordered by quality_score descending.
    pub fn find_episodes(&self, query: &EpisodeQuery) -> Result<Vec<DescEpisode>> {
        let mut p = BTreeMap::new();
        p.insert("lim".to_string(), DataValue::from(query.limit as i64));

        // Build the Datalog body depending on which filters are active.
        let base = "?[episode_id, session_id, description, outcome, reward, tags, created_at, \
                      task_type, solution, quality_score, trajectory_id, updated_at] := \
                    *desc_episodes{ episode_id, session_id, description, outcome, reward, tags, created_at }, \
                    *desc_episode_metadata{ episode_id, task_type, solution, quality_score, trajectory_id, updated_at }";

        let script: String = match (&query.task_type, query.min_quality) {
            (Some(tt), Some(mq)) => {
                p.insert("tt".to_string(), DataValue::from(tt.as_str()));
                p.insert("mq".to_string(), DataValue::from(mq));
                format!(
                    "{base}, task_type = $tt, quality_score >= $mq \
                     :order -quality_score :limit $lim"
                )
            }
            (Some(tt), None) => {
                p.insert("tt".to_string(), DataValue::from(tt.as_str()));
                format!(
                    "{base}, task_type = $tt \
                     :order -quality_score :limit $lim"
                )
            }
            (None, Some(mq)) => {
                p.insert("mq".to_string(), DataValue::from(mq));
                format!(
                    "{base}, quality_score >= $mq \
                     :order -quality_score :limit $lim"
                )
            }
            (None, None) => {
                format!("{base} :order -quality_score :limit $lim")
            }
        };

        let result = self
            .db
            .run_script(&script, p, ScriptMutability::Immutable)
            .map_err(|e| anyhow::anyhow!("find_episodes query failed: {}", e))?;

        Ok(result.rows.iter().map(|row| row_to_episode(row)).collect())
    }

    /// Update the quality score of an existing episode.
    ///
    /// Reads the current metadata row and overwrites only `quality_score`
    /// and `updated_at`, preserving all other fields.
    pub fn update_quality(&self, episode_id: &str, new_quality: f64) -> Result<()> {
        let now = now_epoch();

        // Fetch current metadata to preserve task_type, solution, trajectory_id.
        let mut p = BTreeMap::new();
        p.insert("eid".to_string(), DataValue::from(episode_id));

        let cur = self
            .db
            .run_script(
                "?[task_type, solution, trajectory_id] := \
                 *desc_episode_metadata{ episode_id, task_type, solution, trajectory_id }, \
                 episode_id = $eid",
                p,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("update_quality read failed: {}", e))?;

        if cur.rows.is_empty() {
            return Err(anyhow::anyhow!(
                "episode '{}' not found in desc_episode_metadata",
                episode_id
            ));
        }

        let task_type = cur.rows[0][0].get_str().unwrap_or("").to_string();
        let solution = cur.rows[0][1].get_str().unwrap_or("").to_string();
        let trajectory_id = cur.rows[0][2].get_str().unwrap_or("").to_string();

        let mut p2 = BTreeMap::new();
        p2.insert("eid".to_string(), DataValue::from(episode_id));
        p2.insert("tt".to_string(), DataValue::from(task_type.as_str()));
        p2.insert("sol".to_string(), DataValue::from(solution.as_str()));
        p2.insert("qs".to_string(), DataValue::from(new_quality));
        p2.insert("tid".to_string(), DataValue::from(trajectory_id.as_str()));
        p2.insert("ts".to_string(), DataValue::from(now as i64));

        self.db
            .run_script(
                "?[episode_id, task_type, solution, quality_score, trajectory_id, updated_at] \
                 <- [[$eid, $tt, $sol, $qs, $tid, $ts]] \
                 :put desc_episode_metadata { \
                     episode_id => task_type, solution, quality_score, trajectory_id, updated_at \
                 }",
                p2,
                ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("update_quality write failed: {}", e))?;

        Ok(())
    }
}

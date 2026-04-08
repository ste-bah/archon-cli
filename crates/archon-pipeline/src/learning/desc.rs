//! DESC Episode Store — episode CRUD, injection filtering, quality monitoring.
//!
//! Implements REQ-LEARN-008. Native Rust — no UCM daemon dependency.
//! All operations use direct CozoDB calls against `desc_episodes` and
//! `desc_episode_metadata` relations initialized by `schema::initialize_learning_schemas`.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default quality degradation threshold for `QualityMonitor`.
pub const DEFAULT_QUALITY_THRESHOLD: f64 = 0.5;

/// Default maximum number of episodes returned by `InjectionFilter`.
pub const DEFAULT_MAX_INJECTION: usize = 3;

/// Default minimum quality score for `InjectionFilter`.
pub const DEFAULT_MIN_INJECTION_QUALITY: f64 = 0.3;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A DESC episode recording a past pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescEpisode {
    pub episode_id: String,
    pub session_id: String,
    pub task_type: String,
    pub description: String,
    pub solution: String,
    pub outcome: String,
    pub quality_score: f64,
    pub reward: f64,
    pub tags: Vec<String>,
    pub trajectory_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Query parameters for finding episodes.
#[derive(Debug, Clone)]
pub struct EpisodeQuery {
    pub task_type: Option<String>,
    pub min_quality: Option<f64>,
    pub limit: usize,
}

impl Default for EpisodeQuery {
    fn default() -> Self {
        Self {
            task_type: None,
            min_quality: None,
            limit: 10,
        }
    }
}

/// Result of injection filtering — episodes ranked for prompt injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionResult {
    pub episodes: Vec<RankedEpisode>,
    pub injection_confidence: f64,
}

/// An episode ranked for injection with a combined score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedEpisode {
    pub episode: DescEpisode,
    /// Combined score: quality_score * similarity.
    pub score: f64,
}

/// Quality monitoring report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    pub mean_quality: f64,
    pub min_quality: f64,
    pub max_quality: f64,
    pub total_episodes: usize,
    pub degradation_detected: bool,
    pub degradation_threshold: f64,
}

// ---------------------------------------------------------------------------
// DescEpisodeStore
// ---------------------------------------------------------------------------

/// DESC Episode Store — direct CozoDB access, no UCM daemon.
pub struct DescEpisodeStore {
    pub(crate) db: DbInstance,
}

impl DescEpisodeStore {
    /// Create a new store wrapping an already-initialised `DbInstance`.
    ///
    /// Call `schema::initialize_learning_schemas(&db)` before constructing.
    pub fn new(db: DbInstance) -> Self {
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

// ---------------------------------------------------------------------------
// InjectionFilter
// ---------------------------------------------------------------------------

/// Filter and rank DESC episodes for prompt injection into a running pipeline.
pub struct InjectionFilter;

impl InjectionFilter {
    /// Return up to `max_count` episodes ranked by `quality_score * similarity`.
    ///
    /// Similarity is approximated via task-type matching: 1.0 for exact match,
    /// 0.5 otherwise. Only episodes with `quality_score >= min_quality` are
    /// considered.
    pub fn filter_for_injection(
        store: &DescEpisodeStore,
        task_type: &str,
        max_count: usize,
        min_quality: f64,
    ) -> Result<InjectionResult> {
        // Over-fetch to allow ranking before truncation.
        let overfetch = max_count.saturating_mul(3).max(10);
        let query = EpisodeQuery {
            min_quality: Some(min_quality),
            limit: overfetch,
            ..Default::default()
        };

        let episodes = store.find_episodes(&query)?;

        let mut ranked: Vec<RankedEpisode> = episodes
            .into_iter()
            .map(|ep| {
                let similarity = if ep.task_type == task_type {
                    1.0_f64
                } else {
                    0.5_f64
                };
                let score = ep.quality_score * similarity;
                RankedEpisode { episode: ep, score }
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(max_count);

        let injection_confidence = ranked.first().map(|r| r.score).unwrap_or(0.0);

        Ok(InjectionResult {
            episodes: ranked,
            injection_confidence,
        })
    }
}

// ---------------------------------------------------------------------------
// ConfidenceCalculator
// ---------------------------------------------------------------------------

/// Calculate a confidence score incorporating quality, similarity, and recency.
pub struct ConfidenceCalculator;

impl ConfidenceCalculator {
    /// Combined confidence: `quality * similarity * recency`, clamped to [0, 1].
    ///
    /// Recency decays with a 1-day half-life: `1 / (1 + age_days)`.
    pub fn calculate(quality: f64, similarity: f64, age_secs: u64) -> f64 {
        let recency = 1.0 / (1.0 + (age_secs as f64 / 86_400.0));
        (quality * similarity * recency).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// QualityMonitor
// ---------------------------------------------------------------------------

/// Monitor quality distribution across all stored episodes and detect degradation.
pub struct QualityMonitor;

impl QualityMonitor {
    /// Compute quality statistics and flag degradation when mean < `threshold`.
    pub fn check(store: &DescEpisodeStore, threshold: f64) -> Result<QualityReport> {
        let episodes = store.find_episodes(&EpisodeQuery {
            limit: 1_000,
            ..Default::default()
        })?;

        if episodes.is_empty() {
            return Ok(QualityReport {
                mean_quality: 0.0,
                min_quality: 0.0,
                max_quality: 0.0,
                total_episodes: 0,
                degradation_detected: false,
                degradation_threshold: threshold,
            });
        }

        let qualities: Vec<f64> = episodes.iter().map(|e| e.quality_score).collect();
        let sum: f64 = qualities.iter().sum();
        let mean = sum / qualities.len() as f64;
        let min = qualities.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = qualities.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        Ok(QualityReport {
            mean_quality: mean,
            min_quality: min,
            max_quality: max,
            total_episodes: episodes.len(),
            degradation_detected: mean < threshold,
            degradation_threshold: threshold,
        })
    }
}

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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Convert a CozoDB result row to a `DescEpisode`.
///
/// Expected column order (join of desc_episodes + desc_episode_metadata):
/// 0: episode_id, 1: session_id, 2: description, 3: outcome, 4: reward,
/// 5: tags, 6: created_at, 7: task_type, 8: solution, 9: quality_score,
/// 10: trajectory_id, 11: updated_at
fn row_to_episode(row: &[DataValue]) -> DescEpisode {
    let tags: Vec<String> = match &row[5] {
        DataValue::List(list) => list
            .iter()
            .filter_map(|v| v.get_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };

    let trajectory_id = row[10]
        .get_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    DescEpisode {
        episode_id: row[0].get_str().unwrap_or("").to_string(),
        session_id: row[1].get_str().unwrap_or("").to_string(),
        description: row[2].get_str().unwrap_or("").to_string(),
        outcome: row[3].get_str().unwrap_or("").to_string(),
        reward: row[4].get_float().unwrap_or(0.0),
        tags,
        created_at: row[6].get_int().unwrap_or(0) as u64,
        task_type: row[7].get_str().unwrap_or("").to_string(),
        solution: row[8].get_str().unwrap_or("").to_string(),
        quality_score: row[9].get_float().unwrap_or(0.0),
        trajectory_id,
        updated_at: row[11].get_int().unwrap_or(0) as u64,
    }
}

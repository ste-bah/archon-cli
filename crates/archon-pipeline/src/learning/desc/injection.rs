use super::store::DescEpisodeStore;
use super::types::{EpisodeQuery, InjectionResult, RankedEpisode};
use anyhow::Result;

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

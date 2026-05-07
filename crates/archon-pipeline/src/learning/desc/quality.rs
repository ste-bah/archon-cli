use super::store::DescEpisodeStore;
use super::types::{EpisodeQuery, QualityReport};
use anyhow::Result;

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

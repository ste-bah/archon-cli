//! Training history manager — records training runs and provides statistics.

use serde::{Deserialize, Serialize};

/// Configuration snapshot for a training run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingRunConfig {
    pub learning_rate: f32,
    pub epochs: usize,
    pub batch_size: usize,
    pub margin: f32,
    pub ewc_lambda: f32,
}

/// Metrics from a completed training run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingRunMetrics {
    pub final_loss: f32,
    pub best_loss: f32,
    pub final_val_loss: Option<f32>,
    pub epochs_completed: usize,
    pub early_stopped: bool,
    pub duration_secs: f64,
}

/// A single training run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingRunRecord {
    pub run_id: String,
    pub config: TrainingRunConfig,
    pub metrics: TrainingRunMetrics,
    pub timestamp: u64,
}

/// Aggregate statistics across all recorded runs.
#[derive(Debug, Clone)]
pub struct TrainingStats {
    pub total_runs: usize,
    pub avg_final_loss: f32,
    pub best_loss_ever: f32,
    pub avg_epochs: f32,
    pub total_training_time_secs: f64,
    pub early_stop_rate: f32,
}

/// Manages training run history.
pub struct TrainingHistoryManager {
    records: Vec<TrainingRunRecord>,
    max_records: usize,
}

impl TrainingHistoryManager {
    /// Create a new history manager with a maximum record count.
    pub fn new(max_records: usize) -> Self {
        Self {
            records: Vec::new(),
            max_records,
        }
    }

    /// Record a completed training run.
    pub fn record_run(&mut self, config: TrainingRunConfig, metrics: TrainingRunMetrics) -> String {
        let run_id = uuid::Uuid::new_v4().to_string();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let record = TrainingRunRecord {
            run_id: run_id.clone(),
            config,
            metrics,
            timestamp,
        };

        self.records.push(record);

        // Evict oldest if over capacity
        if self.records.len() > self.max_records {
            self.records.remove(0);
        }

        run_id
    }

    /// Get aggregate statistics across all runs.
    pub fn get_stats(&self) -> TrainingStats {
        if self.records.is_empty() {
            return TrainingStats {
                total_runs: 0,
                avg_final_loss: 0.0,
                best_loss_ever: f32::INFINITY,
                avg_epochs: 0.0,
                total_training_time_secs: 0.0,
                early_stop_rate: 0.0,
            };
        }

        let n = self.records.len() as f32;
        let avg_final_loss: f32 = self
            .records
            .iter()
            .map(|r| r.metrics.final_loss)
            .sum::<f32>()
            / n;
        let best_loss_ever = self
            .records
            .iter()
            .map(|r| r.metrics.best_loss)
            .fold(f32::INFINITY, f32::min);
        let avg_epochs: f32 = self
            .records
            .iter()
            .map(|r| r.metrics.epochs_completed as f32)
            .sum::<f32>()
            / n;
        let total_time: f64 = self.records.iter().map(|r| r.metrics.duration_secs).sum();
        let early_stops = self
            .records
            .iter()
            .filter(|r| r.metrics.early_stopped)
            .count() as f32;

        TrainingStats {
            total_runs: self.records.len(),
            avg_final_loss,
            best_loss_ever,
            avg_epochs,
            total_training_time_secs: total_time,
            early_stop_rate: early_stops / n,
        }
    }

    /// Get the most recent N training runs.
    pub fn get_recent_runs(&self, n: usize) -> &[TrainingRunRecord] {
        let start = self.records.len().saturating_sub(n);
        &self.records[start..]
    }

    /// Get all records.
    pub fn all_records(&self) -> &[TrainingRunRecord] {
        &self.records
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

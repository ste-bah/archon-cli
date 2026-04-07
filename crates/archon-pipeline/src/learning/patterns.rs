//! PatternStore — in-memory pattern storage with CRUD, duplicate detection,
//! EMA update, and pruning (TASK-PIPE-F02).
//!
//! Implements REQ-LEARN-006. Will be backed by CozoDB when TASK-PIPE-F10 wires up.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::sona::cosine_similarity;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// EMA smoothing factor for success rate updates.
pub const EMA_ALPHA: f64 = 0.2;

/// Cosine similarity threshold above which a pattern is considered duplicate.
pub const DUPLICATE_SIMILARITY_THRESHOLD: f64 = 0.95;

/// Minimum allowed initial success rate.
pub const MIN_INITIAL_SUCCESS_RATE: f64 = 0.1;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Task type classification for patterns.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskType {
    Coding,
    Research,
    Analysis,
    Planning,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Coding => write!(f, "Coding"),
            Self::Research => write!(f, "Research"),
            Self::Analysis => write!(f, "Analysis"),
            Self::Planning => write!(f, "Planning"),
        }
    }
}

/// A stored reasoning pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub task_type: TaskType,
    pub template: String,
    pub embedding: Vec<f64>,
    pub success_rate: f64,
    pub sona_weight: f64,
    pub usage_count: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Parameters for creating a new pattern.
pub struct CreatePatternParams {
    pub task_type: TaskType,
    pub template: String,
    pub embedding: Vec<f64>,
    pub initial_success_rate: f64,
}

/// Parameters for pruning low-quality patterns.
pub struct PruneParams {
    pub min_success_rate: f64,
    pub min_usage_count: u64,
}

/// Result of a prune operation.
pub struct PruneResult {
    pub pruned_count: usize,
}

/// Aggregate statistics for the pattern store.
pub struct PatternStats {
    pub total_patterns: usize,
    pub by_task_type: HashMap<String, usize>,
}

// ---------------------------------------------------------------------------
// PatternStore
// ---------------------------------------------------------------------------

/// In-memory pattern store with duplicate detection and pruning.
pub struct PatternStore {
    patterns: HashMap<String, Pattern>,
}

impl PatternStore {
    pub fn new() -> Self {
        Self {
            patterns: HashMap::new(),
        }
    }

    /// Create a new pattern. Rejects if initial_success_rate < 0.1 or duplicate detected.
    pub fn create_pattern(
        &mut self,
        params: CreatePatternParams,
    ) -> Result<Pattern, String> {
        if params.initial_success_rate < MIN_INITIAL_SUCCESS_RATE {
            return Err(format!(
                "initial_success_rate {} below minimum {}",
                params.initial_success_rate, MIN_INITIAL_SUCCESS_RATE
            ));
        }

        // Duplicate detection: same task type + cosine > 0.95
        for existing in self.patterns.values() {
            if existing.task_type == params.task_type {
                let sim = cosine_similarity(&existing.embedding, &params.embedding);
                if sim > DUPLICATE_SIMILARITY_THRESHOLD {
                    return Err(format!(
                        "duplicate pattern detected (cosine similarity {:.4} > {})",
                        sim, DUPLICATE_SIMILARITY_THRESHOLD
                    ));
                }
            }
        }

        let now = epoch_secs();
        let pattern = Pattern {
            id: uuid::Uuid::new_v4().to_string(),
            task_type: params.task_type,
            template: params.template,
            embedding: params.embedding,
            success_rate: params.initial_success_rate,
            sona_weight: 1.0,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        };

        self.patterns.insert(pattern.id.clone(), pattern.clone());
        Ok(pattern)
    }

    /// Get a pattern by ID.
    pub fn get_pattern(&self, id: &str) -> Option<&Pattern> {
        self.patterns.get(id)
    }

    /// Get all patterns matching a task type.
    pub fn get_patterns_by_task_type(&self, task_type: &TaskType) -> Vec<&Pattern> {
        self.patterns
            .values()
            .filter(|p| &p.task_type == task_type)
            .collect()
    }

    /// Alias for get_patterns_by_task_type — used by ReasoningBank.
    pub fn find_by_type(&self, task_type: &TaskType) -> Vec<&Pattern> {
        self.get_patterns_by_task_type(task_type)
    }

    /// Return all stored patterns.
    pub fn all(&self) -> Vec<&Pattern> {
        self.patterns.values().collect()
    }

    /// Update success rate using EMA: new = alpha * value + (1 - alpha) * old.
    pub fn update_success_rate(&mut self, id: &str, new_value: f64) {
        if let Some(pattern) = self.patterns.get_mut(id) {
            pattern.success_rate =
                EMA_ALPHA * new_value + (1.0 - EMA_ALPHA) * pattern.success_rate;
            pattern.updated_at = epoch_secs();
        }
    }

    /// Update SONA weight for a pattern.
    pub fn update_sona_weight(&mut self, id: &str, weight: f64) {
        if let Some(pattern) = self.patterns.get_mut(id) {
            pattern.sona_weight = weight;
            pattern.updated_at = epoch_secs();
        }
    }

    /// Increment usage count.
    pub fn increment_usage(&mut self, id: &str) {
        if let Some(pattern) = self.patterns.get_mut(id) {
            pattern.usage_count += 1;
            pattern.updated_at = epoch_secs();
        }
    }

    /// Delete a pattern by ID. Returns true if it existed.
    pub fn delete_pattern(&mut self, id: &str) -> bool {
        self.patterns.remove(id).is_some()
    }

    /// Prune patterns with success_rate < min AND usage_count >= min.
    pub fn prune(&mut self, params: PruneParams) -> PruneResult {
        let to_remove: Vec<String> = self
            .patterns
            .values()
            .filter(|p| {
                p.success_rate < params.min_success_rate
                    && p.usage_count >= params.min_usage_count
            })
            .map(|p| p.id.clone())
            .collect();

        let pruned_count = to_remove.len();
        for id in to_remove {
            self.patterns.remove(&id);
        }

        PruneResult { pruned_count }
    }

    /// Total number of patterns.
    pub fn count(&self) -> usize {
        self.patterns.len()
    }

    /// Aggregate statistics.
    pub fn stats(&self) -> PatternStats {
        let mut by_task_type: HashMap<String, usize> = HashMap::new();
        for pattern in self.patterns.values() {
            *by_task_type.entry(pattern.task_type.to_string()).or_insert(0) += 1;
        }
        PatternStats {
            total_patterns: self.patterns.len(),
            by_task_type,
        }
    }
}

impl Default for PatternStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

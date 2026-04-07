//! ReasoningBank — unified reasoning orchestrator with 4 modes.
//!
//! Implements REQ-LEARN-005.
//! Modes: PatternMatch, CausalInference, Contextual, Hybrid.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::confidence;
use super::patterns::{PatternStore, TaskType};
use super::sona::SonaEngine;

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

/// Reasoning mode selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningMode {
    PatternMatch,
    CausalInference,
    Contextual,
    Hybrid,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the ReasoningBank.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningBankConfig {
    pub pattern_weight: f64,
    pub causal_weight: f64,
    pub contextual_weight: f64,
    pub default_max_results: usize,
    pub default_confidence_threshold: f64,
    pub default_min_l_score: f64,
    pub enable_trajectory_tracking: bool,
    pub enable_auto_mode_selection: bool,
}

impl Default for ReasoningBankConfig {
    fn default() -> Self {
        Self {
            pattern_weight: 0.3,
            causal_weight: 0.3,
            contextual_weight: 0.4,
            default_max_results: 10,
            default_confidence_threshold: 0.7,
            default_min_l_score: 0.5,
            enable_trajectory_tracking: true,
            enable_auto_mode_selection: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// A reasoning request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningRequest {
    pub query: String,
    pub query_embedding: Option<Vec<f64>>,
    pub mode: Option<ReasoningMode>,
    pub task_type: Option<TaskType>,
    pub max_results: Option<usize>,
    pub confidence_threshold: Option<f64>,
}

/// A single pattern match result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatchResult {
    pub pattern_id: String,
    pub template: String,
    pub confidence: f64,
}

/// A single inference result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResult {
    pub label: String,
    pub confidence: f64,
    pub reasoning_path: Vec<String>,
}

/// Provenance information for a reasoning result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceInfo {
    pub source: String,
    pub mode: ReasoningMode,
    pub timestamp: u64,
}

/// A reasoning response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningResponse {
    pub mode_used: ReasoningMode,
    pub patterns: Vec<PatternMatchResult>,
    pub inferences: Vec<InferenceResult>,
    pub overall_confidence: f64,
    pub provenance: Vec<ProvenanceInfo>,
    pub trajectory_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Trajectory types
// ---------------------------------------------------------------------------

/// A recorded reasoning trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryRecord {
    pub trajectory_id: String,
    pub mode: ReasoningMode,
    pub query: String,
    pub steps: Vec<String>,
    pub result_count: usize,
    pub confidence: f64,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Placeholder dependencies
// ---------------------------------------------------------------------------

// GNNEnhancer is provided by super::gnn (wired in F07).
use super::gnn::GNNEnhancer;

// CausalMemory is now provided by super::causal (wired in F04).
use super::causal::CausalMemory;

/// Dependencies injected into ReasoningBank.
pub struct ReasoningBankDeps {
    pub pattern_store: PatternStore,
    pub causal_memory: Option<CausalMemory>,
    pub gnn_enhancer: Option<GNNEnhancer>,
    pub sona_engine: Option<SonaEngine>,
    pub config: ReasoningBankConfig,
}

// ---------------------------------------------------------------------------
// ReasoningBank
// ---------------------------------------------------------------------------

/// Unified reasoning orchestrator — routes queries through 4 reasoning modes.
pub struct ReasoningBank {
    pattern_store: PatternStore,
    causal_memory: Option<CausalMemory>,
    gnn_enhancer: Option<GNNEnhancer>,
    /// Retained for future direct trajectory feedback — currently consumed
    /// indirectly through [`LearningIntegration`] which holds its own `SonaEngine`.
    #[allow(dead_code)]
    sona_engine: Option<SonaEngine>,
    config: ReasoningBankConfig,
    trajectory_records: Vec<TrajectoryRecord>,
}

impl ReasoningBank {
    /// Construct a new ReasoningBank from injected dependencies.
    pub fn new(deps: ReasoningBankDeps) -> Self {
        Self {
            pattern_store: deps.pattern_store,
            causal_memory: deps.causal_memory,
            gnn_enhancer: deps.gnn_enhancer,
            sona_engine: deps.sona_engine,
            config: deps.config,
            trajectory_records: Vec::new(),
        }
    }

    /// Main reasoning entry point. Selects mode automatically if not specified.
    pub fn reason(&mut self, request: &ReasoningRequest) -> ReasoningResponse {
        let mode = request.mode.unwrap_or_else(|| {
            if self.config.enable_auto_mode_selection {
                ModeSelector::select(&request.query)
            } else {
                ReasoningMode::Hybrid
            }
        });

        let max_results = request
            .max_results
            .unwrap_or(self.config.default_max_results);
        let threshold = request
            .confidence_threshold
            .unwrap_or(self.config.default_confidence_threshold);

        let response = match mode {
            ReasoningMode::PatternMatch => {
                self.reason_pattern_match(request, max_results, threshold)
            }
            ReasoningMode::CausalInference => self.reason_causal(request, max_results),
            ReasoningMode::Contextual => {
                self.reason_contextual(request, max_results, threshold)
            }
            ReasoningMode::Hybrid => self.reason_hybrid(request, max_results, threshold),
        };

        if self.config.enable_trajectory_tracking {
            let record = TrajectoryTracker::record(&request.query, mode, &response);
            self.trajectory_records.push(record);
        }

        response
    }

    /// Return all recorded trajectory records.
    pub fn trajectories(&self) -> &[TrajectoryRecord] {
        &self.trajectory_records
    }

    // -----------------------------------------------------------------------
    // Private mode implementations
    // -----------------------------------------------------------------------

    fn reason_pattern_match(
        &self,
        request: &ReasoningRequest,
        max_results: usize,
        threshold: f64,
    ) -> ReasoningResponse {
        let task_type = request
            .task_type
            .clone()
            .unwrap_or(TaskType::Coding);
        let patterns = self.pattern_store.find_by_type(&task_type);

        let mut results: Vec<PatternMatchResult> = patterns
            .iter()
            .map(|p| {
                let conf = if let Some(ref emb) = request.query_embedding {
                    let sim = cosine_sim_f64(emb, &p.embedding);
                    confidence::calculate_confidence(sim, p.success_rate, p.sona_weight)
                } else {
                    confidence::calculate_confidence(0.5, p.success_rate, p.sona_weight)
                };
                PatternMatchResult {
                    pattern_id: p.id.clone(),
                    template: p.template.clone(),
                    confidence: conf,
                }
            })
            .filter(|r| r.confidence >= threshold)
            .collect();

        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(max_results);

        let overall = results.first().map(|r| r.confidence).unwrap_or(0.0);

        ReasoningResponse {
            mode_used: ReasoningMode::PatternMatch,
            patterns: results,
            inferences: vec![],
            overall_confidence: overall,
            provenance: vec![ProvenanceInfo {
                source: "pattern_store".into(),
                mode: ReasoningMode::PatternMatch,
                timestamp: now_epoch(),
            }],
            trajectory_id: None,
        }
    }

    fn reason_causal(
        &self,
        request: &ReasoningRequest,
        max_results: usize,
    ) -> ReasoningResponse {
        let causal = match self.causal_memory.as_ref() {
            Some(cm) => cm,
            None => {
                warn!("CausalMemory not available -- causal-inference mode returning empty");
                return ReasoningResponse {
                    mode_used: ReasoningMode::CausalInference,
                    patterns: vec![],
                    inferences: vec![],
                    overall_confidence: 0.0,
                    provenance: vec![ProvenanceInfo {
                        source: "causal_memory_none".into(),
                        mode: ReasoningMode::CausalInference,
                        timestamp: now_epoch(),
                    }],
                    trajectory_id: None,
                };
            }
        };

        // Use the query string as the node to look up in the causal graph.
        let result = causal.infer_causation(&request.query);

        // Convert causes and effects into InferenceResult entries.
        let mut inferences: Vec<InferenceResult> = Vec::new();

        if !result.causes.is_empty() {
            inferences.push(InferenceResult {
                label: format!("causes of '{}'", request.query),
                confidence: result.confidence,
                reasoning_path: result.causes.clone(),
            });
        }

        if !result.effects.is_empty() {
            inferences.push(InferenceResult {
                label: format!("effects of '{}'", request.query),
                confidence: result.confidence,
                reasoning_path: result.effects.clone(),
            });
        }

        if !result.chain.is_empty() {
            inferences.push(InferenceResult {
                label: format!("causal chain through '{}'", request.query),
                confidence: result.confidence,
                reasoning_path: result.chain.clone(),
            });
        }

        inferences.truncate(max_results);

        ReasoningResponse {
            mode_used: ReasoningMode::CausalInference,
            patterns: vec![],
            inferences,
            overall_confidence: result.confidence,
            provenance: vec![ProvenanceInfo {
                source: "causal_memory".into(),
                mode: ReasoningMode::CausalInference,
                timestamp: now_epoch(),
            }],
            trajectory_id: None,
        }
    }

    fn reason_contextual(
        &self,
        request: &ReasoningRequest,
        max_results: usize,
        threshold: f64,
    ) -> ReasoningResponse {
        let all_patterns = self.pattern_store.all();

        // When GNNEnhancer is available and a query embedding is provided,
        // enhance the embedding through the 3-layer GNN before similarity matching.
        let (effective_embedding, source) = match (&self.gnn_enhancer, &request.query_embedding) {
            (Some(gnn), Some(emb)) => {
                // Convert f64 embedding to f32 for GNN, enhance, convert back
                let f32_emb: Vec<f32> = emb.iter().map(|&v| v as f32).collect();
                let result = gnn.enhance(&f32_emb);
                let enhanced_f64: Vec<f64> = result.enhanced.iter().map(|&v| v as f64).collect();
                (Some(enhanced_f64), "gnn_enhanced")
            }
            (None, Some(emb)) => (Some(emb.clone()), "raw_embeddings"),
            _ => (None, "raw_embeddings"),
        };

        let mut results: Vec<PatternMatchResult> = if let Some(ref emb) = effective_embedding {
            all_patterns
                .iter()
                .map(|p| {
                    let sim = cosine_sim_f64(emb, &p.embedding);
                    PatternMatchResult {
                        pattern_id: p.id.clone(),
                        template: p.template.clone(),
                        confidence: sim,
                    }
                })
                .filter(|r| r.confidence >= threshold)
                .collect()
        } else {
            vec![]
        };

        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(max_results);

        let overall = results.first().map(|r| r.confidence).unwrap_or(0.0);

        ReasoningResponse {
            mode_used: ReasoningMode::Contextual,
            patterns: results,
            inferences: vec![],
            overall_confidence: overall,
            provenance: vec![ProvenanceInfo {
                source: source.into(),
                mode: ReasoningMode::Contextual,
                timestamp: now_epoch(),
            }],
            trajectory_id: None,
        }
    }

    fn reason_hybrid(
        &mut self,
        request: &ReasoningRequest,
        max_results: usize,
        threshold: f64,
    ) -> ReasoningResponse {
        let pattern_resp = self.reason_pattern_match(request, max_results, threshold);
        let causal_resp = self.reason_causal(request, max_results);
        let contextual_resp = self.reason_contextual(request, max_results, threshold);

        let pw = self.config.pattern_weight;
        let cw = self.config.causal_weight;
        let xw = self.config.contextual_weight;

        // Merge pattern results with weighted confidence using pattern_id as key.
        let mut merged: HashMap<String, PatternMatchResult> = HashMap::new();

        for p in &pattern_resp.patterns {
            merged
                .entry(p.pattern_id.clone())
                .or_insert_with(|| PatternMatchResult {
                    pattern_id: p.pattern_id.clone(),
                    template: p.template.clone(),
                    confidence: 0.0,
                })
                .confidence += p.confidence * pw;
        }

        for p in &contextual_resp.patterns {
            merged
                .entry(p.pattern_id.clone())
                .or_insert_with(|| PatternMatchResult {
                    pattern_id: p.pattern_id.clone(),
                    template: p.template.clone(),
                    confidence: 0.0,
                })
                .confidence += p.confidence * xw;
        }

        // Causal results contribute via cw weight when CausalMemory is available (wired in F04).
        // Causal mode produces inferences, not patterns, so cw affects overall_confidence.
        let _ = cw;

        let mut results: Vec<PatternMatchResult> = merged.into_values().collect();
        results.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(max_results);

        let overall = pw * pattern_resp.overall_confidence
            + cw * causal_resp.overall_confidence
            + xw * contextual_resp.overall_confidence;

        ReasoningResponse {
            mode_used: ReasoningMode::Hybrid,
            patterns: results,
            inferences: causal_resp.inferences,
            overall_confidence: overall,
            provenance: vec![
                ProvenanceInfo {
                    source: "pattern_store".into(),
                    mode: ReasoningMode::PatternMatch,
                    timestamp: now_epoch(),
                },
                ProvenanceInfo {
                    source: if self.causal_memory.is_some() {
                        "causal_memory".into()
                    } else {
                        "causal_memory_none".into()
                    },
                    mode: ReasoningMode::CausalInference,
                    timestamp: now_epoch(),
                },
                ProvenanceInfo {
                    source: "raw_embeddings".into(),
                    mode: ReasoningMode::Contextual,
                    timestamp: now_epoch(),
                },
            ],
            trajectory_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ModeSelector
// ---------------------------------------------------------------------------

/// Selects the appropriate reasoning mode based on query keywords.
pub struct ModeSelector;

impl ModeSelector {
    /// Auto-select reasoning mode based on query keywords.
    pub fn select(query: &str) -> ReasoningMode {
        let lower = query.to_lowercase();

        // Causal queries
        if lower.starts_with("why ")
            || lower.contains("because")
            || lower.contains("caused by")
            || lower.contains("root cause")
            || lower.contains("what caused")
        {
            return ReasoningMode::CausalInference;
        }

        // Contextual / similarity queries
        if lower.contains("similar to")
            || lower.contains("like this")
            || lower.contains("resembles")
            || lower.contains("related code")
            || lower.contains("find similar")
        {
            return ReasoningMode::Contextual;
        }

        // Pattern-match queries
        if lower.starts_with("how to")
            || lower.starts_with("how do")
            || lower.contains("best practice")
            || lower.contains("pattern for")
            || lower.contains("template for")
            || lower.contains("example of")
        {
            return ReasoningMode::PatternMatch;
        }

        // Default: hybrid
        ReasoningMode::Hybrid
    }
}

// ---------------------------------------------------------------------------
// TrajectoryTracker
// ---------------------------------------------------------------------------

/// Records reasoning execution paths for observability and future replay.
pub struct TrajectoryTracker;

impl TrajectoryTracker {
    /// Create a TrajectoryRecord from a completed reasoning response.
    pub fn record(
        query: &str,
        mode: ReasoningMode,
        response: &ReasoningResponse,
    ) -> TrajectoryRecord {
        TrajectoryRecord {
            trajectory_id: uuid::Uuid::new_v4().to_string(),
            mode,
            query: query.to_string(),
            steps: vec![
                format!("mode_selected: {:?}", mode),
                format!(
                    "results_count: {}",
                    response.patterns.len() + response.inferences.len()
                ),
                format!("confidence: {:.4}", response.overall_confidence),
            ],
            result_count: response.patterns.len() + response.inferences.len(),
            confidence: response.overall_confidence,
            timestamp: now_epoch(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cosine_sim_f64(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    (dot / (mag_a * mag_b)).clamp(0.0, 1.0)
}

//! ReasoningBank — unified reasoning orchestrator with 14 modes.
//!
//! Implements REQ-LEARN-005.
//! Modes: Deductive, Inductive, Abductive, Analogical, Adversarial,
//! Counterfactual, Temporal, Constraint, Decomposition, FirstPrinciples,
//! Causal, Contextual, PatternMatch, Hybrid.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::confidence;
use super::patterns::{PatternStore, TaskType};
use super::sona::SonaEngine;

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

/// Reasoning mode selection — 12 spec modes + 2 meta-modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningMode {
    // Core 4 (formal logical reasoning)
    Deductive,  // general rules → specific conclusions
    Inductive,  // specific observations → general rules
    Abductive,  // best explanation for observations
    Analogical, // structural similarity transfer

    // Extended 8 (specialized reasoning paradigms)
    Adversarial,     // counterexample / red-team
    Counterfactual,  // alternate-outcome "what if"
    Temporal,        // time-aware sequence
    Constraint,      // constraint satisfaction
    Decomposition,   // sub-problem breakdown
    FirstPrinciples, // axiom-based derivation
    #[serde(alias = "CausalInference")]
    Causal, // cause-effect (renamed from CausalInference)
    Contextual,      // context-aware

    // Meta-modes (not on the "12" list, kept as orchestrators)
    PatternMatch, // legacy LLM-based template matching
    Hybrid,       // auto-aggregator across modes
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the ReasoningBank.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningBankConfig {
    // Core 4 weights
    pub deductive_weight: f64,
    pub inductive_weight: f64,
    pub abductive_weight: f64,
    pub analogical_weight: f64,
    // Extended 8 weights
    pub adversarial_weight: f64,
    pub counterfactual_weight: f64,
    pub temporal_weight: f64,
    pub constraint_weight: f64,
    pub decomposition_weight: f64,
    pub first_principles_weight: f64,
    pub causal_weight: f64,
    pub contextual_weight: f64,
    // Legacy
    pub pattern_weight: f64,
    pub default_max_results: usize,
    pub default_confidence_threshold: f64,
    pub default_min_l_score: f64,
    pub enable_trajectory_tracking: bool,
    pub enable_auto_mode_selection: bool,
}

impl Default for ReasoningBankConfig {
    fn default() -> Self {
        Self {
            deductive_weight: 1.0,
            inductive_weight: 1.0,
            abductive_weight: 1.0,
            analogical_weight: 1.0,
            adversarial_weight: 1.0,
            counterfactual_weight: 1.0,
            temporal_weight: 1.0,
            constraint_weight: 1.0,
            decomposition_weight: 1.0,
            first_principles_weight: 1.0,
            causal_weight: 1.0,
            contextual_weight: 1.0,
            pattern_weight: 0.5,
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningRequest {
    pub query: String,
    pub query_embedding: Option<Vec<f64>>,
    pub mode: Option<ReasoningMode>,
    pub task_type: Option<TaskType>,
    pub max_results: Option<usize>,
    pub confidence_threshold: Option<f64>,
    /// Optional context strings for extended reasoning engines.
    pub context: Option<Vec<String>>,
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
    /// Arbitrary metadata keyed by engine (e.g. engine_name → "deductive").
    pub context_metadata: HashMap<String, String>,
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
use crate::learning::modes;

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

/// Unified reasoning orchestrator — routes queries through 14 reasoning modes.
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
            ReasoningMode::Causal => self.reason_causal(request, max_results),
            ReasoningMode::Contextual => self.reason_contextual(request, max_results, threshold),
            ReasoningMode::Hybrid => self.reason_hybrid(request, max_results, threshold),
            ReasoningMode::Deductive => self.reason_via_engine(
                &modes::deductive::DeductiveEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Inductive => self.reason_via_engine(
                &modes::inductive::InductiveEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Abductive => self.reason_via_engine(
                &modes::abductive::AbductiveEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Analogical => self.reason_via_engine(
                &modes::analogical::AnalogicalEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Adversarial => self.reason_via_engine(
                &modes::adversarial::AdversarialEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Counterfactual => self.reason_via_engine(
                &modes::counterfactual::CounterfactualEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Temporal => self.reason_via_engine(
                &modes::temporal::TemporalEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Constraint => self.reason_via_engine(
                &modes::constraint::ConstraintEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::Decomposition => self.reason_via_engine(
                &modes::decomposition::DecompositionEngine::new(),
                request,
                max_results,
            ),
            ReasoningMode::FirstPrinciples => self.reason_via_engine(
                &modes::first_principles::FirstPrinciplesEngine::new(),
                request,
                max_results,
            ),
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
    // Engine dispatch helpers
    // -----------------------------------------------------------------------

    /// Dispatch a reasoning request to a [`ReasoningEngine`] and convert the output.
    fn reason_via_engine(
        &self,
        engine: &dyn modes::ReasoningEngine,
        request: &ReasoningRequest,
        max_results: usize,
    ) -> ReasoningResponse {
        let mode_request = modes::ReasoningRequest {
            query: request.query.clone(),
            context: request.context.clone().unwrap_or_default(),
            parameters: std::collections::HashMap::new(),
        };

        let output = engine.reason(&mode_request).unwrap_or_else(|e| {
            tracing::warn!(engine = engine.name(), error = %e, "Reasoning engine failed");
            modes::ReasoningOutput {
                engine_name: engine.name().to_string(),
                result_type: modes::ResultType::ContextualInsights,
                items: vec![],
                confidence: 0.0,
                provenance: vec![],
            }
        });

        self.engine_output_to_response(output, max_results)
    }

    /// Convert a [`modes::ReasoningOutput`] into a [`ReasoningResponse`].
    fn engine_output_to_response(
        &self,
        output: modes::ReasoningOutput,
        max_results: usize,
    ) -> ReasoningResponse {
        let mode = match output.engine_name.as_str() {
            "deductive" => ReasoningMode::Deductive,
            "inductive" => ReasoningMode::Inductive,
            "abductive" => ReasoningMode::Abductive,
            "analogical" => ReasoningMode::Analogical,
            "adversarial" => ReasoningMode::Adversarial,
            "counterfactual" => ReasoningMode::Counterfactual,
            "temporal" => ReasoningMode::Temporal,
            "constraint" => ReasoningMode::Constraint,
            "decomposition" => ReasoningMode::Decomposition,
            "first_principles" => ReasoningMode::FirstPrinciples,
            "causal" => ReasoningMode::Causal,
            "contextual" => ReasoningMode::Contextual,
            _ => ReasoningMode::Hybrid,
        };

        let mut patterns: Vec<PatternMatchResult> = output
            .items
            .iter()
            .take(max_results)
            .map(|item| PatternMatchResult {
                pattern_id: item.label.clone(),
                template: item.description.clone(),
                confidence: item.confidence,
            })
            .collect();

        // Flow provenance strings into context_metadata
        let mut context_metadata = HashMap::new();
        context_metadata.insert("engine_name".to_string(), output.engine_name.clone());
        if !output.provenance.is_empty() {
            context_metadata.insert("provenance".to_string(), output.provenance.join(", "));
        }

        let overall = if patterns.is_empty() {
            output.confidence
        } else {
            patterns.iter().map(|p| p.confidence).sum::<f64>() / patterns.len() as f64
        };

        // Sort by confidence descending
        patterns.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        ReasoningResponse {
            mode_used: mode,
            patterns,
            inferences: vec![],
            overall_confidence: overall,
            provenance: vec![ProvenanceInfo {
                source: output.engine_name.clone(),
                mode,
                timestamp: now_epoch(),
            }],
            trajectory_id: None,
            context_metadata,
        }
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
        let task_type = request.task_type.clone().unwrap_or(TaskType::Coding);
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
            context_metadata: HashMap::new(),
        }
    }

    fn reason_causal(&self, request: &ReasoningRequest, max_results: usize) -> ReasoningResponse {
        let causal = match self.causal_memory.as_ref() {
            Some(cm) => cm,
            None => {
                warn!("CausalMemory not available -- causal-inference mode returning empty");
                return ReasoningResponse {
                    mode_used: ReasoningMode::Causal,
                    patterns: vec![],
                    inferences: vec![],
                    overall_confidence: 0.0,
                    provenance: vec![ProvenanceInfo {
                        source: "causal_memory_none".into(),
                        mode: ReasoningMode::Causal,
                        timestamp: now_epoch(),
                    }],
                    trajectory_id: None,
                    context_metadata: HashMap::new(),
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
            mode_used: ReasoningMode::Causal,
            patterns: vec![],
            inferences,
            overall_confidence: result.confidence,
            provenance: vec![ProvenanceInfo {
                source: "causal_memory".into(),
                mode: ReasoningMode::Causal,
                timestamp: now_epoch(),
            }],
            trajectory_id: None,
            context_metadata: HashMap::new(),
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
                let result = gnn.enhance(&f32_emb, None, None, false);
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
            context_metadata: HashMap::new(),
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
                    mode: ReasoningMode::Causal,
                    timestamp: now_epoch(),
                },
                ProvenanceInfo {
                    source: "raw_embeddings".into(),
                    mode: ReasoningMode::Contextual,
                    timestamp: now_epoch(),
                },
            ],
            trajectory_id: None,
            context_metadata: HashMap::new(),
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
        let q = query.to_lowercase();

        // Decomposition: "break down", "steps", "subtasks" (must precede "break")
        if q.contains("break down") || q.contains("steps") || q.contains("subtasks") {
            return ReasoningMode::Decomposition;
        }

        // Counterfactual: "what if", "had X been", "suppose X were"
        if q.contains("what if")
            || q.contains("suppose")
            || (q.contains("had ") && q.contains(" been"))
        {
            return ReasoningMode::Counterfactual;
        }

        // Adversarial: "break", "attack", "exploit", "fail", "edge case"
        if (q.contains("break") && !q.contains("break down"))
            || q.contains("attack")
            || q.contains("exploit")
            || q.contains("edge case")
        {
            return ReasoningMode::Adversarial;
        }

        // Temporal: "before", "after", "since", "when did", "sequence"
        if q.contains("before ")
            || q.contains("after ")
            || q.contains("when did")
            || q.contains("sequence")
        {
            return ReasoningMode::Temporal;
        }

        // Constraint: "must", "constraint", "require", "satisfy"
        if q.contains("constraint") || q.contains("must satisfy") || q.contains("requirement") {
            return ReasoningMode::Constraint;
        }

        // Abductive: "why did", "best explanation", "diagnose"
        if q.contains("why did") || q.contains("best explanation") || q.contains("diagnose") {
            return ReasoningMode::Abductive;
        }

        // Analogical: "similar to", "like", "analogous"
        if q.contains("similar to") || q.contains("analogous") || q.contains("like the ") {
            return ReasoningMode::Analogical;
        }

        // Inductive: "pattern", "general rule", "always", "usually"
        if q.contains("pattern") || q.contains("general rule") || q.contains("always") {
            return ReasoningMode::Inductive;
        }

        // Deductive: "if X then Y", "rule states", "implies"
        if q.contains("therefore") || q.contains("implies") || q.contains("rule states") {
            return ReasoningMode::Deductive;
        }

        // First principles: "from scratch", "fundamentals", "axioms"
        if q.contains("from scratch")
            || q.contains("fundamentals")
            || q.contains("first principles")
        {
            return ReasoningMode::FirstPrinciples;
        }

        // Causal: "cause", "because"
        if q.contains("cause") || q.contains("because") {
            return ReasoningMode::Causal;
        }

        // Contextual: "context", "when"
        if q.contains("context") || q.contains("when") {
            return ReasoningMode::Contextual;
        }

        // Pattern match: "similar pattern", "template"
        if q.contains("similar pattern") || q.contains("template") {
            return ReasoningMode::PatternMatch;
        }

        // Default: Hybrid (aggregator over all)
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

//! Private reasoning mode implementations — PatternMatch, Causal, Contextual, Hybrid.

use std::collections::HashMap;

use tracing::warn;

use super::super::confidence;
use super::super::patterns::TaskType;
use super::cosine_sim_f64;
use super::{
    InferenceResult, PatternMatchResult, ProvenanceInfo, ReasoningMode, ReasoningRequest,
    ReasoningResponse, now_epoch,
};

impl super::ReasoningBank {
    pub(super) fn reason_pattern_match(
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

    pub(super) fn reason_causal(
        &self,
        request: &ReasoningRequest,
        max_results: usize,
    ) -> ReasoningResponse {
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

        let result = causal.infer_causation(&request.query);

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

    pub(super) fn reason_contextual(
        &self,
        request: &ReasoningRequest,
        max_results: usize,
        threshold: f64,
    ) -> ReasoningResponse {
        let all_patterns = self.pattern_store.all();

        let (effective_embedding, source) = match (&self.gnn_enhancer, &request.query_embedding) {
            (Some(gnn), Some(emb)) => {
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

    pub(super) fn reason_hybrid(
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

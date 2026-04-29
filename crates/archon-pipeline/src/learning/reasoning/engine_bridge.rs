//! Bridge between ReasoningBank and the 12 engine modules.

use std::collections::HashMap;

use super::{
    PatternMatchResult, ProvenanceInfo, ReasoningMode, ReasoningRequest, ReasoningResponse,
    now_epoch,
};
use crate::learning::modes;

impl super::ReasoningBank {
    /// Dispatch a reasoning request to a [`ReasoningEngine`] and convert the output.
    pub(super) fn reason_via_engine(
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
    pub(super) fn engine_output_to_response(
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
}

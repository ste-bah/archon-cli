//! Abductive reasoning engine — backward causal inference.
//!
//! Extracts observed effects from context, generates candidate hypotheses
//! (cause combinations), and scores them by coverage, parsimony, and prior
//! probability.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Abductive reasoning: inference to the best explanation.
pub struct AbductiveEngine;

impl AbductiveEngine {
    pub fn new() -> Self {
        Self
    }

    /// Extract observed effects from context lines.
    fn extract_effects(context: &[String]) -> Vec<String> {
        context
            .iter()
            .filter(|c| !c.trim().is_empty())
            .map(|c| c.trim().to_string())
            .collect()
    }

    /// Generate candidate hypotheses that could explain the observed effects.
    /// Each hypothesis is a potential cause; we create single-cause and
    /// multi-cause combinations (up to pairs).
    fn generate_hypotheses(effects: &[String]) -> Vec<Hypothesis> {
        let mut hypotheses = Vec::new();

        // Single-cause hypotheses: each effect might have a direct cause.
        for (i, effect) in effects.iter().enumerate() {
            let cause = format!("Root cause explaining: {}", effect);
            let covered: Vec<usize> = vec![i];
            hypotheses.push(Hypothesis {
                label: format!("H{}", hypotheses.len() + 1),
                causes: vec![cause],
                covered_effects: covered,
            });
        }

        // Combined hypothesis: a single root cause explaining all effects.
        if effects.len() > 1 {
            let cause = format!(
                "Unified root cause explaining all {} observations",
                effects.len()
            );
            let covered: Vec<usize> = (0..effects.len()).collect();
            hypotheses.push(Hypothesis {
                label: format!("H{}", hypotheses.len() + 1),
                causes: vec![cause],
                covered_effects: covered,
            });
        }

        // Pair-wise combined hypotheses for adjacent effects.
        for i in 0..effects.len().saturating_sub(1) {
            let cause = format!(
                "Shared cause for '{}' and '{}'",
                effects[i], effects[i + 1]
            );
            let covered = vec![i, i + 1];
            hypotheses.push(Hypothesis {
                label: format!("H{}", hypotheses.len() + 1),
                causes: vec![cause],
                covered_effects: covered,
            });
        }

        hypotheses
    }

    /// Score a hypothesis by coverage, parsimony, and prior probability.
    fn score_hypothesis(hypothesis: &Hypothesis, total_effects: usize) -> f64 {
        let coverage = if total_effects > 0 {
            hypothesis.covered_effects.len() as f64 / total_effects as f64
        } else {
            0.0
        };

        // Parsimony: fewer causes = higher score.
        let parsimony = 1.0 / (hypothesis.causes.len() as f64);

        // Prior: hypotheses covering more effects get a slight prior boost.
        let prior = 0.5 + 0.5 * coverage;

        // Weighted combination.
        let score = 0.4 * coverage + 0.3 * parsimony + 0.3 * prior;
        score.clamp(0.0, 1.0)
    }
}

struct Hypothesis {
    label: String,
    causes: Vec<String>,
    covered_effects: Vec<usize>,
}

impl ReasoningEngine for AbductiveEngine {
    fn name(&self) -> &str {
        "abductive"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let effects = Self::extract_effects(&request.context);
        let hypotheses = Self::generate_hypotheses(&effects);

        let mut scored: Vec<(Hypothesis, f64)> = hypotheses
            .into_iter()
            .map(|h| {
                let score = Self::score_hypothesis(&h, effects.len());
                (h, score)
            })
            .collect();

        // Sort descending by score.
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let items: Vec<ReasoningItem> = scored
            .iter()
            .map(|(h, score)| {
                let evidence: Vec<String> = h
                    .covered_effects
                    .iter()
                    .filter_map(|&idx| effects.get(idx).cloned())
                    .collect();
                ReasoningItem {
                    label: h.label.clone(),
                    description: h.causes.join("; "),
                    confidence: *score,
                    supporting_evidence: evidence,
                }
            })
            .collect();

        let overall_confidence = items.first().map(|i| i.confidence).unwrap_or(0.0);

        Ok(ReasoningOutput {
            engine_name: "abductive".to_string(),
            result_type: ResultType::Hypotheses,
            items,
            confidence: overall_confidence,
            provenance: vec![format!("query: {}", request.query)],
        })
    }
}

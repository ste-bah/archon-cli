//! Adversarial reasoning engine — counter-argument generation.
//!
//! Takes a claim from the query, generates counterarguments by inverting key
//! assertions, and scores argument strength by evidence quality and logical
//! coherence.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Adversarial reasoning: systematic counter-argument generation.
pub struct AdversarialEngine;

impl Default for AdversarialEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AdversarialEngine {
    pub fn new() -> Self {
        Self
    }

    /// Extract the main claim from the query.
    fn extract_claim(query: &str) -> String {
        // Strip common prefixes like "argue against:" or "counter:".
        let prefixes = ["argue against:", "counter:", "devil's advocate:"];
        let lower = query.to_lowercase();
        for prefix in &prefixes {
            if lower.starts_with(prefix) {
                return query[prefix.len()..].trim().to_string();
            }
        }
        query.trim().to_string()
    }

    /// Generate counterarguments by applying different attack strategies.
    fn generate_counterarguments(claim: &str, context: &[String]) -> Vec<CounterArgument> {
        let mut args = Vec::new();

        // Strategy 1: Direct negation.
        args.push(CounterArgument {
            strategy: "Direct Negation".to_string(),
            argument: format!("The opposite may hold: it is not the case that {}", claim),
            evidence: context.first().cloned().unwrap_or_default(),
            logical_coherence: 0.6,
            evidence_quality: if context.is_empty() { 0.3 } else { 0.6 },
        });

        // Strategy 2: Scope limitation.
        args.push(CounterArgument {
            strategy: "Scope Limitation".to_string(),
            argument: format!(
                "While '{}' may be true in some cases, it does not generalize universally",
                claim
            ),
            evidence: context.get(1).cloned().unwrap_or_default(),
            logical_coherence: 0.8,
            evidence_quality: if context.len() > 1 { 0.7 } else { 0.4 },
        });

        // Strategy 3: Alternative explanation.
        args.push(CounterArgument {
            strategy: "Alternative Explanation".to_string(),
            argument: format!(
                "An alternative explanation exists that better accounts for the evidence than '{}'",
                claim
            ),
            evidence: context.get(2).cloned().unwrap_or_default(),
            logical_coherence: 0.7,
            evidence_quality: if context.len() > 2 { 0.8 } else { 0.3 },
        });

        // Strategy 4: Consequence reductio.
        args.push(CounterArgument {
            strategy: "Reductio ad Absurdum".to_string(),
            argument: format!(
                "If '{}' were true, it would lead to contradictory or absurd consequences",
                claim
            ),
            evidence: context
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join("; "),
            logical_coherence: 0.75,
            evidence_quality: if context.len() >= 2 { 0.65 } else { 0.35 },
        });

        // Strategy 5: Empirical challenge using provided context.
        if !context.is_empty() {
            args.push(CounterArgument {
                strategy: "Empirical Challenge".to_string(),
                argument: format!(
                    "Available evidence contradicts '{}': {}",
                    claim,
                    context.join(", ")
                ),
                evidence: context.join("; "),
                logical_coherence: 0.65,
                evidence_quality: 0.5 + 0.1 * context.len().min(5) as f64,
            });
        }

        args
    }

    /// Score a counterargument by combining evidence quality and logical
    /// coherence.
    fn score_argument(arg: &CounterArgument) -> f64 {
        let score = 0.5 * arg.logical_coherence + 0.5 * arg.evidence_quality;
        score.clamp(0.0, 1.0)
    }
}

struct CounterArgument {
    strategy: String,
    argument: String,
    evidence: String,
    logical_coherence: f64,
    evidence_quality: f64,
}

impl ReasoningEngine for AdversarialEngine {
    fn name(&self) -> &str {
        "adversarial"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let claim = Self::extract_claim(&request.query);
        let args = Self::generate_counterarguments(&claim, &request.context);

        let mut scored: Vec<(CounterArgument, f64)> = args
            .into_iter()
            .map(|a| {
                let score = Self::score_argument(&a);
                (a, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let items: Vec<ReasoningItem> = scored
            .iter()
            .map(|(a, score)| ReasoningItem {
                label: a.strategy.clone(),
                description: a.argument.clone(),
                confidence: *score,
                supporting_evidence: if a.evidence.is_empty() {
                    vec![]
                } else {
                    vec![a.evidence.clone()]
                },
            })
            .collect();

        let overall_confidence = items.first().map(|i| i.confidence).unwrap_or(0.0);

        Ok(ReasoningOutput {
            engine_name: "adversarial".to_string(),
            result_type: ResultType::Arguments,
            items,
            confidence: overall_confidence,
            provenance: vec![format!("claim: {}", claim)],
        })
    }
}

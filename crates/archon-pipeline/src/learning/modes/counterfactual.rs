//! Counterfactual reasoning engine — "what if" analysis.
//!
//! Takes a factual situation and a hypothetical change, propagates effects
//! through available context, identifies changed outcomes, and scores
//! plausibility.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Counterfactual reasoning: exploring hypothetical alternatives.
pub struct CounterfactualEngine;

impl CounterfactualEngine {
    pub fn new() -> Self {
        Self
    }

    /// Extract the hypothetical change from the query.
    fn extract_hypothetical(query: &str) -> String {
        let prefixes = ["what if ", "hypothetically, ", "suppose "];
        let lower = query.to_lowercase();
        for prefix in &prefixes {
            if let Some(pos) = lower.find(prefix) {
                return query[pos + prefix.len()..].trim().to_string();
            }
        }
        query.trim().to_string()
    }

    /// Parse factual context into causal factors.
    fn parse_factors(context: &[String]) -> Vec<CausalFactor> {
        context
            .iter()
            .filter(|c| !c.trim().is_empty())
            .enumerate()
            .map(|(i, c)| {
                let trimmed = c.trim().to_string();
                // Try to extract numeric values for impact estimation.
                let numeric_value = extract_first_number(&trimmed);
                CausalFactor {
                    index: i,
                    description: trimmed,
                    numeric_value,
                }
            })
            .collect()
    }

    /// Propagate the hypothetical change through causal factors to produce
    /// counterfactual outcomes.
    fn propagate_change(
        hypothetical: &str,
        factors: &[CausalFactor],
    ) -> Vec<CounterfactualOutcome> {
        let hyp_lower = hypothetical.to_lowercase();
        let mut outcomes = Vec::new();

        for factor in factors {
            let factor_lower = factor.description.to_lowercase();

            // Compute relevance: how many words are shared between the
            // hypothetical and this factor.
            let hyp_words: std::collections::HashSet<&str> =
                hyp_lower.split_whitespace().collect();
            let factor_words: std::collections::HashSet<&str> =
                factor_lower.split_whitespace().collect();
            let shared = hyp_words.intersection(&factor_words).count();
            let total = hyp_words.union(&factor_words).count().max(1);
            let relevance = shared as f64 / total as f64;

            // Direct impact: factor is directly affected by the change.
            if relevance > 0.0 || shared > 0 {
                let impact = if let Some(val) = factor.numeric_value {
                    // If there's a numeric value, hypothesize a change.
                    format!(
                        "If {}, then '{}' could shift (current value: {})",
                        hypothetical, factor.description, val
                    )
                } else {
                    format!(
                        "If {}, then '{}' would be affected",
                        hypothetical, factor.description
                    )
                };

                let plausibility = (0.3 + 0.7 * relevance).clamp(0.1, 0.95);

                outcomes.push(CounterfactualOutcome {
                    description: impact,
                    plausibility,
                    affected_factor: factor.description.clone(),
                    is_direct: relevance > 0.15,
                });
            }
        }

        // If no factors were directly relevant, generate a generic outcome.
        if outcomes.is_empty() {
            outcomes.push(CounterfactualOutcome {
                description: format!(
                    "If {}, downstream effects would propagate through the system",
                    hypothetical
                ),
                plausibility: 0.4,
                affected_factor: "system-wide".to_string(),
                is_direct: false,
            });
        }

        // Sort by plausibility descending.
        outcomes.sort_by(|a, b| {
            b.plausibility
                .partial_cmp(&a.plausibility)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        outcomes
    }
}

struct CausalFactor {
    #[allow(dead_code)]
    index: usize,
    description: String,
    numeric_value: Option<f64>,
}

struct CounterfactualOutcome {
    description: String,
    plausibility: f64,
    affected_factor: String,
    is_direct: bool,
}

/// Extract the first number found in a string.
fn extract_first_number(s: &str) -> Option<f64> {
    let mut num_str = String::new();
    let mut found_digit = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() || (ch == '.' && found_digit) {
            num_str.push(ch);
            found_digit = true;
        } else if found_digit {
            break;
        }
    }
    if found_digit {
        num_str.parse::<f64>().ok()
    } else {
        None
    }
}

impl ReasoningEngine for CounterfactualEngine {
    fn name(&self) -> &str {
        "counterfactual"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let hypothetical = Self::extract_hypothetical(&request.query);
        let factors = Self::parse_factors(&request.context);
        let outcomes = Self::propagate_change(&hypothetical, &factors);

        let items: Vec<ReasoningItem> = outcomes
            .iter()
            .map(|o| ReasoningItem {
                label: if o.is_direct {
                    "Direct Effect".to_string()
                } else {
                    "Indirect Effect".to_string()
                },
                description: o.description.clone(),
                confidence: o.plausibility,
                supporting_evidence: vec![
                    format!("Affected: {}", o.affected_factor),
                    format!("Hypothetical: {}", hypothetical),
                ],
            })
            .collect();

        let overall_confidence = if items.is_empty() {
            0.0
        } else {
            items.iter().map(|i| i.confidence).sum::<f64>() / items.len() as f64
        };

        Ok(ReasoningOutput {
            engine_name: "counterfactual".to_string(),
            result_type: ResultType::Outcomes,
            items,
            confidence: overall_confidence,
            provenance: vec![format!("hypothetical: {}", hypothetical)],
        })
    }
}

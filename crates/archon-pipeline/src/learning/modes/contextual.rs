//! Contextual reasoning engine — context-dependent reasoning.
//!
//! Adapts reasoning strategy based on situational signals in the context.
//! Evaluates relevance of each context line to the query, weights findings
//! by contextual fit, and produces ranked insights.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Contextual reasoning: adapts conclusions based on situational context.
pub struct ContextualEngine;

impl Default for ContextualEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextualEngine {
    pub fn new() -> Self {
        Self
    }

    /// Score how relevant a context line is to the query.
    fn relevance_score(context_line: &str, query: &str) -> f64 {
        let cl = context_line.to_lowercase();
        let ql = query.to_lowercase();

        let query_terms: Vec<&str> = ql.split_whitespace().collect();
        if query_terms.is_empty() {
            return 0.0;
        }

        let match_count = query_terms.iter().filter(|term| cl.contains(*term)).count();

        match_count as f64 / query_terms.len() as f64
    }

    /// Extract contextual signals from a line.
    /// Recognises "signal:", "context:", "role:", "domain:", "scenario:" prefixes.
    fn extract_signals(context: &[String]) -> Vec<ContextualSignal> {
        let mut signals = Vec::new();
        for line in context {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed
                .strip_prefix("signal:")
                .or_else(|| trimmed.strip_prefix("context:"))
                .or_else(|| trimmed.strip_prefix("role:"))
                .or_else(|| trimmed.strip_prefix("domain:"))
                .or_else(|| trimmed.strip_prefix("scenario:"))
            {
                signals.push(ContextualSignal {
                    text: rest.trim().to_string(),
                    weight: 1.0,
                });
            } else {
                signals.push(ContextualSignal {
                    text: trimmed.to_string(),
                    weight: 0.5,
                });
            }
        }
        signals
    }

    /// Generate contextual insights by cross-referencing signals.
    fn generate_insights(signals: &[ContextualSignal], query: &str) -> Vec<ContextualInsight> {
        let mut insights = Vec::new();

        // Insight 1: Most relevant signal to the query.
        let mut scored: Vec<(&ContextualSignal, f64)> = signals
            .iter()
            .map(|s| {
                let relevance = Self::relevance_score(&s.text, query);
                (s, relevance * s.weight)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (signal, score) in scored.iter().take(5) {
            if *score > 0.0 {
                insights.push(ContextualInsight {
                    description: format!("Relevant context (score={:.2}): {}", score, signal.text),
                    confidence: *score,
                });
            }
        }

        // Insight 2: Contradicting signals (pairs with opposite implications).
        for i in 0..signals.len() {
            for j in (i + 1)..signals.len() {
                let si = signals[i].text.to_lowercase();
                let sj = signals[j].text.to_lowercase();
                let contradiction = (si.contains("not") && sj.contains(&si.replace("not ", "")))
                    || (sj.contains("not") && si.contains(&sj.replace("not ", "")));
                if contradiction {
                    insights.push(ContextualInsight {
                        description: format!(
                            "Potential contradiction: '{}' vs '{}'",
                            signals[i].text, signals[j].text
                        ),
                        confidence: 0.7,
                    });
                }
            }
        }

        // Insight 3: Contextual summary.
        if signals.len() >= 3 {
            insights.push(ContextualInsight {
                description: format!(
                    "Context contains {} signals; primary domain indicators suggest adaptation needed",
                    signals.len()
                ),
                confidence: 0.6,
            });
        }

        insights.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        insights
    }
}

struct ContextualSignal {
    text: String,
    weight: f64,
}

struct ContextualInsight {
    description: String,
    confidence: f64,
}

impl ReasoningEngine for ContextualEngine {
    fn name(&self) -> &str {
        "contextual"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let signals = Self::extract_signals(&request.context);

        if signals.is_empty() {
            return Ok(ReasoningOutput {
                engine_name: "contextual".to_string(),
                result_type: ResultType::ContextualInsights,
                items: vec![ReasoningItem {
                    label: "No Signals".to_string(),
                    description: "No contextual signals found in context".to_string(),
                    confidence: 0.0,
                    supporting_evidence: vec![],
                }],
                confidence: 0.0,
                provenance: vec![format!("query: {}", request.query)],
            });
        }

        let insights = Self::generate_insights(&signals, &request.query);

        let items: Vec<ReasoningItem> = if insights.is_empty() {
            vec![ReasoningItem {
                label: "No Insights".to_string(),
                description: "Context analysed but no actionable insights generated".to_string(),
                confidence: 0.3,
                supporting_evidence: signals.iter().map(|s| s.text.clone()).collect(),
            }]
        } else {
            insights
                .iter()
                .enumerate()
                .map(|(i, ins)| ReasoningItem {
                    label: format!("CI{}", i + 1),
                    description: ins.description.clone(),
                    confidence: ins.confidence,
                    supporting_evidence: vec![format!("query: {}", request.query)],
                })
                .collect()
        };

        let overall = items
            .iter()
            .map(|i| i.confidence)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        Ok(ReasoningOutput {
            engine_name: "contextual".to_string(),
            result_type: ResultType::ContextualInsights,
            items,
            confidence: overall,
            provenance: vec![
                format!("query: {}", request.query),
                format!("signal_count: {}", signals.len()),
            ],
        })
    }
}

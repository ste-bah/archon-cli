//! First principles reasoning engine — fundamental truth reasoning.
//!
//! Strips assumptions from the query, identifies axioms/first principles
//! from context, builds a reasoning chain from fundamentals, and returns
//! axiom-based conclusions.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// First principles reasoning: building from fundamental truths.
pub struct FirstPrinciplesEngine;

impl FirstPrinciplesEngine {
    pub fn new() -> Self {
        Self
    }

    /// Classify context lines into axioms and assumptions.
    fn classify_context(context: &[String]) -> (Vec<Axiom>, Vec<Assumption>) {
        let mut axioms = Vec::new();
        let mut assumptions = Vec::new();

        for line in context {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let lower = trimmed.to_lowercase();

            if lower.starts_with("axiom:") || lower.starts_with("principle:") || lower.starts_with("fact:") {
                let content = trimmed
                    .splitn(2, ':')
                    .nth(1)
                    .unwrap_or(trimmed)
                    .trim()
                    .to_string();
                axioms.push(Axiom {
                    statement: content,
                    certainty: 1.0,
                });
            } else if lower.starts_with("assumption:") || lower.starts_with("belief:") {
                let content = trimmed
                    .splitn(2, ':')
                    .nth(1)
                    .unwrap_or(trimmed)
                    .trim()
                    .to_string();
                assumptions.push(Assumption {
                    statement: content,
                    challenged: true,
                });
            } else {
                // Heuristic: shorter statements are more likely axioms.
                let word_count = trimmed.split_whitespace().count();
                if word_count <= 8 {
                    axioms.push(Axiom {
                        statement: trimmed.to_string(),
                        certainty: 0.7,
                    });
                } else {
                    assumptions.push(Assumption {
                        statement: trimmed.to_string(),
                        challenged: false,
                    });
                }
            }
        }

        (axioms, assumptions)
    }

    /// Build a reasoning chain from axioms toward the query conclusion.
    fn build_reasoning_chain(
        query: &str,
        axioms: &[Axiom],
        assumptions: &[Assumption],
    ) -> Vec<ReasoningStep> {
        let mut chain = Vec::new();

        // Step 1: State the axioms as foundation.
        for (i, axiom) in axioms.iter().enumerate() {
            chain.push(ReasoningStep {
                step_type: StepType::Foundation,
                statement: format!("Axiom {}: {}", i + 1, axiom.statement),
                derives_from: vec![],
                confidence: axiom.certainty,
            });
        }

        // Step 2: Challenge each assumption.
        for assumption in assumptions {
            let challenge = if assumption.challenged {
                format!(
                    "Challenged assumption: '{}' — this is not a first principle and may not hold",
                    assumption.statement
                )
            } else {
                format!(
                    "Unchallenged claim: '{}' — needs verification against axioms",
                    assumption.statement
                )
            };
            chain.push(ReasoningStep {
                step_type: StepType::Challenge,
                statement: challenge,
                derives_from: vec![],
                confidence: 0.3,
            });
        }

        // Step 3: Derive intermediate conclusions from axiom pairs.
        if axioms.len() >= 2 {
            for i in 0..axioms.len() - 1 {
                let derived = format!(
                    "From '{}' and '{}', we can derive that both conditions hold simultaneously",
                    axioms[i].statement,
                    axioms[i + 1].statement
                );
                let avg_certainty = (axioms[i].certainty + axioms[i + 1].certainty) / 2.0;
                chain.push(ReasoningStep {
                    step_type: StepType::Derivation,
                    statement: derived,
                    derives_from: vec![i, i + 1],
                    confidence: avg_certainty * 0.9,
                });
            }
        }

        // Step 4: Final conclusion linking axioms to query.
        if !axioms.is_empty() {
            let axiom_summary: Vec<String> =
                axioms.iter().map(|a| a.statement.clone()).collect();
            let conclusion = format!(
                "From first principles [{}], we conclude regarding '{}': \
                 the answer must be grounded in these fundamentals, \
                 rejecting assumptions that contradict them",
                axiom_summary.join("; "),
                query
            );
            let min_certainty = axioms
                .iter()
                .map(|a| a.certainty)
                .fold(f64::MAX, f64::min);
            chain.push(ReasoningStep {
                step_type: StepType::Conclusion,
                statement: conclusion,
                derives_from: (0..axioms.len()).collect(),
                confidence: min_certainty * 0.85,
            });
        }

        chain
    }
}

struct Axiom {
    statement: String,
    certainty: f64,
}

struct Assumption {
    statement: String,
    challenged: bool,
}

struct ReasoningStep {
    step_type: StepType,
    statement: String,
    derives_from: Vec<usize>,
    confidence: f64,
}

enum StepType {
    Foundation,
    Challenge,
    Derivation,
    Conclusion,
}

impl ReasoningEngine for FirstPrinciplesEngine {
    fn name(&self) -> &str {
        "first_principles"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let (axioms, assumptions) = Self::classify_context(&request.context);
        let chain = Self::build_reasoning_chain(&request.query, &axioms, &assumptions);

        let items: Vec<ReasoningItem> = chain
            .iter()
            .map(|step| {
                let label = match step.step_type {
                    StepType::Foundation => "Foundation".to_string(),
                    StepType::Challenge => "Challenge".to_string(),
                    StepType::Derivation => "Derivation".to_string(),
                    StepType::Conclusion => "Conclusion".to_string(),
                };
                let evidence = if step.derives_from.is_empty() {
                    vec!["Self-evident or given".to_string()]
                } else {
                    step.derives_from
                        .iter()
                        .map(|&i| format!("Derives from step {}", i + 1))
                        .collect()
                };
                ReasoningItem {
                    label,
                    description: step.statement.clone(),
                    confidence: step.confidence,
                    supporting_evidence: evidence,
                }
            })
            .collect();

        let overall_confidence = if items.is_empty() {
            0.0
        } else {
            // Use the conclusion confidence if available, else average.
            items
                .last()
                .map(|i| i.confidence)
                .unwrap_or(0.0)
        };

        Ok(ReasoningOutput {
            engine_name: "first_principles".to_string(),
            result_type: ResultType::Axioms,
            items,
            confidence: overall_confidence,
            provenance: vec![
                format!("query: {}", request.query),
                format!("axiom_count: {}", axioms.len()),
                format!("assumption_count: {}", assumptions.len()),
            ],
        })
    }
}

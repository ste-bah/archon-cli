//! Deductive reasoning engine — top-down logical inference.
//!
//! Given premises from context, derives conclusions by applying logical rules.
//! Implements modus ponens, modus tollens, and syllogism patterns.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Deductive reasoning: top-down inference from premises to conclusions.
pub struct DeductiveEngine;

impl Default for DeductiveEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DeductiveEngine {
    pub fn new() -> Self {
        Self
    }

    /// Extract premises from context lines.
    /// Recognises "premise:", "if:", "given:", "all:", "no:" prefixes.
    fn extract_premises(context: &[String]) -> Vec<Premise> {
        let mut premises = Vec::new();
        for line in context {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed
                .strip_prefix("premise:")
                .or_else(|| trimmed.strip_prefix("if:"))
                .or_else(|| trimmed.strip_prefix("given:"))
                .or_else(|| trimmed.strip_prefix("all:"))
                .or_else(|| trimmed.strip_prefix("no:"))
            {
                premises.push(Premise {
                    text: rest.trim().to_string(),
                    universal: trimmed.starts_with("all:") || trimmed.starts_with("no:"),
                    negative: trimmed.starts_with("no:"),
                });
            } else {
                premises.push(Premise {
                    text: trimmed.to_string(),
                    universal: false,
                    negative: false,
                });
            }
        }
        premises
    }

    /// Apply modus ponens: if P→Q and P, then Q.
    fn apply_modus_ponens(premises: &[Premise]) -> Vec<Conclusion> {
        let mut conclusions = Vec::new();
        for (i, p) in premises.iter().enumerate() {
            let lower = p.text.to_lowercase();
            // Look for implication patterns: "if X then Y", "X implies Y", "X -> Y"
            if let Some(antecedent) = lower
                .strip_prefix("if ")
                .and_then(|rest| rest.split(" then ").next())
                .or_else(|| {
                    lower
                        .split(" implies ")
                        .next()
                        .or_else(|| lower.split(" -> ").next())
                })
            {
                let consequent = lower
                    .split(" then ")
                    .nth(1)
                    .or_else(|| lower.split(" implies ").nth(1))
                    .or_else(|| lower.split(" -> ").nth(1))
                    .unwrap_or("unknown");

                // Check if the antecedent is asserted in another premise.
                for other in premises {
                    if other.text.to_lowercase().contains(antecedent) && other.text != p.text {
                        conclusions.push(Conclusion {
                            description: format!(
                                "Modus ponens: Given '{}' and '{}', conclude '{}'",
                                p.text, other.text, consequent
                            ),
                            confidence: 0.85,
                            premises_used: vec![p.text.clone(), other.text.clone()],
                        });
                    }
                }
                let _ = (i, consequent);
            }

            // Modus tollens: if P→Q and not Q, then not P.
            if let Some(antecedent) = lower.strip_prefix("if ").and_then(|rest| {
                rest.split(" then ")
                    .next()
                    .or_else(|| rest.split(" -> ").next())
            }) {
                let consequent = lower
                    .split(" then ")
                    .nth(1)
                    .or_else(|| lower.split(" -> ").nth(1))
                    .unwrap_or("unknown");

                for other in premises {
                    if other.negative && other.text.to_lowercase().contains(consequent) {
                        conclusions.push(Conclusion {
                            description: format!(
                                "Modus tollens: Given '{}' and 'not {}', conclude 'not {}'",
                                p.text, consequent, antecedent
                            ),
                            confidence: 0.80,
                            premises_used: vec![p.text.clone(), other.text.clone()],
                        });
                    }
                }
                let _ = (i, antecedent, consequent);
            }
        }
        conclusions
    }

    /// Apply syllogism: all A are B, all B are C → all A are C.
    fn apply_syllogism(premises: &[Premise]) -> Vec<Conclusion> {
        let mut conclusions = Vec::new();
        let universal: Vec<&Premise> = premises.iter().filter(|p| p.universal).collect();

        for (i, a) in universal.iter().enumerate() {
            for b in universal.iter().skip(i + 1) {
                let a_text = a.text.to_lowercase();
                let b_text = b.text.to_lowercase();
                if a_text.contains("are") && b_text.contains("are") {
                    let a_parts: Vec<&str> = a_text.split(" are ").collect();
                    let b_parts: Vec<&str> = b_text.split(" are ").collect();
                    if a_parts.len() == 2
                        && b_parts.len() == 2
                        && a_parts[1].trim() == b_parts[0].trim()
                    {
                        conclusions.push(Conclusion {
                            description: format!(
                                "Syllogism: '{}' and '{}' → '{} are {}'",
                                a.text, b.text, a_parts[0], b_parts[1]
                            ),
                            confidence: 0.90,
                            premises_used: vec![a.text.clone(), b.text.clone()],
                        });
                    }
                }
            }
        }
        conclusions
    }
}

struct Premise {
    text: String,
    universal: bool,
    negative: bool,
}

struct Conclusion {
    description: String,
    confidence: f64,
    premises_used: Vec<String>,
}

impl ReasoningEngine for DeductiveEngine {
    fn name(&self) -> &str {
        "deductive"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let premises = Self::extract_premises(&request.context);

        if premises.is_empty() {
            return Ok(ReasoningOutput {
                engine_name: "deductive".to_string(),
                result_type: ResultType::Deductions,
                items: vec![ReasoningItem {
                    label: "No Premises".to_string(),
                    description: "No premises found in context for deductive reasoning".to_string(),
                    confidence: 0.0,
                    supporting_evidence: vec![],
                }],
                confidence: 0.0,
                provenance: vec![format!("query: {}", request.query)],
            });
        }

        let mut conclusions = Self::apply_modus_ponens(&premises);
        conclusions.extend(Self::apply_syllogism(&premises));

        if conclusions.is_empty() {
            // Fallback: restate premises as low-confidence conclusions.
            conclusions = premises
                .iter()
                .map(|p| Conclusion {
                    description: format!("Premise (restated): {}", p.text),
                    confidence: 0.5,
                    premises_used: vec![p.text.clone()],
                })
                .collect();
        }

        let items: Vec<ReasoningItem> = conclusions
            .iter()
            .enumerate()
            .map(|(i, c)| ReasoningItem {
                label: format!("D{}", i + 1),
                description: c.description.clone(),
                confidence: c.confidence,
                supporting_evidence: c.premises_used.clone(),
            })
            .collect();

        let overall = items
            .iter()
            .map(|i| i.confidence)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        Ok(ReasoningOutput {
            engine_name: "deductive".to_string(),
            result_type: ResultType::Deductions,
            items,
            confidence: overall,
            provenance: vec![
                format!("query: {}", request.query),
                format!("premise_count: {}", premises.len()),
            ],
        })
    }
}

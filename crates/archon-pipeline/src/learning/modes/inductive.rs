//! Inductive reasoning engine — bottom-up generalization from examples.
//!
//! Extracts patterns from context examples and generates general rules
//! with confidence based on example count and consistency.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Inductive reasoning: bottom-up generalisation from specific examples.
pub struct InductiveEngine;

impl InductiveEngine {
    pub fn new() -> Self {
        Self
    }

    /// Parse examples from context.
    /// Format: "example:label|feature1,feature2,..."
    fn parse_examples(context: &[String]) -> Vec<Example> {
        let mut examples = Vec::new();
        for line in context {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("example:") {
                let parts: Vec<&str> = rest.splitn(2, '|').collect();
                let label = parts.first().map(|s| s.trim()).unwrap_or("unknown");
                let features: Vec<String> = if parts.len() > 1 {
                    parts[1].split(',').map(|s| s.trim().to_string()).collect()
                } else {
                    vec![]
                };
                examples.push(Example {
                    label: label.to_string(),
                    features,
                });
            } else {
                // Free-form: treat each line as an example.
                let words: Vec<String> =
                    trimmed.split_whitespace().map(|s| s.to_string()).collect();
                examples.push(Example {
                    label: format!("ex{}", examples.len() + 1),
                    features: words,
                });
            }
        }
        examples
    }

    /// Find common features across examples sharing the same label.
    fn extract_common_features(examples: &[Example]) -> Vec<Generalisation> {
        let mut by_label: std::collections::HashMap<&str, Vec<&Example>> =
            std::collections::HashMap::new();
        for ex in examples {
            by_label.entry(ex.label.as_str()).or_default().push(ex);
        }

        let mut generalisations = Vec::new();

        for (label, group) in &by_label {
            if group.len() < 2 {
                continue;
            }

            // Find features present in ALL examples of this label.
            if let Some(first) = group.first() {
                let common: Vec<String> = first
                    .features
                    .iter()
                    .filter(|feat| {
                        group
                            .iter()
                            .all(|ex| ex.features.iter().any(|f| f.contains(feat.as_str())))
                    })
                    .cloned()
                    .collect();

                if !common.is_empty() {
                    let confidence = 0.5 + 0.1 * (group.len() as f64).min(5.0);
                    generalisations.push(Generalisation {
                        rule: format!(
                            "Examples labelled '{}' all share features: [{}]",
                            label,
                            common.join(", ")
                        ),
                        confidence,
                        example_count: group.len(),
                    });
                }
            }
        }

        // Cross-label patterns: features unique to one label.
        let all_features: Vec<(&str, &String)> = examples
            .iter()
            .flat_map(|ex| ex.features.iter().map(move |f| (ex.label.as_str(), f)))
            .collect();

        let mut feature_labels: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        for (label, feat) in &all_features {
            feature_labels.entry(feat.as_str()).or_default().push(label);
        }

        for (feat, labels) in &feature_labels {
            if labels.len() == 1 && examples.iter().filter(|ex| ex.label == labels[0]).count() >= 2
            {
                generalisations.push(Generalisation {
                    rule: format!(
                        "Feature '{}' appears exclusively in '{}' examples ({} instances)",
                        feat,
                        labels[0],
                        labels.len()
                    ),
                    confidence: 0.6,
                    example_count: labels.len(),
                });
            }
        }

        generalisations
    }
}

struct Example {
    label: String,
    features: Vec<String>,
}

struct Generalisation {
    rule: String,
    confidence: f64,
    example_count: usize,
}

impl ReasoningEngine for InductiveEngine {
    fn name(&self) -> &str {
        "inductive"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let examples = Self::parse_examples(&request.context);

        if examples.is_empty() {
            return Ok(ReasoningOutput {
                engine_name: "inductive".to_string(),
                result_type: ResultType::Generalizations,
                items: vec![ReasoningItem {
                    label: "No Examples".to_string(),
                    description: "No examples found in context for inductive generalisation"
                        .to_string(),
                    confidence: 0.0,
                    supporting_evidence: vec![],
                }],
                confidence: 0.0,
                provenance: vec![format!("query: {}", request.query)],
            });
        }

        let generalisations = Self::extract_common_features(&examples);

        let items: Vec<ReasoningItem> = if generalisations.is_empty() {
            vec![ReasoningItem {
                label: "Insufficient Data".to_string(),
                description: format!(
                    "{} examples found but no generalisable patterns detected",
                    examples.len()
                ),
                confidence: 0.3,
                supporting_evidence: examples.iter().map(|e| e.label.clone()).collect(),
            }]
        } else {
            generalisations
                .iter()
                .enumerate()
                .map(|(i, g)| ReasoningItem {
                    label: format!("G{}", i + 1),
                    description: format!("{} (from {} examples)", g.rule, g.example_count),
                    confidence: g.confidence,
                    supporting_evidence: vec![format!("{} examples", g.example_count)],
                })
                .collect()
        };

        let overall = items
            .iter()
            .map(|i| i.confidence)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        Ok(ReasoningOutput {
            engine_name: "inductive".to_string(),
            result_type: ResultType::Generalizations,
            items,
            confidence: overall,
            provenance: vec![
                format!("query: {}", request.query),
                format!("example_count: {}", examples.len()),
            ],
        })
    }
}

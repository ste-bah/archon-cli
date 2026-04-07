//! Analogical reasoning engine — reasoning by analogy.
//!
//! Takes source and target domains, identifies structural correspondences,
//! transfers inferences from source to target, and scores by structural
//! similarity and relational depth.

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Analogical reasoning: structural mapping between domains.
pub struct AnalogicalEngine;

impl AnalogicalEngine {
    pub fn new() -> Self {
        Self
    }

    /// Extract domain concepts from context lines.
    fn extract_concepts(context: &[String]) -> Vec<DomainConcept> {
        context
            .iter()
            .filter(|c| !c.trim().is_empty())
            .map(|c| {
                let parts: Vec<&str> = c.splitn(2, ' ').collect();
                let entity = parts.first().unwrap_or(&"").to_string();
                let relation = parts.get(1).unwrap_or(&"").to_string();
                DomainConcept { entity, relation }
            })
            .collect()
    }

    /// Build structural mappings between source and target domain concepts.
    fn build_mappings(
        source: &str,
        target: &str,
        concepts: &[DomainConcept],
    ) -> Vec<AnalogicalMapping> {
        let mut mappings = Vec::new();

        // Group concepts by whether they mention source or target domain.
        let source_lower = source.to_lowercase();
        let target_lower = target.to_lowercase();

        let source_concepts: Vec<&DomainConcept> = concepts
            .iter()
            .filter(|c| c.entity.to_lowercase().contains(&source_lower)
                || c.relation.to_lowercase().contains(&source_lower))
            .collect();

        let target_concepts: Vec<&DomainConcept> = concepts
            .iter()
            .filter(|c| c.entity.to_lowercase().contains(&target_lower)
                || c.relation.to_lowercase().contains(&target_lower))
            .collect();

        // Create positional mappings between source and target concepts.
        let pair_count = source_concepts.len().min(target_concepts.len());
        for i in 0..pair_count {
            let sc = &source_concepts[i];
            let tc = &target_concepts[i];

            // Structural similarity: how similar are the relation structures.
            let shared_words = count_shared_words(&sc.relation, &tc.relation);
            let total_words = count_total_words(&sc.relation, &tc.relation).max(1);
            let structural_sim = shared_words as f64 / total_words as f64;

            // Relational depth: longer relations = deeper mapping.
            let depth = (sc.relation.split_whitespace().count()
                + tc.relation.split_whitespace().count()) as f64
                / 20.0;
            let depth_score = depth.min(1.0);

            mappings.push(AnalogicalMapping {
                source_element: format!("{} {}", sc.entity, sc.relation),
                target_element: format!("{} {}", tc.entity, tc.relation),
                structural_similarity: structural_sim.clamp(0.1, 1.0),
                relational_depth: depth_score.clamp(0.1, 1.0),
            });
        }

        // If no paired mappings, create a high-level domain mapping.
        if mappings.is_empty() {
            mappings.push(AnalogicalMapping {
                source_element: source.to_string(),
                target_element: target.to_string(),
                structural_similarity: 0.5,
                relational_depth: 0.3,
            });
        }

        mappings
    }

    /// Transfer inferences from source mappings to target domain.
    fn transfer_inferences(
        mappings: &[AnalogicalMapping],
        source: &str,
        target: &str,
    ) -> Vec<ReasoningItem> {
        mappings
            .iter()
            .map(|m| {
                let score = 0.6 * m.structural_similarity + 0.4 * m.relational_depth;
                ReasoningItem {
                    label: format!("{} → {}", source, target),
                    description: format!(
                        "Just as '{}' in {}, so too '{}' in {}",
                        m.source_element, source, m.target_element, target
                    ),
                    confidence: score.clamp(0.0, 1.0),
                    supporting_evidence: vec![
                        format!("Source: {}", m.source_element),
                        format!("Target: {}", m.target_element),
                        format!("Structural similarity: {:.2}", m.structural_similarity),
                    ],
                }
            })
            .collect()
    }
}

struct DomainConcept {
    entity: String,
    relation: String,
}

struct AnalogicalMapping {
    source_element: String,
    target_element: String,
    structural_similarity: f64,
    relational_depth: f64,
}

fn count_shared_words(a: &str, b: &str) -> usize {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    let a_words: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
    let b_words: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();
    a_words.intersection(&b_words).count()
}

fn count_total_words(a: &str, b: &str) -> usize {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    let a_words: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
    let b_words: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();
    a_words.union(&b_words).count()
}

impl ReasoningEngine for AnalogicalEngine {
    fn name(&self) -> &str {
        "analogical"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let source = request
            .parameters
            .get("source_domain")
            .cloned()
            .unwrap_or_else(|| "source".to_string());
        let target = request
            .parameters
            .get("target_domain")
            .cloned()
            .unwrap_or_else(|| "target".to_string());

        let concepts = Self::extract_concepts(&request.context);
        let mappings = Self::build_mappings(&source, &target, &concepts);
        let items = Self::transfer_inferences(&mappings, &source, &target);

        let overall_confidence = if items.is_empty() {
            0.0
        } else {
            items.iter().map(|i| i.confidence).sum::<f64>() / items.len() as f64
        };

        Ok(ReasoningOutput {
            engine_name: "analogical".to_string(),
            result_type: ResultType::Mappings,
            items,
            confidence: overall_confidence,
            provenance: vec![format!("source: {}, target: {}", source, target)],
        })
    }
}

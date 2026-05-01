//! Causal reasoning engine — cause-effect chain reasoning.
//!
//! Distinct from CausalMemory (the storage layer). This mode reasons over
//! causal links extracted from context to build and traverse cause-effect chains.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Causal reasoning: cause-effect chain analysis.
pub struct CausalEngine;

impl Default for CausalEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CausalEngine {
    pub fn new() -> Self {
        Self
    }

    /// Parse causal links from context.
    /// Format: "cause:X -> effect:Y" or "X causes Y" or "X leads to Y".
    fn parse_causal_links(context: &[String]) -> Vec<CausalLink> {
        let mut links = Vec::new();
        for line in context {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("cause:")
                && let Some((cause, effect)) = rest.split_once("->")
            {
                let effect = effect.trim();
                let effect = effect.strip_prefix("effect:").unwrap_or(effect);
                links.push(CausalLink {
                    cause: cause.trim().to_string(),
                    effect: effect.trim().to_string(),
                });
                continue;
            }
            if let Some((cause, effect)) = trimmed.split_once(" causes ") {
                links.push(CausalLink {
                    cause: cause.trim().to_string(),
                    effect: effect.trim().to_string(),
                });
                continue;
            }
            if let Some((cause, effect)) = trimmed.split_once(" leads to ") {
                links.push(CausalLink {
                    cause: cause.trim().to_string(),
                    effect: effect.trim().to_string(),
                });
                continue;
            }
        }
        links
    }

    /// Build adjacency list: node -> [(neighbor, link_index)].
    fn build_graph(links: &[CausalLink]) -> HashMap<&str, Vec<(&str, usize)>> {
        let mut graph: HashMap<&str, Vec<(&str, usize)>> = HashMap::new();
        for (i, link) in links.iter().enumerate() {
            graph
                .entry(link.cause.as_str())
                .or_default()
                .push((link.effect.as_str(), i));
        }
        graph
    }

    /// Find all causal chains via DFS up to max_depth.
    fn find_chains(
        graph: &HashMap<&str, Vec<(&str, usize)>>,
        links: &[CausalLink],
        start: &str,
        max_depth: usize,
    ) -> Vec<CausalChain> {
        let mut chains = Vec::new();
        let mut stack: Vec<(Vec<usize>, &str)> = vec![(vec![], start)];

        while let Some((path_indices, current)) = stack.pop() {
            if path_indices.len() >= max_depth {
                continue;
            }
            if let Some(neighbors) = graph.get(current) {
                for (next, link_idx) in neighbors {
                    if path_indices.contains(link_idx) {
                        continue; // cycle detection
                    }
                    let mut new_path = path_indices.clone();
                    new_path.push(*link_idx);

                    let chain_nodes: Vec<String> = {
                        let mut nodes = vec![links[new_path[0]].cause.clone()];
                        for idx in &new_path {
                            nodes.push(links[*idx].effect.clone());
                        }
                        nodes
                    };

                    chains.push(CausalChain {
                        description: chain_nodes.join(" → "),
                        length: new_path.len(),
                        confidence: 0.9_f64.powi(new_path.len() as i32),
                    });

                    stack.push((new_path, next));
                }
            }
        }

        chains.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        chains
    }

    /// Identify root causes (nodes with no incoming edges) and leaf effects (no outgoing).
    fn identify_roots_and_leaves(links: &[CausalLink]) -> (Vec<String>, Vec<String>) {
        let mut has_incoming: HashSet<&str> = HashSet::new();
        let mut has_outgoing: HashSet<&str> = HashSet::new();
        let mut all_nodes: HashSet<&str> = HashSet::new();

        for link in links {
            has_outgoing.insert(link.cause.as_str());
            has_incoming.insert(link.effect.as_str());
            all_nodes.insert(link.cause.as_str());
            all_nodes.insert(link.effect.as_str());
        }

        let roots: Vec<String> = all_nodes
            .iter()
            .filter(|n| !has_incoming.contains(*n))
            .map(|n| n.to_string())
            .collect();

        let leaves: Vec<String> = all_nodes
            .iter()
            .filter(|n| !has_outgoing.contains(*n))
            .map(|n| n.to_string())
            .collect();

        (roots, leaves)
    }
}

struct CausalLink {
    cause: String,
    effect: String,
}

struct CausalChain {
    description: String,
    length: usize,
    confidence: f64,
}

impl ReasoningEngine for CausalEngine {
    fn name(&self) -> &str {
        "causal"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let links = Self::parse_causal_links(&request.context);

        if links.is_empty() {
            return Ok(ReasoningOutput {
                engine_name: "causal".to_string(),
                result_type: ResultType::CausalChains,
                items: vec![ReasoningItem {
                    label: "No Links".to_string(),
                    description: "No causal links found in context".to_string(),
                    confidence: 0.0,
                    supporting_evidence: vec![],
                }],
                confidence: 0.0,
                provenance: vec![format!("query: {}", request.query)],
            });
        }

        let graph = Self::build_graph(&links);
        let (roots, leaves) = Self::identify_roots_and_leaves(&links);

        let mut items = Vec::new();

        // Report root causes.
        if !roots.is_empty() {
            items.push(ReasoningItem {
                label: "Root Causes".to_string(),
                description: format!("Root cause(s): {}", roots.join(", ")),
                confidence: 0.95,
                supporting_evidence: roots.clone(),
            });
        }

        // Report leaf effects.
        if !leaves.is_empty() {
            items.push(ReasoningItem {
                label: "Leaf Effects".to_string(),
                description: format!("Final effect(s): {}", leaves.join(", ")),
                confidence: 0.95,
                supporting_evidence: leaves.clone(),
            });
        }

        // Find chains from each root.
        for root in &roots {
            let chains = Self::find_chains(&graph, &links, root, 5);
            for chain in chains.iter().take(3) {
                items.push(ReasoningItem {
                    label: format!("Chain from {}", root),
                    description: format!("{} (len={})", chain.description, chain.length),
                    confidence: chain.confidence,
                    supporting_evidence: links
                        .iter()
                        .map(|l| format!("{} → {}", l.cause, l.effect))
                        .collect(),
                });
            }
        }

        let overall = items.first().map(|i| i.confidence).unwrap_or(0.0);

        Ok(ReasoningOutput {
            engine_name: "causal".to_string(),
            result_type: ResultType::CausalChains,
            items,
            confidence: overall,
            provenance: vec![
                format!("query: {}", request.query),
                format!("link_count: {}", links.len()),
                format!("root_count: {}", roots.len()),
                format!("leaf_count: {}", leaves.len()),
            ],
        })
    }
}

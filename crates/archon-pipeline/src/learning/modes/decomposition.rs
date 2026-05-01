//! Decomposition reasoning engine — problem decomposition.
//!
//! Breaks a complex problem into subproblems, identifies dependencies
//! between them, performs topological sorting, and returns an ordered
//! subproblem list.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Decomposition reasoning: divide and conquer.
pub struct DecompositionEngine;

impl Default for DecompositionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DecompositionEngine {
    pub fn new() -> Self {
        Self
    }

    /// Parse subproblems and dependencies from context.
    /// Lines without "depends on" are subproblems.
    /// Lines like "X depends on Y" define edges.
    fn parse_subproblems(context: &[String]) -> (Vec<Subproblem>, Vec<(String, String)>) {
        let mut subproblems: Vec<Subproblem> = Vec::new();
        let mut dependencies: Vec<(String, String)> = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();

        for line in context {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let lower = trimmed.to_lowercase();
            if lower.contains("depends on") || lower.contains("depends on") {
                // Parse dependency: "X depends on Y"
                let parts: Vec<&str> = if lower.contains("depends on") {
                    lower.splitn(2, "depends on").collect()
                } else {
                    continue;
                };

                if parts.len() == 2 {
                    let dependent = parts[0].trim().to_string();
                    let dependency = parts[1].trim().to_string();
                    dependencies.push((dependent, dependency));
                }
            } else {
                // It's a subproblem description.
                // Strip common prefixes like "need ".
                let name = trimmed
                    .strip_prefix("need ")
                    .or_else(|| trimmed.strip_prefix("requires "))
                    .unwrap_or(trimmed)
                    .to_string();

                if seen_names.insert(name.to_lowercase()) {
                    subproblems.push(Subproblem {
                        name: name.clone(),
                        description: trimmed.to_string(),
                        complexity: estimate_complexity(trimmed),
                    });
                }
            }
        }

        // Ensure dependency targets exist as subproblems.
        for (dep, req) in &dependencies {
            if seen_names.insert(dep.to_lowercase()) {
                subproblems.push(Subproblem {
                    name: dep.clone(),
                    description: dep.clone(),
                    complexity: 0.5,
                });
            }
            if seen_names.insert(req.to_lowercase()) {
                subproblems.push(Subproblem {
                    name: req.clone(),
                    description: req.clone(),
                    complexity: 0.5,
                });
            }
        }

        (subproblems, dependencies)
    }

    #[allow(clippy::needless_range_loop)]
    /// Topological sort using Kahn's algorithm.
    fn topological_sort(
        subproblems: &[Subproblem],
        dependencies: &[(String, String)],
    ) -> Vec<usize> {
        let name_to_idx: HashMap<String, usize> = subproblems
            .iter()
            .enumerate()
            .map(|(i, sp)| (sp.name.to_lowercase(), i))
            .collect();

        let n = subproblems.len();
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

        for (dependent, dependency) in dependencies {
            let dep_lower = dependent.to_lowercase();
            let req_lower = dependency.to_lowercase();
            if let (Some(&from_idx), Some(&to_idx)) =
                (name_to_idx.get(&req_lower), name_to_idx.get(&dep_lower))
            {
                adj[from_idx].push(to_idx);
                in_degree[to_idx] += 1;
            }
        }

        // Kahn's algorithm.
        let mut queue: VecDeque<usize> = VecDeque::new();
        for i in 0..n {
            if in_degree[i] == 0 {
                queue.push_back(i);
            }
        }

        let mut order = Vec::new();
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for &neighbor in &adj[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }

        // If there's a cycle, append remaining nodes.
        if order.len() < n {
            for i in 0..n {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
        }

        order
    }
}

struct Subproblem {
    name: String,
    description: String,
    complexity: f64,
}

/// Estimate complexity based on description length and keywords.
fn estimate_complexity(description: &str) -> f64 {
    let word_count = description.split_whitespace().count();
    let complexity_keywords = [
        "complex",
        "difficult",
        "hard",
        "critical",
        "important",
        "major",
    ];
    let keyword_boost: f64 = complexity_keywords
        .iter()
        .filter(|kw| description.to_lowercase().contains(*kw))
        .count() as f64
        * 0.1;

    let base = (word_count as f64 / 20.0).clamp(0.1, 0.8);
    (base + keyword_boost).clamp(0.1, 1.0)
}

impl ReasoningEngine for DecompositionEngine {
    fn name(&self) -> &str {
        "decomposition"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let (subproblems, dependencies) = Self::parse_subproblems(&request.context);

        if subproblems.is_empty() {
            // Decompose the query itself into words/phrases as subproblems.
            let words: Vec<&str> = request.query.split_whitespace().collect();
            let items = vec![ReasoningItem {
                label: "S1".to_string(),
                description: format!("Analyze: {}", request.query),
                confidence: 0.5,
                supporting_evidence: words.iter().map(|w| w.to_string()).collect(),
            }];

            return Ok(ReasoningOutput {
                engine_name: "decomposition".to_string(),
                result_type: ResultType::Subproblems,
                items,
                confidence: 0.5,
                provenance: vec![format!("query: {}", request.query)],
            });
        }

        let order = Self::topological_sort(&subproblems, &dependencies);

        let items: Vec<ReasoningItem> = order
            .iter()
            .enumerate()
            .filter_map(|(rank, &idx)| {
                subproblems.get(idx).map(|sp| {
                    let deps_for_this: Vec<String> = dependencies
                        .iter()
                        .filter(|(d, _)| d.to_lowercase() == sp.name.to_lowercase())
                        .map(|(_, req)| format!("depends on: {}", req))
                        .collect();

                    ReasoningItem {
                        label: format!("S{}", rank + 1),
                        description: sp.description.clone(),
                        confidence: 1.0 - sp.complexity * 0.3,
                        supporting_evidence: deps_for_this,
                    }
                })
            })
            .collect();

        let overall_confidence = if items.is_empty() {
            0.0
        } else {
            items.iter().map(|i| i.confidence).sum::<f64>() / items.len() as f64
        };

        Ok(ReasoningOutput {
            engine_name: "decomposition".to_string(),
            result_type: ResultType::Subproblems,
            items,
            confidence: overall_confidence,
            provenance: vec![
                format!("query: {}", request.query),
                format!("dependency_count: {}", dependencies.len()),
            ],
        })
    }
}

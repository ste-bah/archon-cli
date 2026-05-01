//! Constraint satisfaction engine — backtracking search with arc consistency.
//!
//! Parses variables, domains, and constraints from context. Implements
//! backtracking search with AC-3 arc consistency pruning. Returns satisfying
//! assignments or proof of unsatisfiability.

use std::collections::HashMap;

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Constraint satisfaction reasoning engine.
pub struct ConstraintEngine;

impl Default for ConstraintEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstraintEngine {
    pub fn new() -> Self {
        Self
    }

    /// Parse a CSP from context lines.
    /// Format: "var:name:val1,val2,..." for variables,
    ///         "constraint:name1!=name2" for constraints.
    fn parse_csp(context: &[String]) -> CSP {
        let mut variables: HashMap<String, Vec<String>> = HashMap::new();
        let mut constraints: Vec<Constraint> = Vec::new();

        for line in context {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("var:") {
                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                if parts.len() == 2 {
                    let name = parts[0].to_string();
                    let domain: Vec<String> =
                        parts[1].split(',').map(|v| v.trim().to_string()).collect();
                    variables.insert(name, domain);
                }
            } else if let Some(rest) = trimmed.strip_prefix("constraint:") {
                if let Some(pos) = rest.find("!=") {
                    let left = rest[..pos].trim().to_string();
                    let right = rest[pos + 2..].trim().to_string();
                    constraints.push(Constraint::NotEqual(left, right));
                } else if let Some(pos) = rest.find('=') {
                    let left = rest[..pos].trim().to_string();
                    let right = rest[pos + 1..].trim().to_string();
                    constraints.push(Constraint::Equal(left, right));
                }
            }
        }

        CSP {
            variables,
            constraints,
        }
    }

    /// AC-3 arc consistency: prune domain values that cannot participate in any
    /// consistent assignment.
    fn ac3(csp: &mut CSP) -> bool {
        let mut queue: Vec<(String, String)> = Vec::new();

        // Initialize work queue with all constraint arcs.
        for constraint in &csp.constraints {
            match constraint {
                Constraint::NotEqual(a, b) | Constraint::Equal(a, b) => {
                    queue.push((a.clone(), b.clone()));
                    queue.push((b.clone(), a.clone()));
                }
            }
        }

        while let Some((xi, xj)) = queue.pop() {
            if Self::revise(csp, &xi, &xj) {
                if let Some(domain) = csp.variables.get(&xi)
                    && domain.is_empty()
                {
                    return false; // Domain wiped out — unsatisfiable.
                }
                // Add all neighbors of xi (except xj) back to queue.
                for constraint in &csp.constraints {
                    let neighbor = match constraint {
                        Constraint::NotEqual(a, b) | Constraint::Equal(a, b) => {
                            if a == &xi && b != &xj {
                                Some(b.clone())
                            } else if b == &xi && a != &xj {
                                Some(a.clone())
                            } else {
                                None
                            }
                        }
                    };
                    if let Some(n) = neighbor {
                        queue.push((n, xi.clone()));
                    }
                }
            }
        }

        true
    }

    #[allow(clippy::if_same_then_else)]
    /// Revise domain of xi to be arc-consistent with xj.
    fn revise(csp: &mut CSP, xi: &str, xj: &str) -> bool {
        let mut revised = false;
        let xj_domain = csp.variables.get(xj).cloned().unwrap_or_default();
        let xi_domain = csp.variables.get(xi).cloned().unwrap_or_default();

        let relevant_constraints: Vec<Constraint> = csp
            .constraints
            .iter()
            .filter(|c| match c {
                Constraint::NotEqual(a, b) | Constraint::Equal(a, b) => {
                    (a == xi && b == xj) || (a == xj && b == xi)
                }
            })
            .cloned()
            .collect();

        let mut new_domain = Vec::new();
        for vi in &xi_domain {
            let has_support = xj_domain.iter().any(|vj| {
                relevant_constraints.iter().all(|c| match c {
                    Constraint::NotEqual(a, _) => {
                        if a == xi {
                            vi != vj
                        } else {
                            vj != vi
                        }
                    }
                    Constraint::Equal(a, _) => {
                        if a == xi {
                            vi == vj
                        } else {
                            vj == vi
                        }
                    }
                })
            });
            if has_support {
                new_domain.push(vi.clone());
            } else {
                revised = true;
            }
        }

        if revised {
            csp.variables.insert(xi.to_string(), new_domain);
        }
        revised
    }

    /// Backtracking search for a satisfying assignment.
    fn backtrack(
        csp: &CSP,
        assignment: &mut HashMap<String, String>,
        var_order: &[String],
        idx: usize,
    ) -> bool {
        if idx == var_order.len() {
            return true;
        }

        let var = &var_order[idx];
        let domain = csp.variables.get(var).cloned().unwrap_or_default();

        for value in &domain {
            assignment.insert(var.clone(), value.clone());
            if Self::is_consistent(csp, assignment)
                && Self::backtrack(csp, assignment, var_order, idx + 1)
            {
                return true;
            }
            assignment.remove(var);
        }

        false
    }

    /// Check if current assignment is consistent with all constraints.
    fn is_consistent(csp: &CSP, assignment: &HashMap<String, String>) -> bool {
        csp.constraints.iter().all(|c| match c {
            Constraint::NotEqual(a, b) => {
                match (assignment.get(a), assignment.get(b)) {
                    (Some(va), Some(vb)) => va != vb,
                    _ => true, // Not yet assigned — consistent by default.
                }
            }
            Constraint::Equal(a, b) => match (assignment.get(a), assignment.get(b)) {
                (Some(va), Some(vb)) => va == vb,
                _ => true,
            },
        })
    }
}

#[allow(clippy::upper_case_acronyms)]
struct CSP {
    variables: HashMap<String, Vec<String>>,
    constraints: Vec<Constraint>,
}

#[derive(Clone)]
enum Constraint {
    NotEqual(String, String),
    Equal(String, String),
}

impl ReasoningEngine for ConstraintEngine {
    fn name(&self) -> &str {
        "constraint"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let mut csp = Self::parse_csp(&request.context);

        let satisfiable = Self::ac3(&mut csp);

        let mut items = Vec::new();

        if satisfiable {
            let var_order: Vec<String> = csp.variables.keys().cloned().collect();
            let mut assignment = HashMap::new();

            if Self::backtrack(&csp, &mut assignment, &var_order, 0) {
                for (var, val) in &assignment {
                    items.push(ReasoningItem {
                        label: var.clone(),
                        description: format!("{} = {}", var, val),
                        confidence: 1.0,
                        supporting_evidence: vec!["Satisfies all constraints".to_string()],
                    });
                }
            } else {
                items.push(ReasoningItem {
                    label: "unsatisfiable".to_string(),
                    description: "No satisfying assignment found after backtracking search"
                        .to_string(),
                    confidence: 1.0,
                    supporting_evidence: vec!["Exhaustive search completed".to_string()],
                });
            }
        } else {
            items.push(ReasoningItem {
                label: "unsatisfiable".to_string(),
                description: "Arc consistency (AC-3) proved unsatisfiability".to_string(),
                confidence: 1.0,
                supporting_evidence: vec!["Domain wiped out during AC-3".to_string()],
            });
        }

        let confidence = if items.iter().any(|i| i.label == "unsatisfiable") {
            0.0
        } else {
            1.0
        };

        Ok(ReasoningOutput {
            engine_name: "constraint".to_string(),
            result_type: ResultType::Assignments,
            items,
            confidence,
            provenance: vec![format!("query: {}", request.query)],
        })
    }
}

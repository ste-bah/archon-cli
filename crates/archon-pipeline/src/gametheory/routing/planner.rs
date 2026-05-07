use std::collections::{HashMap, HashSet};

use super::super::errors::GameTheoryError;
use super::super::fingerprint::GameTheoryFingerprint;
use super::conditions::{EvalContext, eval_expr, parse_condition};
use super::dag::detect_cycle;
use super::types::{AgentEntry, GameTheorySpec, RoutingDecision};

/// Evaluate a routing specification against a fingerprint.
///
/// Processes tiers in order, evaluates each non-mandatory agent's condition,
/// and collects enabled/skipped specialists. Mandatory agents are always enabled.
/// Detects dependency cycles and returns deterministic results.
pub fn evaluate_routing(
    spec: &GameTheorySpec,
    fingerprint: &GameTheoryFingerprint,
    run_id: &str,
    created_at: &str,
) -> Result<RoutingDecision, GameTheoryError> {
    let mut enabled: Vec<String> = Vec::new();
    let mut enabled_set: HashSet<String> = HashSet::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut evaluated: Vec<(String, bool)> = Vec::new();

    // Build dependency map for cycle detection
    let mut dep_map: HashMap<String, Vec<String>> = HashMap::new();
    for tier in &spec.tiers {
        for agent in &tier.agents {
            if !agent.depends_on.is_empty() {
                dep_map.insert(agent.key.clone(), agent.depends_on.clone());
            }
        }
    }

    // Check for cycles
    for key in dep_map.keys() {
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        if let Some(cycle) = detect_cycle(key, &dep_map, &mut visited, &mut path) {
            return Err(GameTheoryError::RoutingCycle { cycle });
        }
    }

    // Also verify all dependency targets exist in the spec
    let all_keys: HashSet<&str> = spec
        .tiers
        .iter()
        .flat_map(|t| t.agents.iter().map(|a| a.key.as_str()))
        .collect();
    for tier in &spec.tiers {
        for agent in &tier.agents {
            for dep in &agent.depends_on {
                if !all_keys.contains(dep.as_str()) {
                    return Err(GameTheoryError::ConditionError {
                        expression: format!("depends_on: {}", dep),
                        message: format!(
                            "agent '{}' depends on '{}' which is not in the spec",
                            agent.key, dep
                        ),
                    });
                }
            }
        }
    }

    let mut enabled_count: i64 = 0;
    let mut nash_count: i64 = 0;

    // Process tiers in order
    for tier in &spec.tiers {
        for agent in &tier.agents {
            if agent.mandatory {
                if let Some(reason) = unmet_dependency_reason(agent, &enabled_set) {
                    skipped.push((agent.key.clone(), reason));
                } else {
                    enable_agent(
                        &agent.key,
                        &mut enabled,
                        &mut enabled_set,
                        &mut enabled_count,
                        &mut nash_count,
                    );
                }
                continue;
            }

            let condition = match &agent.condition {
                Some(c) => c.clone(),
                None => {
                    // No condition = enabled when its dependency chain is enabled.
                    if let Some(reason) = unmet_dependency_reason(agent, &enabled_set) {
                        skipped.push((agent.key.clone(), reason));
                    } else {
                        enable_agent(
                            &agent.key,
                            &mut enabled,
                            &mut enabled_set,
                            &mut enabled_count,
                            &mut nash_count,
                        );
                    }
                    continue;
                }
            };

            let ctx = EvalContext {
                fingerprint,
                nash_count,
                enabled_count,
            };

            match parse_condition(&condition).and_then(|expr| eval_expr(&expr, &ctx)) {
                Ok(true) => {
                    evaluated.push((condition.clone(), true));
                    if let Some(reason) = unmet_dependency_reason(agent, &enabled_set) {
                        skipped.push((agent.key.clone(), reason));
                    } else {
                        enable_agent(
                            &agent.key,
                            &mut enabled,
                            &mut enabled_set,
                            &mut enabled_count,
                            &mut nash_count,
                        );
                    }
                }
                Ok(false) => {
                    evaluated.push((condition.clone(), false));
                    skipped.push((agent.key.clone(), "condition evaluated to false".into()));
                }
                Err(e) => {
                    return Err(GameTheoryError::ConditionError {
                        expression: condition.clone(),
                        message: e,
                    });
                }
            }
        }
    }

    Ok(RoutingDecision {
        run_id: run_id.to_string(),
        fingerprint_id: fingerprint.run_id.clone(),
        enabled_specialists: enabled,
        skipped_specialists: skipped,
        evaluated_conditions: evaluated,
        created_at: created_at.to_string(),
    })
}

fn enable_agent(
    key: &str,
    enabled: &mut Vec<String>,
    enabled_set: &mut HashSet<String>,
    enabled_count: &mut i64,
    nash_count: &mut i64,
) {
    enabled.push(key.to_string());
    enabled_set.insert(key.to_string());
    *enabled_count += 1;
    if key.contains("nash") {
        *nash_count += 1;
    }
}

fn unmet_dependency_reason(agent: &AgentEntry, enabled_set: &HashSet<String>) -> Option<String> {
    let missing: Vec<&str> = agent
        .depends_on
        .iter()
        .filter(|dep| !enabled_set.contains(dep.as_str()))
        .map(String::as_str)
        .collect();

    if missing.is_empty() {
        None
    } else {
        Some(format!("dependency not enabled: {}", missing.join(", ")))
    }
}

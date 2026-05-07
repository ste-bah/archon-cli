use std::collections::{HashMap, HashSet};

use super::super::routing::{GameTheorySpec, RoutingDecision};
use super::costs::agent_tier;

/// Build a dependency map from spec agent entries.
pub(super) fn build_dependency_map(spec: &GameTheorySpec) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for tier in &spec.tiers {
        for agent in &tier.agents {
            map.insert(agent.key.clone(), agent.depends_on.clone());
        }
    }
    map
}

pub(super) fn apply_policy_gates_to_routing(
    routing: &mut RoutingDecision,
    dep_map: &HashMap<String, Vec<String>>,
    enable_tier11: bool,
) {
    let mut skipped = Vec::new();
    let mut enabled: Vec<String> = routing.enabled_specialists.clone();
    loop {
        let enabled_set: HashSet<String> = enabled.iter().cloned().collect();
        let before = enabled.len();
        enabled.retain(|agent_key| {
            if !enable_tier11 && agent_tier(agent_key) == Some(11) {
                skipped.push((
                    agent_key.clone(),
                    "Tier 11 disabled: pass --enable-tier11 and set policy.gametheory.enable_tier11=true".to_string(),
                ));
                return false;
            }
            if let Some(missing_dep) = dep_map
                .get(agent_key)
                .and_then(|deps| deps.iter().find(|dep| !enabled_set.contains(*dep)))
            {
                skipped.push((
                    agent_key.clone(),
                    format!("dependency '{missing_dep}' was skipped by policy"),
                ));
                return false;
            }
            true
        });
        if enabled.len() == before {
            break;
        }
    }
    routing.enabled_specialists = enabled;
    routing.skipped_specialists.extend(skipped);
}

use std::collections::{HashMap, HashSet};

// ── Cycle detection ────────────────────────────────────────────────────────

/// Detect cycles in the dependency graph. Returns the cycle path if found.
pub(super) fn detect_cycle(
    agent_key: &str,
    deps: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    path: &mut Vec<String>,
) -> Option<String> {
    if path.contains(&agent_key.to_string()) {
        // Build cycle description
        let cycle_start = path.iter().position(|k| k == agent_key).unwrap();
        let cycle: Vec<_> = path[cycle_start..].to_vec();
        let mut cycle_rep = cycle.join(" -> ");
        cycle_rep.push_str(" -> ");
        cycle_rep.push_str(agent_key);
        return Some(cycle_rep);
    }
    if visited.contains(agent_key) {
        return None;
    }
    visited.insert(agent_key.to_string());
    path.push(agent_key.to_string());

    if let Some(dep_list) = deps.get(agent_key) {
        for dep in dep_list {
            if let Some(cycle) = detect_cycle(dep, deps, visited, path) {
                return Some(cycle);
            }
        }
    }

    path.pop();
    None
}

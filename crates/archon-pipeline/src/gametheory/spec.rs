//! Pipeline spec builder for Tier 1 game-theory execution and Phase 4
//! specialist DAG-wave generation.
//!
//! Builds a [`PipelineSpec`] from the mandatory Tier 1 agents (single parallel
//! wave) or from a routing decision with dependency depth grouping and
//! tier-level concurrency caps.

use std::collections::HashMap;

use super::agents::GameTheoryAgent;
use super::registry::GAMETHEORY_AGENTS;
use super::routing::{GameTheorySpec, RoutingDecision};
use crate::spec::{PipelineSpec, StepSpec};

/// Build a Tier 1 [`PipelineSpec`] from the mandatory game-theory agents.
///
/// All mandatory agents are placed in one parallel wave — each step has no
/// `depends_on` edges, so the DAG builder assigns them to a single level.
pub fn build_tier1_spec(situation: &str) -> PipelineSpec {
    let tier1: Vec<&GameTheoryAgent> = GAMETHEORY_AGENTS.iter().filter(|a| a.mandatory).collect();

    let steps: Vec<StepSpec> = tier1
        .iter()
        .map(|agent| StepSpec {
            id: agent.key.to_string(),
            agent: agent.key.to_string(),
            input: serde_json::json!({
                "situation": situation,
                "prompt_source": agent.prompt_source_path,
            }),
            depends_on: vec![],
            retry: Default::default(),
            timeout_secs: 600,
            condition: None,
            on_failure: Default::default(),
        })
        .collect();

    PipelineSpec {
        name: "gametheory-tier1-classify".into(),
        version: "1.0".into(),
        global_timeout_secs: 3600,
        max_parallelism: 4,
        steps,
    }
}

/// Build a specialist execution [`PipelineSpec`] from a routing decision.
///
/// Each enabled specialist becomes a step. Dependencies come from the
/// `dependency_map` (agent_key → depends_on keys). The concurrency cap is
/// the maximum tier-level cap from the spec. Skipped specialists are omitted.
///
/// The resulting DAG groups agents into waves by dependency depth: agents with
/// no dependencies run in wave 0; agents depending only on wave-0 agents run in
/// wave 1, and so on.
pub fn build_specialist_spec(
    routing_decision: &RoutingDecision,
    dependency_map: &HashMap<String, Vec<String>>,
    spec: &GameTheorySpec,
    situation: &str,
) -> PipelineSpec {
    let steps: Vec<StepSpec> = routing_decision
        .enabled_specialists
        .iter()
        .map(|agent_key| {
            let depends_on = dependency_map
                .get(agent_key)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|dep| routing_decision.enabled_specialists.contains(dep))
                .collect();
            StepSpec {
                id: agent_key.clone(),
                agent: agent_key.clone(),
                input: serde_json::json!({"situation": situation}),
                depends_on,
                retry: Default::default(),
                timeout_secs: 600,
                condition: None,
                on_failure: Default::default(),
            }
        })
        .collect();

    let max_parallelism = spec
        .tiers
        .iter()
        .map(|t| t.concurrency_cap as u32)
        .max()
        .unwrap_or(4);

    PipelineSpec {
        name: "gametheory-specialist-execution".into(),
        version: "1.0".into(),
        global_timeout_secs: 3600,
        max_parallelism,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::DagBuilder;

    #[test]
    fn test_dag_executes_tier1_in_parallel_wave() {
        let spec = build_tier1_spec("Two firms set prices.");
        let mandatory = GAMETHEORY_AGENTS.iter().filter(|a| a.mandatory).count();
        assert_eq!(
            spec.steps.len(),
            mandatory,
            "all mandatory agents are steps"
        );
        assert!(spec.steps.len() > 1, "need >1 agent for parallel wave test");

        // No inter-dependencies -> all steps in a single DAG level
        let dag = DagBuilder::build(&spec).expect("DAG build should succeed");
        assert_eq!(
            dag.levels.len(),
            1,
            "mandatory agents must be one parallel wave"
        );
        assert_eq!(
            dag.levels[0].len(),
            spec.steps.len(),
            "all steps in level 0"
        );
    }

    #[test]
    fn test_tier1_spec_has_no_dependencies() {
        let spec = build_tier1_spec("A simple game.");
        for step in &spec.steps {
            assert!(
                step.depends_on.is_empty(),
                "step {} must not have dependencies",
                step.id
            );
        }
    }

    fn make_test_routing_decision(enabled: &[&str]) -> RoutingDecision {
        RoutingDecision {
            run_id: "test-run".into(),
            fingerprint_id: "test-fp".into(),
            enabled_specialists: enabled.iter().map(|s| s.to_string()).collect(),
            skipped_specialists: vec![],
            evaluated_conditions: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn make_test_spec(concurrency_caps: &[usize]) -> GameTheorySpec {
        GameTheorySpec {
            version: "1.0".into(),
            spec_id: "test".into(),
            cost_cap_usd: 5.0,
            tiers: concurrency_caps
                .iter()
                .enumerate()
                .map(|(i, cap)| super::super::routing::TierEntry {
                    id: i as u8 + 1,
                    name: format!("Tier {}", i + 1),
                    concurrency_cap: *cap,
                    agents: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn test_specialist_spec_dag_depth_grouping() {
        // Agents: A depends on nothing, B depends on A, C depends on B
        // Expected DAG: 3 levels (A alone, then B, then C)
        let routing = make_test_routing_decision(&["A", "B", "C"]);
        let mut deps = HashMap::new();
        deps.insert("A".to_string(), vec![]);
        deps.insert("B".to_string(), vec!["A".to_string()]);
        deps.insert("C".to_string(), vec!["B".to_string()]);
        let spec = make_test_spec(&[4, 3, 2]);

        let pipeline = build_specialist_spec(&routing, &deps, &spec, "Test situation.");
        let dag = DagBuilder::build(&pipeline).expect("DAG build should succeed");

        assert_eq!(
            dag.levels.len(),
            3,
            "three dependency depths → 3 DAG levels"
        );
        assert_eq!(dag.levels[0], vec!["A"]);
        assert_eq!(dag.levels[1], vec!["B"]);
        assert_eq!(dag.levels[2], vec!["C"]);
    }

    #[test]
    fn test_specialist_spec_respects_concurrency_cap() {
        // Tier caps: [2, 5, 3] → max_parallelism = 5
        let routing = make_test_routing_decision(&["X", "Y"]);
        let deps = HashMap::new();
        let spec = make_test_spec(&[2, 5, 3]);

        let pipeline = build_specialist_spec(&routing, &deps, &spec, "Situation.");
        assert_eq!(
            pipeline.max_parallelism, 5,
            "max concurrency cap across tiers"
        );

        // Single tier → uses that tier's cap
        let spec2 = make_test_spec(&[7]);
        let pipeline2 = build_specialist_spec(&routing, &deps, &spec2, "Situation.");
        assert_eq!(pipeline2.max_parallelism, 7);
    }
}

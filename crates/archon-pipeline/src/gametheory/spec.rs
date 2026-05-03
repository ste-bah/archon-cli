//! Pipeline spec builder for Tier 1 game-theory execution.
//!
//! Builds a [`PipelineSpec`] from the mandatory Tier 1 agents, placing them
//! in a single parallel wave (no inter-dependencies).

use crate::spec::{PipelineSpec, StepSpec};
use super::agents::GameTheoryAgent;
use super::registry::GAMETHEORY_AGENTS;

/// Build a Tier 1 [`PipelineSpec`] from the mandatory game-theory agents.
///
/// All mandatory agents are placed in one parallel wave — each step has no
/// `depends_on` edges, so the DAG builder assigns them to a single level.
pub fn build_tier1_spec(situation: &str) -> PipelineSpec {
    let tier1: Vec<&GameTheoryAgent> = GAMETHEORY_AGENTS
        .iter()
        .filter(|a| a.mandatory)
        .collect();

    let steps: Vec<StepSpec> = tier1
        .iter()
        .map(|agent| {
            StepSpec {
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
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::DagBuilder;

    #[test]
    fn test_dag_executes_tier1_in_parallel_wave() {
        let spec = build_tier1_spec("Two firms set prices.");
        let mandatory = GAMETHEORY_AGENTS
            .iter()
            .filter(|a| a.mandatory)
            .count();
        assert_eq!(spec.steps.len(), mandatory, "all mandatory agents are steps");
        assert!(spec.steps.len() > 1, "need >1 agent for parallel wave test");

        // No inter-dependencies -> all steps in a single DAG level
        let dag = DagBuilder::build(&spec).expect("DAG build should succeed");
        assert_eq!(dag.levels.len(), 1, "mandatory agents must be one parallel wave");
        assert_eq!(dag.levels[0].len(), spec.steps.len(), "all steps in level 0");
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
}

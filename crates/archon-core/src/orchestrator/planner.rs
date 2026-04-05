use super::events::{Subtask, SubtaskStatus};

/// Parse a coordinator agent's JSON plan output into a list of subtasks.
/// The coordinator must output a JSON object with a "subtasks" array, e.g.:
/// {"subtasks":[{"id":"1","description":"...","agent_type":"coder","dependencies":[]}]}
/// The JSON may be embedded in surrounding text — the parser locates the first `{`.
pub fn parse_plan(coordinator_output: &str) -> anyhow::Result<Vec<Subtask>> {
    let json_start = coordinator_output
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("coordinator output contains no JSON object"))?;

    #[derive(serde::Deserialize)]
    struct Plan {
        subtasks: Vec<SubtaskSpec>,
    }
    #[derive(serde::Deserialize)]
    struct SubtaskSpec {
        id: String,
        description: String,
        agent_type: String,
        #[serde(default)]
        dependencies: Vec<String>,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    }
    fn default_max_retries() -> u32 {
        2
    }

    let plan: Plan = serde_json::from_str(&coordinator_output[json_start..])
        .map_err(|e| anyhow::anyhow!("failed to parse coordinator plan JSON: {e}"))?;

    Ok(plan
        .subtasks
        .into_iter()
        .map(|s| Subtask {
            id: s.id,
            description: s.description,
            agent_type: s.agent_type,
            dependencies: s.dependencies,
            status: SubtaskStatus::Pending,
            retries: 0,
            max_retries: s.max_retries,
        })
        .collect())
}

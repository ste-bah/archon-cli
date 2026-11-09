//! Candidate readers retain authored fields and ignore host-owned echoes.
use super::*;
#[derive(Deserialize)]
struct Candidate {
    schema_version: u32,
    acceptance_digest: String,
    tasks: Vec<Task>,
}
#[derive(Deserialize)]
struct Task {
    task_id: String,
    file_name: String,
    #[serde(default)]
    depends_on: Vec<FrozenDependency>,
    #[serde(default)]
    blocks: Vec<String>,
    #[serde(default)]
    implements: Vec<String>,
    #[serde(default)]
    source_sections: Vec<String>,
    #[serde(default)]
    deliverable_contracts: Vec<WorkflowV2DeliverableContract>,
    #[serde(default, rename = "status", deserialize_with = "discard")]
    _status: (),
    #[serde(default, rename = "requirement_states", deserialize_with = "discard")]
    _states: (),
    #[serde(default, rename = "proven_done", deserialize_with = "discard")]
    _proofs: (),
}
pub fn read_candidate(bytes: &[u8]) -> Result<TaskSkeleton, serde_json::Error> {
    let candidate: Candidate = serde_json::from_slice(bytes)?;
    Ok(TaskSkeleton {
        schema_version: candidate.schema_version,
        acceptance_digest: candidate.acceptance_digest,
        tasks: candidate
            .tasks
            .into_iter()
            .map(|task| FrozenTask {
                task_id: task.task_id,
                file_name: task.file_name,
                depends_on: task.depends_on,
                blocks: task.blocks,
                implements: task.implements,
                source_sections: task.source_sections,
                deliverable_contracts: task.deliverable_contracts,
                status: None,
                requirement_states: Default::default(),
                proven_done: Vec::new(),
            })
            .collect(),
    })
}

fn discard<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    let _ = serde_json::Value::deserialize(deserializer)?;
    Ok(())
}

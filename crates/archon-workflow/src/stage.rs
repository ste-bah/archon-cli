use std::collections::{BTreeMap, BTreeSet};

use crate::error::{WorkflowError, WorkflowResult};
use crate::run::{StageStatus, WorkflowRun};
use crate::spec::{StageSpec, WorkflowSpec};

pub fn ordered_stages(spec: &WorkflowSpec) -> WorkflowResult<Vec<StageSpec>> {
    let mut remaining: BTreeMap<String, StageSpec> = spec
        .stages
        .iter()
        .map(|stage| (stage.id.clone(), stage.clone()))
        .collect();
    let mut accepted = BTreeSet::new();
    let mut ordered = Vec::new();
    while !remaining.is_empty() {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|(_, stage)| stage.depends_on.iter().all(|dep| accepted.contains(dep)))
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            return Err(WorkflowError::DependencyCycle(
                remaining.keys().cloned().collect(),
            ));
        }
        for id in ready {
            if let Some(stage) = remaining.remove(&id) {
                accepted.insert(id);
                ordered.push(stage);
            }
        }
    }
    Ok(ordered)
}

pub fn stage_ready(run: &WorkflowRun, stage: &StageSpec) -> bool {
    run.stages
        .get(&stage.id)
        .is_some_and(|state| state.status == StageStatus::Pending)
        && stage.depends_on.iter().all(|dep| run.accepted_stage(dep))
}

pub fn source_input_hash(stage: &StageSpec) -> String {
    let body = match serde_json::to_vec(stage) {
        Ok(body) => body,
        Err(_) => stage.id.as_bytes().to_vec(),
    };
    blake3::hash(&body).to_hex().to_string()
}

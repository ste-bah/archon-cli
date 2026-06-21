use serde_json::Value;

use crate::policy::WorkflowPolicy;
use crate::run::WorkflowRun;
use crate::spec::{ProviderTier, StageSpec};

pub(super) fn item_target_files(stage: &StageSpec, payload: &Value) -> Vec<String> {
    let mut targets = string_list(payload.get("target_files"))
        .into_iter()
        .chain(string_list(payload.get("expected_target_files")))
        .chain(string_list(payload.get("target_file")))
        .chain(string_list(payload.get("target_path")))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        targets = stage.expected_target_files.clone();
    }
    targets.retain(|target| !target.trim().is_empty());
    targets
}

pub(super) fn stage_max_agents(
    policy: &WorkflowPolicy,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> usize {
    let base = run.spec.max_agents.min(policy.max_agents_per_run);
    if stage.provider_tier == Some(ProviderTier::Local) {
        base.min(policy.local_provider_max_agents) as usize
    } else {
        base as usize
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

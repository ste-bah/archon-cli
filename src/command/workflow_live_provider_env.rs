use std::collections::BTreeSet;

use archon_tools::provider_env::{
    ProviderEnvPolicy, ProviderEnvProof, ProviderEnvResolution, provider_env_internal_marker,
    resolve_operator_provider_env,
};
use archon_workflow::{WorkflowV2AgentRequest, WorkflowV2Evidence, WorkflowV2EvidenceKind};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;

pub(super) const PROVIDER_ENV_CONTRACT_VERSION: &str = "provider-env-proof-v1";

#[derive(Debug, Clone)]
pub(super) struct PreparedProviderEnv {
    pub(super) policy: ProviderEnvPolicy,
    pub(super) proof: ProviderEnvProof,
}

pub(super) async fn prepare_provider_env_for_v2_request(
    request: &mut WorkflowV2AgentRequest,
    workflow_resolution: Option<&ProviderEnvResolution>,
) -> Option<PreparedProviderEnv> {
    let mut policy = provider_env_policy_from_input(&request.input)?;
    let authorized = workflow_resolution?;
    policy.required_keys.retain(|key| {
        authorized
            .proof
            .redacted_env_keys_checked
            .iter()
            .any(|proof| &proof.key == key)
    });
    if policy.is_empty() {
        return None;
    }
    let resolved = match workflow_resolution.filter(|resolved| resolved.covers(&policy)) {
        Some(resolved) => resolved.restricted_to(&policy),
        None => return None,
    };
    attach_provider_env_to_input(&mut request.input, &policy, &resolved.proof);
    request.constraints.push(
        "Provider-sensitive verification must report provider_env_proof with redacted key names only; never print or persist credential values.".to_string(),
    );
    Some(PreparedProviderEnv {
        policy,
        proof: resolved.proof,
    })
}

pub(super) async fn resolve_generated_workflow_provider_env(
    task_universe: Option<&WorkflowV2TaskUniverse>,
    operator_grants: &[String],
) -> Option<ProviderEnvResolution> {
    let keys = task_universe
        .into_iter()
        .flat_map(|universe| &universe.tasks)
        .flat_map(|task| task.required_env_keys.iter().cloned())
        .collect::<BTreeSet<_>>();
    if keys.is_empty() {
        return None;
    }
    let mut policy = ProviderEnvPolicy::new(keys.into_iter().collect());
    policy.reason = Some("generated workflow run-scoped provider environment".to_string());
    // Task-universe declarations may request a subset, never add grants.
    // No operator-selected profile input is recorded here: execute none.
    Some(resolve_operator_provider_env(&policy, operator_grants, &[]).await)
}

pub(super) fn provider_env_tool_markers(
    request: &WorkflowV2AgentRequest,
) -> Vec<serde_json::Value> {
    provider_env_policy_from_input(&request.input)
        .filter(|policy| !policy.is_empty())
        .map(|policy| vec![provider_env_internal_marker(&policy)])
        .unwrap_or_default()
}

pub(super) fn stamp_provider_env_result(
    result: &mut archon_workflow::WorkflowV2Result,
    prepared: Option<&PreparedProviderEnv>,
) {
    let Some(prepared) = prepared else {
        return;
    };
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "provider_env_contract_version".to_string(),
        serde_json::json!(PROVIDER_ENV_CONTRACT_VERSION),
    );
    data.insert(
        "provider_env_proof".to_string(),
        serde_json::json!(prepared.proof),
    );
    data.insert(
        "provider_env_required_keys".to_string(),
        serde_json::json!(prepared.policy.required_keys),
    );
    result.data = serde_json::Value::Object(data);
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "provider environment preflight checked required key names and stored redacted proof",
    ));
}

fn attach_provider_env_to_input(
    input: &mut serde_json::Value,
    policy: &ProviderEnvPolicy,
    proof: &ProviderEnvProof,
) {
    let mut root = input.as_object().cloned().unwrap_or_default();
    root.insert(
        "provider_env_contract_version".to_string(),
        serde_json::json!(PROVIDER_ENV_CONTRACT_VERSION),
    );
    root.insert("provider_env_policy".to_string(), serde_json::json!(policy));
    root.insert("provider_env_proof".to_string(), serde_json::json!(proof));
    if let Some(item) = root
        .get_mut("item")
        .and_then(serde_json::Value::as_object_mut)
    {
        item.insert("provider_env_policy".to_string(), serde_json::json!(policy));
        item.insert("provider_env_proof".to_string(), serde_json::json!(proof));
    }
    *input = serde_json::Value::Object(root);
}

pub(super) fn provider_env_policy_from_input(
    input: &serde_json::Value,
) -> Option<ProviderEnvPolicy> {
    let mut keys = provider_env_keys(input);
    if let Some(item) = input.get("item") {
        keys.extend(provider_env_keys(item));
    }
    let keys: Vec<String> = keys
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if keys.is_empty() {
        return None;
    }
    let mut policy = ProviderEnvPolicy::new(keys);
    // Workflow input can request names, but cannot choose executable profiles.
    policy.profile_sources.clear();
    policy.reason = Some("generated workflow provider-sensitive verification".to_string());
    Some(policy)
}

fn provider_env_keys(value: &serde_json::Value) -> Vec<String> {
    let aliases = [
        "provider_env_requirements",
        "providerEnvRequirements",
        "provider_env_required_keys",
        "providerEnvRequiredKeys",
        "required_env_keys",
        "requiredEnvKeys",
        "credential_env_keys",
        "credentialEnvKeys",
        "env_keys",
        "envKeys",
    ];
    aliases
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(value_strings)
        .collect()
}

fn value_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        serde_json::Value::Array(values) => values.iter().flat_map(value_strings).collect(),
        serde_json::Value::Object(object) => object
            .get("required_keys")
            .or_else(|| object.get("requiredKeys"))
            .or_else(|| object.get("keys"))
            .or_else(|| object.get("env_keys"))
            .or_else(|| object.get("envKeys"))
            .map(value_strings)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "workflow_live_provider_env_tests.rs"]
mod tests;

/// The operator launch document's existing structured required_env_keys list.
/// Prose (including negative mentions), malformed lists and absent declarations
/// grant nothing. Generated task-universe requirements are never declarations.
pub(super) fn operator_provider_env_grants(launch: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Declaration {
        #[serde(default)]
        required_env_keys: Vec<String>,
    }
    let keys = serde_yaml_ng::from_str::<Declaration>(launch)
        .map(|declaration| declaration.required_env_keys)
        .unwrap_or_default();
    ProviderEnvPolicy::new(keys).required_keys
}

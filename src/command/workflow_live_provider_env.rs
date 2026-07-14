use std::collections::BTreeSet;

use archon_tools::provider_env::{
    ProviderEnvPolicy, ProviderEnvProof, provider_env_internal_marker, resolve_provider_env,
};
use archon_workflow::{WorkflowV2AgentRequest, WorkflowV2Evidence, WorkflowV2EvidenceKind};

pub(super) const PROVIDER_ENV_CONTRACT_VERSION: &str = "provider-env-proof-v1";

#[derive(Debug, Clone)]
pub(super) struct PreparedProviderEnv {
    pub(super) policy: ProviderEnvPolicy,
    pub(super) proof: ProviderEnvProof,
}

pub(super) async fn prepare_provider_env_for_v2_request(
    request: &mut WorkflowV2AgentRequest,
) -> Option<PreparedProviderEnv> {
    let policy = provider_env_policy_from_input(&request.input)?;
    if policy.is_empty() {
        return None;
    }
    let resolved = resolve_provider_env(&policy).await;
    attach_provider_env_to_input(&mut request.input, &policy, &resolved.proof);
    request.constraints.push(
        "Provider-sensitive verification must report provider_env_proof with redacted key names only; never print or persist credential values.".to_string(),
    );
    Some(PreparedProviderEnv {
        policy,
        proof: resolved.proof,
    })
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
    if contains_canonical_task(input, "TASK-TDL-080") {
        keys.push("POLYGON_API_KEY".to_string());
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
    policy.profile_sources = profile_sources(input);
    policy.reason = Some("generated workflow provider-sensitive verification".to_string());
    Some(policy)
}

fn contains_canonical_task(value: &serde_json::Value, expected: &str) -> bool {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_canonical_task(value, expected)),
        serde_json::Value::Object(object) => {
            let owns_task = [
                "canonical_task_ids",
                "canonicalTaskIds",
                "task_id",
                "taskId",
            ]
            .iter()
            .filter_map(|key| object.get(*key))
            .flat_map(value_strings)
            .any(|task_id| task_id == expected);
            owns_task
                || object
                    .get("item")
                    .is_some_and(|item| contains_canonical_task(item, expected))
        }
        serde_json::Value::String(value) => value == expected,
        _ => false,
    }
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

fn profile_sources(value: &serde_json::Value) -> Vec<String> {
    let sources = ["profile_sources", "profileSources"]
        .iter()
        .filter_map(|key| value.get(*key))
        .flat_map(value_strings)
        .collect::<Vec<_>>();
    if sources.is_empty() {
        vec!["~/.profile".to_string()]
    } else {
        sources
    }
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
        serde_json::Value::Object(object) => {
            let mut values = object
                .get("required_keys")
                .or_else(|| object.get("requiredKeys"))
                .or_else(|| object.get("keys"))
                .or_else(|| object.get("env_keys"))
                .or_else(|| object.get("envKeys"))
                .map(value_strings)
                .unwrap_or_default();
            values.extend(provider_key_hints(object));
            values
        }
        _ => Vec::new(),
    }
}

fn provider_key_hints(object: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    [
        "provider",
        "provider_name",
        "providerName",
        "provider_id",
        "providerId",
    ]
    .iter()
    .filter_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
    .flat_map(provider_keys_for_name)
    .collect()
}

fn provider_keys_for_name(provider: &str) -> Vec<String> {
    let normalized = provider.trim().to_ascii_lowercase();
    let mut keys = Vec::new();
    if normalized.contains("polygon") || normalized.contains("openbb") {
        keys.push("POLYGON_API_KEY".to_string());
    }
    if normalized.contains("tiingo") {
        keys.push("TIINGO_TOKEN".to_string());
    }
    if normalized.contains("fmp") || normalized.contains("financial modeling prep") {
        keys.push("FMP_API_KEY".to_string());
    }
    if normalized.contains("finnhub") {
        keys.push("FINNHUB_API_KEY".to_string());
    }
    keys
}

#[cfg(test)]
#[path = "workflow_live_provider_env_tests.rs"]
mod tests;

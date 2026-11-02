//! Runtime authority supplied only by an operator launch boundary.
use super::*;

/// Host-only launch boundary: names come from a structured operator declaration.
/// Profiles must be selected by that same operator boundary, never copied from
/// a workflow policy or marker.
pub async fn resolve_operator_provider_env(
    policy: &ProviderEnvPolicy,
    operator_grants: &[String],
    operator_profiles: &[String],
) -> ProviderEnvResolution {
    let declared: std::collections::BTreeSet<_> = normalize_keys(operator_grants.to_vec())
        .into_iter()
        .collect();
    let mut approved = policy.clone();
    approved.required_keys = normalize_keys(policy.required_keys.clone())
        .into_iter()
        .filter(|key| declared.contains(key))
        .filter(|key| archon_shell::data_environment::execution_control(key).is_none())
        .collect();
    let supported = archon_shell::read_boundary::operator_grants_supported();
    if !supported {
        tracing::warn!(
            operator_env_grants = "disabled",
            "operator grants disabled for this run: kernel write containment unavailable"
        );
    }
    approved.profile_sources = if supported {
        operator_profiles.to_vec()
    } else {
        Vec::new()
    };
    resolve_provider_env_with_timeout(
        &approved,
        Duration::from_millis(PROFILE_TIMEOUT_MS),
        supported,
    )
    .await
}

pub async fn resolve_provider_env(policy: &ProviderEnvPolicy) -> ProviderEnvResolution {
    // Serializable workflow/tool input is a request, never an operator grant.
    let mut requested = policy.clone();
    requested.profile_sources.clear();
    resolve_provider_env_with_timeout(&requested, Duration::from_millis(PROFILE_TIMEOUT_MS), false)
        .await
}

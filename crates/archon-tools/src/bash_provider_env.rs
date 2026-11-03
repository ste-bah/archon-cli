//! Overlay only the host-authorized provider resolution.
use super::*;

pub(super) async fn provider_env_overlay(
    source: Option<&ProviderEnvSource>,
) -> Option<ProviderEnvResolution> {
    let source = source?;
    if let Some(resolved) = source.resolution() {
        return Some(
            source
                .policy()
                .map_or_else(|| resolved.clone(), |policy| resolved.restricted_to(policy)),
        );
    }
    let policy = source.policy()?;
    Some(crate::provider_env::resolve_provider_env(policy).await)
}

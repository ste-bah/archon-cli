use std::collections::BTreeMap;

use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;

pub(crate) fn freeze_provider_environment(
    env_vars: &ArchonEnvVars,
    config: &ArchonConfig,
) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (name, value) in [
        ("ANTHROPIC_API_KEY", env_vars.anthropic_api_key.as_ref()),
        ("ARCHON_API_KEY", env_vars.archon_api_key.as_ref()),
        ("ARCHON_OAUTH_TOKEN", env_vars.archon_oauth_token.as_ref()),
        ("ARCHON_MODEL", env_vars.model.as_ref()),
        ("ARCHON_EFFORT", env_vars.effort.as_ref()),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            environment.insert(name.to_string(), value.clone());
        }
    }
    if let Some(path) = &env_vars.config_dir {
        environment.insert(
            "ARCHON_CONFIG_DIR".to_string(),
            path.to_string_lossy().into_owned(),
        );
    }
    // Host commands run as separate CLI processes. Preserve the provider URL
    // resolved by the parent even when it came from config.toml rather than
    // the caller's environment; otherwise a child can silently fall back to
    // the public Anthropic endpoint.
    if let Some(base_url) = &config.api.base_url {
        environment.insert("ANTHROPIC_BASE_URL".to_string(), base_url.clone());
    }
    environment
}

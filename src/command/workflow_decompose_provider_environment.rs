use std::collections::BTreeMap;

use archon_core::env_vars::ArchonEnvVars;

pub(crate) fn freeze_provider_environment(
    config: &archon_core::config::ArchonConfig,
    env_vars: &ArchonEnvVars,
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
    let names = super::super::workflow_provider_route::bindings(config)
        .into_values()
        .flat_map(|binding| {
            binding
                .api_key_env
                .into_iter()
                .chain(binding.base_url_env)
                .chain(binding.credential_env)
        });
    environment.extend(archon_core::config::model_policy::capture_environment(
        names,
        |name| std::env::var(name).ok(),
    ));
    if let Some(path) = &env_vars.config_dir {
        environment.insert(
            "ARCHON_CONFIG_DIR".to_string(),
            path.to_string_lossy().into_owned(),
        );
    }
    environment
}

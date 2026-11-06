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

#[cfg(test)]
#[test]
fn freeze_child_captures_credentials_but_excludes_ambient_endpoint() {
    if crate::test_environment::isolated() {
        return;
    }
    let mut config = archon_core::config::ArchonConfig::default();
    config.llm.provider = "freeze-fixture".into();
    config.llm.provider_environment.insert(
        "freeze-fixture".into(),
        archon_core::config::ProviderEnvironmentConfig {
            base_url: Some("http://127.0.0.1:1234/v1/messages".into()),
            base_url_env: Some("ARCHON_TEST_FREEZE_URL".into()),
            api_key_env: Some("ARCHON_TEST_FREEZE_KEY".into()),
            ..Default::default()
        },
    );
    unsafe {
        std::env::set_var("ARCHON_TEST_FREEZE_URL", "http://hostile.invalid");
        std::env::set_var("ARCHON_TEST_FREEZE_KEY", "fixture-key");
    }
    let environment = freeze_provider_environment(
        &config,
        &archon_core::env_vars::load_env_vars_from(&Default::default()),
    );
    unsafe {
        std::env::remove_var("ARCHON_TEST_FREEZE_URL");
        std::env::remove_var("ARCHON_TEST_FREEZE_KEY");
    }
    assert!(!environment.contains_key("ARCHON_TEST_FREEZE_URL"));
    assert_eq!(environment["ARCHON_TEST_FREEZE_KEY"], "fixture-key");
}

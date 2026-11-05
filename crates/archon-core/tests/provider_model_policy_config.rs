use archon_core::config::LlmConfig;

#[test]
fn arbitrary_provider_bindings_and_role_aliases_round_trip() {
    let config: LlmConfig = toml::from_str(
        r#"
provider = "private-service"
[role_models]
judge = "opus"
reauthor = "haiku"
[tier_aliases]
sonnet = "opus"
[model_prices]
"custom-model" = [2.0, 4.0]
[provider_environment.private-service]
base_url = "https://private.invalid/v1"
base_url_env = "PRIVATE_URL"
api_key_env = "PRIVATE_KEY"
credential_env = ["PRIVATE_TOKEN"]
"#,
    )
    .unwrap();
    assert_eq!(config.role_models["judge"], "opus");
    assert_eq!(config.role_models["reauthor"], "haiku");
    assert_eq!(config.tier_aliases["sonnet"], "opus");
    assert_eq!(config.model_prices["custom-model"], [2.0, 4.0]);
    let binding = &config.provider_environment["private-service"];
    assert_eq!(binding.base_url_env.as_deref(), Some("PRIVATE_URL"));
    assert_eq!(binding.api_key_env.as_deref(), Some("PRIVATE_KEY"));
    let restored: LlmConfig = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
    assert_eq!(
        restored.provider_environment["private-service"].credential_env,
        ["PRIVATE_TOKEN"]
    );
}

#[test]
fn existing_config_retains_provider_resolved_tier_defaults() {
    let config: LlmConfig = toml::from_str("provider = 'private-service'").unwrap();
    assert!(config.role_models.is_empty());
    assert!(config.tier_aliases.is_empty());
    assert!(config.model_prices.is_empty());
    assert!(config.provider_environment.is_empty());
}

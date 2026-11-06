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
    for (model, pricing) in archon_core::cost_table::built_in_prices() {
        assert_eq!(
            config.model_prices[model],
            [pricing.input_per_mtok, pricing.output_per_mtok],
            "{model}"
        );
    }
    assert!(config.provider_environment.is_empty());
}

#[test]
fn shipped_default_models_use_the_shared_cost_table() {
    let models = archon_core::config::ModelsConfig::default();
    let prices = LlmConfig::default().model_prices;
    for model in [
        models.anthropic.opus,
        models.anthropic.sonnet,
        models.anthropic.haiku,
    ] {
        let pricing = archon_core::cost_table::lookup(&model).unwrap();
        assert_eq!(
            prices[&model],
            [pricing.input_per_mtok, pricing.output_per_mtok],
            "{model}"
        );
    }
    for (model, pricing) in archon_core::cost_table::built_in_prices() {
        assert_eq!(
            prices[model],
            [pricing.input_per_mtok, pricing.output_per_mtok],
            "{model}"
        );
    }
}

#[test]
fn partial_model_price_overrides_keep_unlisted_defaults() {
    let config: LlmConfig = toml::from_str(
        r#"
[model_prices]
"claude-opus-5" = [7.0, 35.0]
"private-model" = [0.0, 0.0]
"#,
    )
    .unwrap();
    let mut expected = archon_core::config::model_policy::default_model_prices();
    expected.insert("claude-opus-5".into(), [7.0, 35.0]);
    expected.insert("private-model".into(), [0.0, 0.0]);
    assert_eq!(config.model_prices, expected);
    assert!(!config.model_prices.contains_key("unknown-model"));
    let empty: LlmConfig = toml::from_str("[model_prices]").unwrap();
    assert_eq!(empty.model_prices, LlmConfig::default().model_prices);
}

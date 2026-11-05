// Provider environment registry data. Endpoints come from their existing config fields.
[
    ("anthropic", ProviderEnvironmentConfig {
        base_url: config.api.base_url.clone(),
        base_url_env: Some("ANTHROPIC_BASE_URL".into()),
        api_key_env: Some("ANTHROPIC_API_KEY".into()),
        credential_env: vec!["ANTHROPIC_AUTH_TOKEN".into()],
    }),
    ("openai", ProviderEnvironmentConfig {
        base_url: config.llm.openai.base_url.clone(),
        base_url_env: Some("OPENAI_BASE_URL".into()),
        api_key_env: Some("OPENAI_API_KEY".into()),
        credential_env: vec![],
    }),
    ("local", ProviderEnvironmentConfig {
        base_url: Some(config.llm.local.base_url.clone()),
        base_url_env: None,
        api_key_env: None,
        credential_env: vec![],
    }),
    ("openai-codex", ProviderEnvironmentConfig {
        base_url: config.providers.openai_codex.app_server_url.clone(),
        base_url_env: None,
        api_key_env: Some("OPENAI_API_KEY".into()),
        credential_env: vec![],
    }),
    ("bedrock", ProviderEnvironmentConfig {
        base_url: None,
        base_url_env: None,
        api_key_env: None,
        credential_env: vec!["AWS_ACCESS_KEY_ID".into(), "AWS_SECRET_ACCESS_KEY".into(), "AWS_SESSION_TOKEN".into(), "AWS_PROFILE".into(), "AWS_SHARED_CREDENTIALS_FILE".into()],
    }),
    ("vertex", ProviderEnvironmentConfig {
        base_url: None,
        base_url_env: None,
        api_key_env: None,
        credential_env: vec!["GOOGLE_APPLICATION_CREDENTIALS".into(), "GOOGLE_CLOUD_PROJECT".into()],
    }),
]

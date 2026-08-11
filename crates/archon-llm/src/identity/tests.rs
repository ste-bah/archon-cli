use super::*;

mod identity_resolution_tests {
    use std::collections::HashMap;

    use super::*;
    use crate::auth::{AuthProvider, CodexCredentials, OAuthCredentials};
    use crate::types::Secret;

    fn oauth_auth() -> AuthProvider {
        AuthProvider::OAuthToken(OAuthCredentials {
            access_token: Secret::new("sk-ant-oat-test".to_string()),
            refresh_token: Secret::new("refresh".to_string()),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            scopes: vec!["user".to_string()],
            subscription_type: "pro".to_string(),
        })
    }

    fn codex_auth() -> AuthProvider {
        AuthProvider::CodexOAuthToken(CodexCredentials {
            access_token: Secret::new("codex-access".to_string()),
            refresh_token: Secret::new("codex-refresh".to_string()),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            account_id: "acct".to_string(),
        })
    }

    #[test]
    fn oauth_token_forces_spoof_identity() {
        let mode = resolve_identity_mode(&oauth_auth(), false, &IdentityConfigView::default());

        match mode {
            IdentityMode::Spoof { betas, .. } => assert!(
                betas.iter().any(|beta| beta == "oauth-2025-04-20"),
                "OAuth spoof mode must send the OAuth beta"
            ),
            other => panic!("OAuth auth must force spoof identity, got {other:?}"),
        }
    }

    #[test]
    fn bearer_token_forces_spoof_identity() {
        let auth = AuthProvider::BearerToken(Secret::new("sk-ant-oat-test".to_string()));
        let mode = resolve_identity_mode(&auth, false, &IdentityConfigView::default());

        assert!(
            matches!(mode, IdentityMode::Spoof { .. }),
            "Bearer auth must force spoof identity because OAuth env tokens resolve as bearer"
        );
    }

    #[test]
    fn api_key_respects_clean_identity_config() {
        let auth = AuthProvider::ApiKey(Secret::new("sk-ant-api03-test".to_string()));
        let mode = resolve_identity_mode(&auth, false, &IdentityConfigView::default());

        assert!(matches!(mode, IdentityMode::Clean));
    }

    #[test]
    fn api_key_respects_custom_identity_config() {
        let auth = AuthProvider::ApiKey(Secret::new("sk-ant-api03-test".to_string()));
        let mut extra = HashMap::new();
        extra.insert("x-extra".to_string(), "yes".to_string());
        let custom = CustomIdentityConfigView {
            user_agent: "custom-agent",
            x_app: "custom-app",
            extra_headers: Some(&extra),
        };
        let config = IdentityConfigView {
            mode: "custom",
            custom: Some(custom),
            ..IdentityConfigView::default()
        };

        let mode = resolve_identity_mode(&auth, false, &config);

        match mode {
            IdentityMode::Custom {
                user_agent,
                x_app,
                extra_headers,
            } => {
                assert_eq!(user_agent, "custom-agent");
                assert_eq!(x_app, "custom-app");
                assert_eq!(
                    extra_headers.get("x-extra").map(String::as_str),
                    Some("yes")
                );
            }
            other => panic!("custom config should produce custom identity, got {other:?}"),
        }
    }

    #[test]
    fn force_spoof_overrides_clean_config() {
        let auth = AuthProvider::ApiKey(Secret::new("sk-ant-api03-test".to_string()));
        let mode = resolve_identity_mode(&auth, true, &IdentityConfigView::default());

        assert!(matches!(mode, IdentityMode::Spoof { .. }));
    }

    #[test]
    fn codex_oauth_does_not_force_anthropic_spoof() {
        let mode = resolve_identity_mode(&codex_auth(), false, &IdentityConfigView::default());

        assert!(
            matches!(mode, IdentityMode::Clean),
            "Codex OAuth is for OpenAI Responses and must not trigger Anthropic spoofing"
        );
    }
}

mod beta_validation_cache_tests {
    use std::fs;

    use super::*;

    // Every test below drives the cache through an explicit root pointed at its
    // own `TempDir`. Nothing here touches `dirs::config_dir()`, so no two tests
    // share a file and the suite cannot overwrite the developer's real
    // `<config>/archon/validated_betas.json`.
    //
    // These tests used to be marked `#[serial_test::serial(validated_betas_cache)]`
    // instead. That annotation is inert under `cargo nextest`, which runs one
    // process per test: an in-process mutex is not shared with anything and
    // serialises nothing (see the note in `.config/nextest.toml`). The tests ran
    // concurrently against one real file, and the `remove_file` in
    // `resolve_and_validate_betas_falls_back_to_defaults` could land between the
    // save and the load below.

    #[test]
    fn test_load_cached_validated_betas_returns_none_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");

        let result = load_validated_betas_in(dir.path());

        assert_eq!(
            result, None,
            "an empty cache root must read back as None, not as a stale or partial list"
        );
    }

    #[test]
    fn test_save_and_load_validated_betas_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let betas = vec![
            "claude-code-20250219".to_string(),
            "oauth-2025-04-20".to_string(),
            "test-beta-2025-01-01".to_string(),
        ];

        save_validated_betas_in(dir.path(), &betas);
        let loaded = load_validated_betas_in(dir.path());

        assert!(loaded.is_some(), "cache should be present after saving");
        let loaded_betas = loaded.unwrap();
        assert_eq!(loaded_betas.len(), betas.len());
        for b in &betas {
            assert!(loaded_betas.contains(b), "loaded cache should contain {b}");
        }
    }

    #[test]
    fn validated_betas_cache_uses_its_own_file_under_the_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let betas = vec!["claude-code-20250219".to_string()];

        save_validated_betas_in(dir.path(), &betas);

        // Pins the wiring the zero-argument wrappers rely on: the validated list
        // lives at `<root>/validated_betas.json` and not in the discovered-beta
        // file, which is a separate cache with a separate lifetime.
        assert!(
            dir.path().join("validated_betas.json").is_file(),
            "validated betas must be written to validated_betas.json under the root"
        );
        assert!(
            !dir.path().join("discovered_betas.json").exists(),
            "saving validated betas must not touch the discovered-beta cache"
        );
    }

    #[test]
    fn beta_cache_file_is_versioned_integrity_checked_and_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("archon").join("validated_betas.json");
        let betas = vec![
            "claude-code-20250219".to_string(),
            "oauth-2025-04-20".to_string(),
        ];

        save_beta_cache_file(&path, &betas).expect("cache save succeeds");

        let raw = fs::read_to_string(&path).expect("cache JSON exists");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("cache JSON parses");
        assert_eq!(value["version"], BETA_CACHE_VERSION);
        assert_eq!(value["betas"][0], betas[0]);
        let integrity = value["integrity"]
            .as_str()
            .expect("cache carries integrity field");
        assert!(integrity.starts_with("sha256:"));
        assert_eq!(integrity.len(), "sha256:".len() + 64);
        assert_eq!(load_beta_cache_file(&path), Some(betas));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "beta cache must be written as 0600");
        }
    }

    #[test]
    fn beta_cache_load_rejects_tampered_integrity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("archon").join("discovered_betas.json");
        let betas = vec!["claude-code-20250219".to_string()];
        save_beta_cache_file(&path, &betas).expect("cache save succeeds");

        let raw = fs::read_to_string(&path).expect("cache JSON exists");
        let mut value: serde_json::Value = serde_json::from_str(&raw).expect("cache JSON parses");
        value["betas"] = serde_json::json!(["attacker-beta-2099-01-01"]);
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        assert_eq!(load_beta_cache_file(&path), None);
    }

    #[tokio::test]
    async fn test_resolve_and_validate_betas_uses_config_betas_if_provided() {
        use crate::anthropic::AnthropicClient;
        use crate::auth::AuthProvider;
        use crate::identity::{IdentityMode, IdentityProvider};

        let dir = tempfile::tempdir().expect("tempdir");
        let auth = AuthProvider::ApiKey(crate::types::Secret::new("test-key".to_string()));
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".to_string(),
            "test-device".to_string(),
            String::new(),
        );
        let client = AnthropicClient::new(auth, identity, None);

        let config_betas = vec!["explicit-beta-2025-01-01".to_string()];
        let result = resolve_and_validate_betas_in(dir.path(), &client, Some(&config_betas)).await;

        // When config_betas is non-empty, it should be returned as-is without validation
        assert_eq!(result, config_betas);
        assert!(
            !dir.path().join("validated_betas.json").exists(),
            "an explicit config override must short-circuit before touching the cache"
        );
    }

    #[tokio::test]
    async fn test_resolve_and_validate_betas_falls_back_to_defaults_when_no_discovery() {
        use crate::anthropic::AnthropicClient;
        use crate::auth::AuthProvider;
        use crate::identity::{IdentityMode, IdentityProvider};

        // A fresh temp root is already empty, so discovery is forced without
        // having to delete anyone else's cache file.
        let dir = tempfile::tempdir().expect("tempdir");

        let auth = AuthProvider::ApiKey(crate::types::Secret::new("test-key".to_string()));
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".to_string(),
            "test-device".to_string(),
            String::new(),
        );
        let client = AnthropicClient::new(auth, identity, None);

        // Pass None so it attempts discovery; if Claude Code is not installed,
        // should return DEFAULT_BETAS (possibly after a failed API probe).
        // We just verify the result is non-empty (graceful fallback).
        let result = resolve_and_validate_betas_in(dir.path(), &client, None).await;
        assert!(
            !result.is_empty(),
            "should always return at least some betas"
        );
    }
}

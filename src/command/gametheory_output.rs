/// Filter empty strings to None so they don't shadow real credentials.
#[cfg(test)]
fn non_empty(s: Option<&str>) -> Option<&str> {
    match s {
        Some("") => None,
        other => other,
    }
}

fn print_fingerprint(fp: &gametheory::GameTheoryFingerprint) {
    println!("Game-Theory Fingerprint");
    println!("=======================");
    println!("Run ID:         {}", fp.run_id);
    println!("Primary Family: {}", fp.primary_family);
    if let Some(ref classic) = fp.nearest_classic {
        println!("Nearest Classic: {}", classic);
    }
    println!();
    println!("Axes:");
    println!(
        "  Cooperation:    {:20} ({})",
        fp.cooperation.value, fp.cooperation.confidence
    );
    println!(
        "  Payoff Sum:     {:20} ({})",
        fp.payoff_sum.value, fp.payoff_sum.confidence
    );
    println!(
        "  Symmetry:       {:20} ({})",
        fp.symmetry.value, fp.symmetry.confidence
    );
    println!(
        "  Timing:         {:20} ({})",
        fp.timing.value, fp.timing.confidence
    );
    println!(
        "  Perfect Info:   {:20} ({})",
        fp.perfect_info.value, fp.perfect_info.confidence
    );
    println!(
        "  Complete Info:  {:20} ({})",
        fp.complete_info.value, fp.complete_info.confidence
    );
    println!(
        "  Cardinality:    {:20} ({})",
        fp.cardinality.value, fp.cardinality.confidence
    );
    println!(
        "  Strategy Space: {:20} ({})",
        fp.strategy_space.value, fp.strategy_space.confidence
    );
    println!(
        "  Horizon:        {:20} ({})",
        fp.horizon.value, fp.horizon.confidence
    );

    if !fp.shadow_games.is_empty() {
        println!();
        println!("Shadow Games:");
        for sg in &fp.shadow_games {
            println!("  - {}", sg);
        }
    }

    if !fp.ambiguities.is_empty() {
        println!();
        println!("Ambiguities:");
        for a in &fp.ambiguities {
            println!("  - [{}] {}", a.axis, a.note);
        }
    }

    if let Some(ref hg) = fp.hidden_game_scan {
        println!();
        println!("Hidden Game Scan: {} ({})", hg.game_name, hg.confidence);
        println!("  {}", hg.description);
    }

    println!();
}

pub(crate) fn open_db() -> Result<Arc<DbInstance>> {
    crate::command::store_paths::open_evidence_db("gametheory", &["ARCHON_GAMETHEORY_DB_PATH"])
}

fn open_db_at(cwd: &std::path::Path) -> Result<Arc<DbInstance>> {
    let path =
        crate::command::store_paths::evidence_db_path_for_dir(cwd, &["ARCHON_GAMETHEORY_DB_PATH"]);
    crate::command::store_paths::open_sqlite_db(&path, "gametheory")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_empty_filters_empty_string() {
        assert_eq!(non_empty(Some("")), None);
        assert_eq!(non_empty(Some("key")), Some("key"));
        assert_eq!(non_empty(None), None);
    }

    #[test]
    fn test_non_empty_preserves_valid_values() {
        assert_eq!(non_empty(Some("sk-ant-123")), Some("sk-ant-123"));
        assert_eq!(non_empty(Some(" ")), Some(" ")); // whitespace is NOT empty
    }

    #[test]
    fn test_tier11_requires_policy_approval() {
        let tmp = tempfile::tempdir().unwrap();
        let denied = resolve_tier11_policy_for_workspace(true, tmp.path()).unwrap_err();
        assert!(denied.to_string().contains("Tier 11 is disabled"));

        let policy_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&policy_dir).unwrap();
        std::fs::write(
            policy_dir.join("policy.toml"),
            "[policy.gametheory]\nenable_tier11 = true\n",
        )
        .unwrap();
        assert!(resolve_tier11_policy_for_workspace(true, tmp.path()).unwrap());
    }

    #[tokio::test]
    async fn gametheory_run_preflight_failure_creates_no_project_files() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ArchonConfig::default();
        let env_vars = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
        let error = handle_run(
            tmp.path(),
            "test situation",
            false,
            None,
            None,
            false,
            1.0,
            1,
            "",
            false,
            &config,
            &env_vars,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("GameTheory"));
        assert!(error.contains("archon agent sync"));
        for key in archon_pipeline::required_builtin_agent_keys(
            archon_pipeline::runner::PipelineType::GameTheory,
        ) {
            assert!(
                error.contains(&key),
                "missing key {key} absent from {error}"
            );
        }
        assert!(!tmp.path().join(".archon/archon-data.db").exists());
        assert!(!tmp.path().join(".archon").exists());
    }

    #[tokio::test]
    async fn gametheory_run_rejects_frontmatter_only_before_database_or_state_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let (key, relative) = archon_pipeline::required_builtin_agent_assets(
            archon_pipeline::runner::PipelineType::GameTheory,
        )
        .into_iter()
        .next()
        .unwrap();
        let asset = tmp.path().join(relative);
        std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
        std::fs::write(&asset, "---\nname: test\n---\n").unwrap();
        let error = handle_run(
            tmp.path(),
            "test situation",
            false,
            None,
            None,
            false,
            1.0,
            1,
            "",
            false,
            &ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains(key), "{error}");
        assert!(error.contains(relative), "{error}");
        assert!(error.contains("GameTheory"), "{error}");
        assert!(error.contains("archon agent sync"), "{error}");
        assert!(!tmp.path().join(".archon/archon-data.db").exists());
        assert!(!tmp.path().join(".pipeline-state").exists());
    }

    #[tokio::test]
    async fn gametheory_resume_rejects_frontmatter_only_before_database_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let (key, relative) = archon_pipeline::required_builtin_agent_assets(
            archon_pipeline::runner::PipelineType::GameTheory,
        )
        .into_iter()
        .next()
        .unwrap();
        let asset = tmp.path().join(relative);
        std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
        std::fs::write(&asset, "---\nname: test\n---\n").unwrap();
        let error = handle_resume_with_cwd(
            tmp.path(),
            "missing-run",
            None,
            &ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains(key), "{error}");
        assert!(error.contains(relative), "{error}");
        assert!(error.contains("GameTheory"), "{error}");
        assert!(error.contains("archon agent sync"), "{error}");
        assert!(!tmp.path().join(".archon/archon-data.db").exists());
    }

    #[test]
    fn test_gametheory_oauth_auth_forces_spoof_identity() {
        let config = ArchonConfig::default();
        let auth = AuthProvider::BearerToken(archon_llm::types::Secret::new(
            "sk-ant-oat-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());

        match mode {
            IdentityMode::Spoof { betas, .. } => {
                assert!(
                    betas.iter().any(|beta| beta == "oauth-2025-04-20"),
                    "spoof identity must include the OAuth beta"
                );
            }
            other => panic!("OAuth/Bearer auth must use spoof identity, got {other:?}"),
        }
    }

    #[test]
    fn test_gametheory_oauth_spoof_identity_emits_claude_code_headers() {
        let config = ArchonConfig::default();
        let auth = AuthProvider::BearerToken(archon_llm::types::Secret::new(
            "sk-ant-oat-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());
        let identity = IdentityProvider::new(
            mode,
            "session-test".to_string(),
            "device-test".to_string(),
            "account-test".to_string(),
        );
        let headers = identity.request_headers("request-test");

        assert_eq!(headers.get("x-app").map(String::as_str), Some("cli"));
        assert!(
            headers
                .get("User-Agent")
                .is_some_and(|value| value.starts_with("claude-cli/")),
            "spoof identity must use the Claude Code user agent"
        );
        assert!(
            headers
                .get("anthropic-beta")
                .is_some_and(|value| value.contains("oauth-2025-04-20")),
            "spoof identity must send the OAuth beta"
        );
    }

    #[test]
    fn test_gametheory_api_key_respects_clean_identity_config() {
        let config = ArchonConfig::default();
        let auth = AuthProvider::ApiKey(archon_llm::types::Secret::new(
            "sk-ant-api03-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());

        assert!(
            matches!(mode, IdentityMode::Clean),
            "API key auth should respect the default clean identity mode"
        );
    }

    #[test]
    fn test_gametheory_api_key_respects_spoof_identity_config() {
        let mut config = ArchonConfig::default();
        config.identity.mode = "spoof".to_string();
        config.identity.spoof_version = "9.9.9".to_string();
        config.identity.spoof_entrypoint = "cli".to_string();
        let auth = AuthProvider::ApiKey(archon_llm::types::Secret::new(
            "sk-ant-api03-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());

        match mode {
            IdentityMode::Spoof {
                entrypoint, betas, ..
            } => {
                assert_eq!(entrypoint, "cli");
                assert!(
                    betas.iter().any(|beta| beta == "claude-code-20250219"),
                    "spoof identity must include the Claude Code identity beta"
                );
            }
            other => panic!("spoof config should produce spoof identity, got {other:?}"),
        }
    }
}

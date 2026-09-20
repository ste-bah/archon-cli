use archon_core::{
    config::ArchonConfig,
    env_vars::{apply_env_overrides, load_env_vars_from},
};

#[test]
fn configured_cache_roots_survive_loading_and_environment_override() {
    let mut config: ArchonConfig =
        toml::from_str("[tools]\ncache_root='/configured/cache'\nscratch_root='/configured/tmp'\n")
            .unwrap();
    assert_eq!(
        serde_json::to_value(&config).unwrap()["tools"]["cache_root"],
        "/configured/cache"
    );
    let env = [
        ("ARCHON_CACHE_ROOT".into(), "/override/cache".into()),
        ("ARCHON_TMPDIR".into(), "/override/tmp".into()),
    ]
    .into();
    apply_env_overrides(&mut config, &load_env_vars_from(&env));
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(value["tools"]["cache_root"], "/override/cache");
    assert_eq!(value["tools"]["scratch_root"], "/override/tmp");
}

#[test]
fn relative_cache_roots_are_rejected() {
    for field in ["cache_root", "scratch_root"] {
        let config: ArchonConfig =
            toml::from_str(&format!("[tools]\n{field}='relative/path'\n")).unwrap();
        assert!(
            archon_core::config::validate(&config)
                .unwrap_err()
                .to_string()
                .contains(field)
        );
    }
}

#[tokio::test]
async fn child_shell_uses_configured_pool_and_scratch_without_cargo_redirect() {
    use archon_tools::{
        build_cache_lease::BuildCachePool,
        isolation::IsolationTier,
        tool::{Tool, ToolContext},
    };
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let cache = temp.path().join("cache");
    let scratch = temp.path().join("scratch");
    std::fs::create_dir(&repo).unwrap();
    for marker in ["Cargo.toml", "go.mod", "package.json"] {
        std::fs::write(repo.join(marker), "").unwrap();
    }
    archon_tools::cache_paths::configure(Some(cache.clone()), Some(scratch.clone())).unwrap();
    let pool_root = archon_tools::worktree_manager::WorktreeManager::build_cache_root();
    assert_eq!(pool_root, cache.join("build-cache"));
    let tool = archon_tools::bash::BashTool {
        isolation_tier: IsolationTier::WorktreeWithBuilds,
        build_cache_pool: Some(BuildCachePool::new(&pool_root, 1)),
        ..Default::default()
    };
    let result = tool.execute(serde_json::json!({"command":"cargo --version >/dev/null; printf '%s\\n' \"$CARGO_TARGET_DIR\" \"$GOCACHE\" \"$npm_config_cache\" \"$TMPDIR\""}),
        &ToolContext { working_dir: repo, subagent_id: Some("cache-check".into()), session_id: "cache-check".into(), ..Default::default() }).await;
    archon_tools::cache_paths::configure(None, None).unwrap();
    assert!(!result.is_error, "{}", result.content);
    for path in [
        pool_root.join("build-cache-0/cargo"),
        pool_root.join("build-cache-0/go"),
        pool_root.join("build-cache-0/node"),
        scratch,
    ] {
        assert!(
            result
                .content
                .replace('\\', "/")
                .contains(&path.to_string_lossy().replace('\\', "/")),
            "missing {} in {}",
            path.display(),
            result.content
        );
    }
}

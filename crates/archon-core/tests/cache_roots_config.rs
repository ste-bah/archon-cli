use archon_core::{config::ArchonConfig, env_vars::{apply_env_overrides, load_env_vars_from}};

#[test]
fn configured_cache_roots_survive_loading_and_environment_override() {
    let mut config: ArchonConfig = toml::from_str("[tools]\ncache_root='/configured/cache'\nscratch_root='/configured/tmp'\n").unwrap();
    assert_eq!(serde_json::to_value(&config).unwrap()["tools"]["cache_root"], "/configured/cache");
    let env = [("ARCHON_CACHE_ROOT".into(), "/override/cache".into()), ("ARCHON_TMPDIR".into(), "/override/tmp".into())].into();
    apply_env_overrides(&mut config, &load_env_vars_from(&env));
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(value["tools"]["cache_root"], "/override/cache");
    assert_eq!(value["tools"]["scratch_root"], "/override/tmp");
}

#[test]
fn relative_cache_roots_are_rejected() {
    for field in ["cache_root", "scratch_root"] {
        let config: ArchonConfig = toml::from_str(&format!("[tools]\n{field}='relative/path'\n")).unwrap();
        assert!(archon_core::config::validate(&config).unwrap_err().to_string().contains(field));
    }
}

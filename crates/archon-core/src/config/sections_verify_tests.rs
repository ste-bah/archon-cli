use super::*;
use crate::config::{ArchonConfig, load_config_from};

#[test]
fn verify_defaults_are_shipped_data_for_old_and_empty_files() {
    let defaults = ArchonConfig::default().workflow.verify;
    assert_eq!(defaults.refuse_tokens, ["--workspace", "--all"]);
    assert_eq!(defaults.timeout_secs, 900);
    assert_eq!(defaults.min_free_disk_mib, 10240);
    for text in [
        "",
        "[tools.cargo]\nbuild_jobs = 2",
        "[workflow]\nwrite_confinement = true",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        assert_eq!(load_config_from(path).unwrap().workflow.verify, defaults);
    }
}

#[test]
fn verify_layer_overrides_replace_arrays_and_keep_missing_defaults() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".archon")).unwrap();
    std::fs::write(
        dir.path().join(".archon/config.toml"),
        "[workflow.verify]\nrefuse_tokens = ['--everything']\ntimeout_secs = 7",
    )
    .unwrap();
    let config = crate::config_layers::load_layered_config(None, dir.path(), None, None).unwrap();
    assert_eq!(config.workflow.verify.refuse_tokens, ["--everything"]);
    assert_eq!(config.workflow.verify.timeout_secs, 7);
    assert_eq!(config.workflow.verify.min_free_disk_mib, 10240);
    let config: ArchonConfig = toml::from_str("[workflow.verify]\nrefuse_tokens = []").unwrap();
    assert!(config.workflow.verify.refuse_tokens.is_empty());
}

#[test]
fn verify_invalid_limits_name_the_key() {
    for token in ["", " ", "\t"] {
        let mut config = VerifyConfig::default();
        config.refuse_tokens.push(token.into());
        assert!(
            validate(&config)
                .unwrap_err()
                .to_string()
                .contains("workflow.verify.refuse_tokens")
        );
    }
    let mut config = VerifyConfig::default();
    config.timeout_secs = 0;
    assert!(
        validate(&config)
            .unwrap_err()
            .to_string()
            .contains("workflow.verify.timeout_secs")
    );
    config.timeout_secs = 1;
    config.min_free_disk_mib = 0;
    assert!(validate(&config).is_ok());
}

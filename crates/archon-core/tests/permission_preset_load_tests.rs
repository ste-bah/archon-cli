//! The load path for #200 Phase 3: an incoherent permission/sandbox pair must
//! still load, and the coherence check must have something to say about it.
//!
//! The acceptance criterion that matters most here is the negative one — every
//! config that loaded before this feature existed still loads, with the same
//! values. A coherence check that rejected anything would break running
//! installs on upgrade, which is why it warns.

use archon_core::config::{
    ArchonConfig, CUSTOM_PRESET, apply_permission_preset, derive_permission_preset,
    load_config_from, permission_coherence_warnings,
};

const INCOHERENT: &str = r#"
[permissions]
mode = "bubble"

[sandbox]
backend = "disabled"
mode = "all"
scope = "tool"
workspace_access = "scratch"
"#;

fn write(dir: &tempfile::TempDir, body: &str) -> std::path::PathBuf {
    let path = dir.path().join("config.toml");
    std::fs::write(&path, body).expect("write config");
    path
}

#[test]
fn an_incoherent_config_still_loads_with_its_values_intact() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write(&dir, INCOHERENT);

    let config = load_config_from(path).expect("an incoherent config must still load");

    assert_eq!(config.permissions.mode, "bubble");
    assert_eq!(config.sandbox.backend, "disabled");
    assert_eq!(config.sandbox.mode, "all");
    assert_eq!(config.sandbox.scope, "tool");
    assert_eq!(config.sandbox.workspace_access, "scratch");
}

#[test]
fn that_same_config_produces_a_warning_for_every_conflicting_pair() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write(&dir, INCOHERENT);
    let config = load_config_from(path).expect("load");

    let warnings = permission_coherence_warnings(&config);

    // Three inert sandbox fields plus the bubble/disabled pair.
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    for field in [
        "sandbox.mode",
        "sandbox.scope",
        "sandbox.workspace_access",
        "permissions.mode",
    ] {
        assert!(
            warnings.iter().any(|warning| warning.contains(field)),
            "no warning named {field}: {warnings:?}"
        );
    }
    assert!(
        warnings
            .iter()
            .all(|warning| warning.contains("sandbox.backend")),
        "every warning must name the field it conflicts with: {warnings:?}"
    );
    assert_eq!(derive_permission_preset(&config), CUSTOM_PRESET);
}

#[test]
fn a_config_written_from_a_preset_loads_back_as_that_preset() {
    let mut config = ArchonConfig::default();
    apply_permission_preset(&mut config, "sandboxed-throwaway").expect("apply");
    let serialized = toml::to_string_pretty(&config).expect("serialize");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = write(&dir, &serialized);
    let loaded = load_config_from(path).expect("load");

    assert_eq!(derive_permission_preset(&loaded), "sandboxed-throwaway");
    assert!(permission_coherence_warnings(&loaded).is_empty());
}

#[test]
fn the_shipped_template_loads_and_reports_a_preset_name() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("config.toml");
    let raw = std::fs::read_to_string(&path).expect("read template");
    let config: ArchonConfig = toml::from_str(&raw).expect("parse template");

    // Whatever it is, it must be nameable and must not warn — the template is
    // what every fresh install starts from.
    let name = derive_permission_preset(&config);
    assert!(!name.is_empty());
    assert!(
        permission_coherence_warnings(&config).is_empty(),
        "the shipped template must not warn on first run: {:?}",
        permission_coherence_warnings(&config)
    );
}

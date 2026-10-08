use archon_core::config_layers::{ConfigLayer, load_layered_config};

fn malformed(source: &str) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".archon")).unwrap();
    let user = root.path().join("user.toml");
    let settings = root.path().join("settings.toml");
    let path = match source {
        "user" => user.clone(),
        "settings" => settings.clone(),
        _ => root.path().join(".archon/config.toml"),
    };
    std::fs::write(
        &path,
        "[workflow.acceptance_execution]\nenvironment_allowlist=[\n",
    )
    .unwrap();
    let result = load_layered_config(
        Some(&user),
        root.path(),
        (source == "settings").then_some(settings.as_path()),
        None,
    );
    assert!(
        result.is_err(),
        "{source} policy parsing failure was discarded"
    );
    if source == "project" {
        assert!(
            load_layered_config(Some(&user), root.path(), None, Some(&[ConfigLayer::User])).is_ok()
        );
    }
}
#[test]
fn r3_layers_malformed_user() {
    malformed("user");
}
#[test]
fn r3_layers_malformed_project() {
    malformed("project");
}
#[test]
fn r3_layers_malformed_settings() {
    malformed("settings");
}

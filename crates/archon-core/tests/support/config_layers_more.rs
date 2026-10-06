use super::*;

#[test]
fn load_settings_overlay() {
    let tmp = make_temp_dir("load-settings");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let settings_file = tmp.join("override.toml");
    fs::write(
        &settings_file,
        "[api]\ndefault_model = \"claude-opus-4-7\"\n",
    )
    .unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();

    let config = load_layered_config(Some(&user_cfg), &work, Some(&settings_file), None)
        .expect("load should succeed");
    assert_eq!(config.api.default_model, "claude-opus-4-7");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_missing_layers_silently_skipped() {
    let tmp = make_temp_dir("load-missing");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-opus-4-7\"\n").unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();
    // No .archon/ directory at all

    let config = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect("missing layers should be silently skipped");
    assert_eq!(config.api.default_model, "claude-opus-4-7");
    cleanup_temp_dir(&tmp);
}

#[test]
fn load_invalid_layer_refuses_resolution() {
    let tmp = make_temp_dir("load-invalid");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-opus-4-7\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    // Write intentionally broken TOML
    fs::write(
        archon_dir.join("config.toml"),
        "this is [[[not valid toml!!!",
    )
    .unwrap();

    let error = load_layered_config(Some(&user_cfg), &work, None, None)
        .expect_err("invalid selected layers must not silently alter policy");
    assert!(error.to_string().contains("config.toml"));
    cleanup_temp_dir(&tmp);
}

// ===========================================================================
// 4. config_source tracking
// ===========================================================================

#[test]
fn source_tracks_user_origin() {
    let tmp = make_temp_dir("src-user");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-opus-4-7"
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    fs::create_dir_all(&work).unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");

    assert_eq!(
        sources.get("api.default_model"),
        Some(&ConfigLayer::User),
        "api.default_model should be attributed to user layer"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn source_tracks_override() {
    let tmp = make_temp_dir("src-override");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-7\"\n",
    )
    .unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");

    assert_eq!(
        sources.get("api.default_model"),
        Some(&ConfigLayer::Project),
        "api.default_model should be attributed to project layer"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn source_inherits_show_base() {
    let tmp = make_temp_dir("src-inherit");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-sonnet-4-6"
max_retries = 5
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-7\"\n",
    )
    .unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");

    assert_eq!(
        sources.get("api.max_retries"),
        Some(&ConfigLayer::User),
        "api.max_retries should be attributed to user (inherited, not overridden)"
    );
    assert_eq!(
        sources.get("api.default_model"),
        Some(&ConfigLayer::Project),
        "api.default_model should be attributed to project (overridden)"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn format_sources_readable() {
    let tmp = make_temp_dir("src-format");

    let user_cfg = tmp.join("config.toml");
    fs::write(
        &user_cfg,
        r#"
[api]
default_model = "claude-sonnet-4-6"
max_retries = 5
"#,
    )
    .unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-7\"\n",
    )
    .unwrap();

    let sources = ConfigSourceMap::from_layered_load(Some(&user_cfg), &work, None, None)
        .expect("source tracking should succeed");
    let output = format_sources(&sources);

    assert!(
        !output.is_empty(),
        "format_sources should produce non-empty output"
    );
    // Should contain layer names and dotted key paths
    assert!(
        output.contains("user") || output.contains("User"),
        "output should mention 'user' layer: {output}"
    );
    assert!(
        output.contains("project") || output.contains("Project"),
        "output should mention 'project' layer: {output}"
    );
    assert!(
        output.contains("api.default_model"),
        "output should contain dotted key path: {output}"
    );
    cleanup_temp_dir(&tmp);
}

// ===========================================================================
// 5. setting_sources filter
// ===========================================================================

#[test]
fn filter_user_only() {
    let tmp = make_temp_dir("filter-user");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-7\"\n",
    )
    .unwrap();
    fs::write(
        archon_dir.join("config.local.toml"),
        "[api]\ndefault_model = \"claude-haiku-3-6\"\n",
    )
    .unwrap();

    let filter = vec![ConfigLayer::User];
    let config = load_layered_config(Some(&user_cfg), &work, None, Some(&filter))
        .expect("filtered load should succeed");
    assert_eq!(
        config.api.default_model, "claude-sonnet-4-6",
        "only user layer should be loaded when filter=[User]"
    );
    cleanup_temp_dir(&tmp);
}

#[test]
fn filter_user_and_project() {
    let tmp = make_temp_dir("filter-user-proj");

    let user_cfg = tmp.join("config.toml");
    fs::write(&user_cfg, "[api]\ndefault_model = \"claude-sonnet-4-6\"\n").unwrap();

    let work = tmp.join("work");
    let archon_dir = work.join(".archon");
    fs::create_dir_all(&archon_dir).unwrap();
    fs::write(
        archon_dir.join("config.toml"),
        "[api]\ndefault_model = \"claude-opus-4-7\"\n",
    )
    .unwrap();
    fs::write(
        archon_dir.join("config.local.toml"),
        "[api]\ndefault_model = \"claude-haiku-3-6\"\n",
    )
    .unwrap();

    let filter = vec![ConfigLayer::User, ConfigLayer::Project];
    let config = load_layered_config(Some(&user_cfg), &work, None, Some(&filter))
        .expect("filtered load should succeed");
    assert_eq!(
        config.api.default_model, "claude-opus-4-7",
        "local layer should be skipped when filter=[User, Project]"
    );
    cleanup_temp_dir(&tmp);
}

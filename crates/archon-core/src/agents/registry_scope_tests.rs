// -----------------------------------------------------------------------
// G5 — plugin agent loading + priority tests
// -----------------------------------------------------------------------

/// Create a minimal plugin agent fixture at
/// `<root>/<plugin>/agents/<agent>/` with the 6-file structure. The
/// `marker` string is embedded in the INTENT body so tests can assert
/// which version (project vs user vs custom) won a priority collision.
fn create_plugin_fixture(plugins_root: &Path, plugin: &str, agent: &str, marker: &str) {
    let dir = plugins_root.join(plugin).join("agents").join(agent);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("agent.md"),
        format!("# {agent}\n\n## INTENT\n{marker}\n"),
    )
    .unwrap();
    fs::write(dir.join("behavior.md"), "").unwrap();
    fs::write(dir.join("context.md"), "").unwrap();
    fs::write(dir.join("tools.md"), "").unwrap();
    fs::write(
        dir.join("memory-keys.json"),
        r#"{"recall_queries":[],"leann_queries":[],"tags":[]}"#,
    )
    .unwrap();
    fs::write(
        dir.join("meta.json"),
        r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
    )
    .unwrap();
}

#[test]
fn registry_loads_project_plugin_agent() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &project.path().join(".archon/plugins"),
        "foo",
        "bar",
        "project-foo-bar",
    );

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));

    let agent = registry
        .resolve("foo:bar")
        .expect("foo:bar should be resolvable from project plugin");
    assert_eq!(
        agent.source,
        AgentSource::Plugin("foo".to_string()),
        "source must be Plugin(foo)"
    );
    assert!(
        agent.description.contains("project-foo-bar"),
        "description should come from project plugin fixture"
    );
}

#[test]
fn registry_user_plugin_beats_project_plugin_on_same_key() {
    // G5 priority: user plugin > project plugin (but custom beats both).
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &project.path().join(".archon/plugins"),
        "foo",
        "bar",
        "project-version",
    );
    create_plugin_fixture(
        &user.path().join(".archon/plugins"),
        "foo",
        "bar",
        "user-version",
    );

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    let agent = registry.resolve("foo:bar").expect("foo:bar must resolve");

    assert_eq!(
        agent.source,
        AgentSource::Plugin("foo".to_string()),
        "source remains Plugin(foo) — both versions belong to the same plugin name"
    );
    assert!(
        agent.description.contains("user-version"),
        "user plugin must win over project plugin on key collision; \
         got description: {:?}",
        agent.description
    );
    assert!(
        !agent.description.contains("project-version"),
        "project-version must have been overwritten by user-version"
    );
}

#[test]
fn registry_underscore_plugin_dir_skipped() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &project.path().join(".archon/plugins"),
        "_internal",
        "bar",
        "internal-bar",
    );

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    assert!(
        registry.resolve("_internal:bar").is_none(),
        "_-prefixed plugin dirs must be skipped entirely"
    );
}

// #234: Windows does not allow `:` in filenames. The fixture below
// creates `.archon/agents/custom/foo:bar` which `fs::create_dir_all`
// rejects with `Os { code: 267, kind: NotADirectory }` on Windows.
// The source comment a few lines below ("our test tmp on Linux allows
// colons in filenames") already acknowledges this constraint — gate
// the test to non-Windows platforms.
#[cfg(not(windows))]
#[test]
fn registry_custom_agent_beats_plugin_with_same_key() {
    // G5 priority: project custom > user plugin.
    // Create a plugin providing "foo:bar" in USER scope (higher of the
    // two plugin levels) and a custom agent literally named "foo:bar"
    // in PROJECT scope. Custom must still win.
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &user.path().join(".archon/plugins"),
        "foo",
        "bar",
        "plugin-version",
    );

    // Custom agent with literal type "foo:bar" (the agent dir name
    // includes the colon on platforms that allow it; our test tmp on
    // Linux allows colons in filenames).
    let custom_dir = project.path().join(".archon/agents/custom/foo:bar");
    fs::create_dir_all(&custom_dir).unwrap();
    fs::write(
        custom_dir.join("agent.md"),
        "# foo:bar\n\n## INTENT\ncustom-version\n",
    )
    .unwrap();
    fs::write(
        custom_dir.join("meta.json"),
        r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
    )
    .unwrap();

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    let agent = registry.resolve("foo:bar").expect("foo:bar must resolve");

    assert_eq!(
        agent.source,
        AgentSource::Project,
        "custom project agent must override user plugin"
    );
    assert!(
        agent.description.contains("custom-version"),
        "custom agent must win; got {:?}",
        agent.description
    );
}

#[test]
fn color_map_returns_only_colored_agents() {
    let mut registry = AgentRegistry::empty();

    let mut colored = CustomAgentDefinition::default();
    colored.agent_type = "colored".into();
    colored.color = Some("#ff0000".into());
    registry.agents.insert("colored".into(), colored);

    let mut plain = CustomAgentDefinition::default();
    plain.agent_type = "plain".into();
    registry.agents.insert("plain".into(), plain);

    let map = registry.color_map();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("colored").unwrap(), "#ff0000");
}

// -----------------------------------------------------------------------
// Flat-file agent loading tests (v0.1.11)
// -----------------------------------------------------------------------

#[test]
fn load_picks_up_flat_file_agents() {
    let project = TempDir::new().unwrap();
    let agents_dir = project.path().join(".archon/agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("foo.md"),
        "---\nname: foo\ndescription: A flat-file agent\n---\n\nFlat file body.\n",
    )
    .unwrap();

    let registry = AgentRegistry::load_with_user_home(project.path(), None);
    let agent = registry
        .resolve("foo")
        .expect("flat-file agent foo must resolve");
    assert_eq!(agent.source, AgentSource::Project);
    assert_eq!(agent.description, "A flat-file agent");
}

#[test]
fn load_combines_6file_and_flat_file() {
    let project = TempDir::new().unwrap();
    fs::create_dir_all(project.path().join(".archon/agents/custom/six-file")).unwrap();
    create_agent(&project.path().join(".archon/agents/custom"), "six-file");
    fs::write(
        project.path().join(".archon/agents/flat.md"),
        "---\nname: flat\ndescription: A flat-file agent\n---\n\nFlat body.\n",
    )
    .unwrap();

    let registry = AgentRegistry::load_with_user_home(project.path(), None);
    assert!(
        registry.resolve("six-file").is_some(),
        "6-file agent must resolve"
    );
    assert!(
        registry.resolve("flat").is_some(),
        "flat-file agent must resolve"
    );
    // 4 built-ins + 2 host + 1 custom 6-file + 1 flat-file = 8
    assert_eq!(registry.len(), 8);
}

#[test]
fn load_user_flat_file_overrides_project_flat_file() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    fs::create_dir_all(project.path().join(".archon/agents")).unwrap();
    fs::write(
        project.path().join(".archon/agents/shared.md"),
        "---\nname: shared\ndescription: project flat-file version\n---\n\nProject body.\n",
    )
    .unwrap();

    fs::create_dir_all(user.path().join(".archon/agents")).unwrap();
    fs::write(
        user.path().join(".archon/agents/shared.md"),
        "---\nname: shared\ndescription: user flat-file version\n---\n\nUser body.\n",
    )
    .unwrap();

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    let agent = registry.resolve("shared").expect("shared must resolve");
    assert_eq!(
        agent.description, "user flat-file version",
        "user flat-file must override project flat-file"
    );
    assert_eq!(agent.source, AgentSource::User);
}

#[test]
fn load_6file_overrides_flat_file_at_same_scope() {
    let project = TempDir::new().unwrap();

    // Flat-file agent named "collision"
    fs::create_dir_all(project.path().join(".archon/agents")).unwrap();
    fs::write(
        project.path().join(".archon/agents/collision.md"),
        "---\nname: collision\ndescription: flat-file version\n---\n\nFlat body.\n",
    )
    .unwrap();

    // 6-file agent also named "collision" in custom/
    create_agent(&project.path().join(".archon/agents/custom"), "collision");

    let registry = AgentRegistry::load_with_user_home(project.path(), None);
    let agent = registry
        .resolve("collision")
        .expect("collision must resolve");
    assert_eq!(
        agent.source,
        AgentSource::Project,
        "6-file format must win over flat-file at the same project scope"
    );
    assert!(
        agent.description.contains("Test agent collision"),
        "6-file agent description must appear; got {:?}",
        agent.description
    );
}

/// Lockstep regression test: flat-file agents from a real fixture are
/// loaded and counted. Without this, the two-system split (DiscoveryCatalog
/// vs AgentRegistry) can drift again.
#[test]
fn load_real_flat_file_agents_from_archon_dir() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flat_file_agents");
    let registry = AgentRegistry::load_with_user_home(&fixture, None);

    let mut names: Vec<&str> = registry
        .list()
        .iter()
        .filter(|d| matches!(d.source, AgentSource::Project))
        .map(|d| d.agent_type.as_str())
        .collect();
    names.sort();

    // Expected: a, b, c (from sub/).
    // custom/skip.md skipped by filter_entry (custom/ excluded).
    // README.md skipped (no frontmatter).
    assert_eq!(
        names,
        vec!["a", "b", "c"],
        "expected 3 flat-file project agents; got {:?}",
        names
    );
}

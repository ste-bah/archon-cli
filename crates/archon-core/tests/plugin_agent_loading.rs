//! G5 live smoke test — plugin agent loading end-to-end integration.
//!
//! Exercises `AgentRegistry::load_with_user_home` against on-disk plugin
//! fixtures created in a temp directory. Verifies:
//!
//! 1. A plugin agent under `<project>/.archon/plugins/foo/agents/bar/` is
//!    discoverable as `foo:bar` with `AgentSource::Plugin("foo")`.
//! 2. User plugin shadows project plugin on key collision (real priority).
//! 3. `_`-prefixed plugin directories are skipped.
//! 4. A custom agent with the same key wins over any plugin version.

use archon_core::agents::{AgentRegistry, AgentSource};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Create a plugin agent fixture at
/// `<plugins_root>/<plugin>/agents/<agent>/` with the 6-file structure.
fn create_plugin_fixture(plugins_root: &Path, plugin: &str, agent: &str, marker: &str) {
    let dir = plugins_root.join(plugin).join("agents").join(agent);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("agent.md"),
        format!("# {agent}\n\n## INTENT\n{marker}\n"),
    )
    .unwrap();
    fs::write(dir.join("behavior.md"), "Plugin behavior rules.\n").unwrap();
    fs::write(dir.join("context.md"), "Plugin context data.\n").unwrap();
    fs::write(
        dir.join("tools.md"),
        "# Tools\n\n## Primary Tools\n- **Read**: read files\n",
    )
    .unwrap();
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
fn smoke_plugin_agent_discoverable_via_registry() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &project.path().join(".archon/plugins"),
        "foo",
        "bar",
        "smoke-project-foo-bar",
    );

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));

    let agent = registry
        .resolve("foo:bar")
        .expect("foo:bar must be discoverable via the registry");
    assert_eq!(agent.source, AgentSource::Plugin("foo".to_string()));
    assert!(agent.description.contains("smoke-project-foo-bar"));
    assert!(!agent.system_prompt.is_empty());
    assert!(registry.load_errors().is_empty());
}

#[test]
fn smoke_user_plugin_beats_project_plugin() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &project.path().join(".archon/plugins"),
        "foo",
        "bar",
        "project-version-smoke",
    );
    create_plugin_fixture(
        &user.path().join(".archon/plugins"),
        "foo",
        "bar",
        "user-version-smoke",
    );

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    let agent = registry.resolve("foo:bar").expect("foo:bar must resolve");

    assert!(
        agent.description.contains("user-version-smoke"),
        "user plugin must win; got: {:?}",
        agent.description
    );
}

#[test]
fn smoke_underscore_plugin_skipped() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &project.path().join(".archon/plugins"),
        "_internal",
        "bar",
        "should-not-load",
    );

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    assert!(registry.resolve("_internal:bar").is_none());
}

// #234: Windows does not allow `:` in filenames. The fixture below
// creates `.archon/agents/custom/foo:bar`, which fails on Windows with
// `Os { code: 267, kind: NotADirectory }`. Source registry.rs:529-530
// acknowledges colons are only legal "on platforms that allow it" —
// gate this colon-fixture test to non-Windows platforms.
#[cfg(not(windows))]
#[test]
fn smoke_custom_beats_plugin() {
    let project = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    create_plugin_fixture(
        &user.path().join(".archon/plugins"),
        "foo",
        "bar",
        "plugin-version-smoke",
    );

    let custom_dir = project.path().join(".archon/agents/custom/foo:bar");
    fs::create_dir_all(&custom_dir).unwrap();
    fs::write(
        custom_dir.join("agent.md"),
        "# foo:bar\n\n## INTENT\ncustom-version-smoke\n",
    )
    .unwrap();
    fs::write(
        custom_dir.join("meta.json"),
        r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
    )
    .unwrap();

    let registry = AgentRegistry::load_with_user_home(project.path(), Some(user.path()));
    let agent = registry.resolve("foo:bar").expect("foo:bar must resolve");

    assert_eq!(agent.source, AgentSource::Project);
    assert!(
        agent.description.contains("custom-version-smoke"),
        "custom agent must beat plugin; got: {:?}",
        agent.description
    );
}

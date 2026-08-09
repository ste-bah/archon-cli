use std::fs;

use super::*;

fn task(id: &str, tools: &str, env: &str, tail: &str) -> String {
    format!(
        "# {id} — Thing\n\n```yaml\ntask_id: {id}\ntitle: Thing\ncomplexity: medium\n\
         status: pending\ndepends_on: []\nblocks: []\nimplements: []\n\
         required_env_keys: {env}\nrequired_tools: {tools}\ndeliverable_contracts: []\n```\n\n{tail}\n"
    )
}

fn project(files: &[String]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let tasks = dir.path().join("tasks");
    fs::create_dir_all(&tasks).expect("tasks dir");
    fs::create_dir_all(dir.path().join(".archon")).expect("archon dir");
    for (index, contents) in files.iter().enumerate() {
        let id = format!("TASK-X-{:03}", index + 1);
        fs::write(tasks.join(format!("{id}.md")), contents).expect("write");
    }
    dir
}

fn manifest(root: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(root.join(".archon/project.json")).expect("read"))
        .expect("json")
}

/// The gap this closes: a task runs cargo, declares nothing, and the manifest
/// picks it up anyway because the host reads the command.
#[test]
fn a_runner_used_but_not_declared_still_reaches_the_manifest() {
    let dir = project(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Focused Tests\n\n- `cargo test -p thing`\n",
    )]);
    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");
    assert!(sync.created);
    assert_eq!(sync.added_tools, vec!["cargo".to_string()]);
    assert_eq!(manifest(dir.path())["required_tools"], serde_json::json!(["cargo"]));
}

/// A tool a task declares but never invokes stays with that task. Hoisting it
/// would grant it to every task, defeating per-task scoping.
#[test]
fn a_declared_but_uninvoked_tool_is_not_hoisted() {
    let dir = project(&[task(
        "TASK-X-001",
        "[mcp__server__special, cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");
    assert_eq!(sync.added_tools, vec!["cargo".to_string()]);
    assert_eq!(
        manifest(dir.path())["required_tools"],
        serde_json::json!(["cargo"]),
        "only the invoked runner is project-wide"
    );
}

#[test]
fn declared_env_keys_are_carried_up() {
    let dir = project(&[task(
        "TASK-X-001",
        "[bash]",
        "[POLYGON_API_KEY]",
        "## Focused Tests\n\n- `bash -lc 'true'`\n",
    )]);
    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");
    assert_eq!(sync.added_env_keys, vec!["POLYGON_API_KEY".to_string()]);
}

/// A project accumulates PRDs. A decomposition must never strip what an earlier
/// one put there.
#[test]
fn an_existing_capability_is_never_removed() {
    let dir = project(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    fs::write(
        dir.path().join(".archon/project.json"),
        r#"{"schema_version":"archon.project.capabilities.v1",
            "required_env_keys":["OPENBB_API_URL"],
            "required_tools":["node"],"tool_bundles":{}}"#,
    )
    .expect("seed");

    sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");

    let m = manifest(dir.path());
    let tools: Vec<&str> = m["required_tools"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(tools.contains(&"node"), "kept the earlier PRD's tool: {tools:?}");
    assert!(tools.contains(&"cargo"), "added this PRD's tool: {tools:?}");
    assert_eq!(m["required_env_keys"], serde_json::json!(["OPENBB_API_URL"]));
}

/// Re-running a decomposition must not keep reporting changes it did not make.
#[test]
fn a_second_sync_is_a_no_op() {
    let dir = project(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("first");
    let again = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("second");
    assert!(again.added_tools.is_empty() && again.added_env_keys.is_empty());
    assert!(!again.created);
}

#[test]
fn dry_run_reports_without_writing() {
    let dir = project(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), true).expect("sync");
    assert_eq!(sync.added_tools, vec!["cargo".to_string()]);
    assert!(!dir.path().join(".archon/project.json").exists());
}

/// A hand-edited manifest that no longer parses must not be silently replaced.
#[test]
fn a_malformed_manifest_is_refused_not_overwritten() {
    let dir = project(&[task(
        "TASK-X-001",
        "[cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    fs::write(dir.path().join(".archon/project.json"), "{ not json").expect("seed");
    let error = sync_capabilities(dir.path(), &dir.path().join("tasks"), false)
        .expect_err("a malformed manifest must be an error");
    assert!(format!("{error:#}").contains("refusing to overwrite"), "{error:#}");
    assert_eq!(
        fs::read_to_string(dir.path().join(".archon/project.json")).expect("read"),
        "{ not json"
    );
}

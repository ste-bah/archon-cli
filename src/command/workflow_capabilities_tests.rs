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

/// #163 failure 3, at the seam that caused it. A declared tool must be
/// *exercised* for a branch to be accepted, and a task that declares any tool
/// may not declare a no-op — so a tool in the manifest, which merges into every
/// task, traps every branch. The leading runner of a focused-test command is
/// therefore no longer hoisted, and neither is anything else.
#[test]
fn a_runner_a_task_invokes_is_not_hoisted_into_the_manifest() {
    let dir = project(&[task(
        "TASK-X-001",
        "[]",
        "[]",
        "## Focused Tests\n\n- `cargo test -p thing`\n",
    )]);
    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");
    assert!(sync.created);
    assert!(sync.inert_tools.is_empty());
    assert_eq!(
        manifest(dir.path()).get("required_tools"),
        None,
        "a runner a task invokes stays with that task"
    );
}

/// A tool a task declares stays with that task. Hoisting it would grant it —
/// and its invocation obligation — to every task, defeating per-task scoping.
#[test]
fn a_declared_tool_is_not_hoisted() {
    let dir = project(&[task(
        "TASK-X-001",
        "[mcp__server__special, cargo]",
        "[]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");
    assert_eq!(manifest(dir.path()).get("required_tools"), None);
    assert_eq!(
        manifest(dir.path()).get("tool_bundles"),
        None,
        "a new manifest does not gain a key nothing merges"
    );
}

/// A manifest an older build wrote still carries tools. They are left exactly
/// where they are — a manifest only ever grows — but the sync says out loud
/// that nothing reads them, so a hand edit is not silently inert.
#[test]
fn tools_left_by_an_older_manifest_are_reported_inert_not_removed() {
    let dir = project(&[task(
        "TASK-X-001",
        "[]",
        "[POLYGON_API_KEY]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    fs::write(
        dir.path().join(".archon/project.json"),
        r#"{"schema_version":"archon.project.capabilities.v1",
            "required_env_keys":[],
            "required_tools":["bash","cargo"],
            "tool_bundles":{"lake":["python3"]}}"#,
    )
    .expect("seed");

    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("sync");
    assert_eq!(
        sync.inert_tools,
        vec![
            "bash".to_string(),
            "cargo".to_string(),
            "python3".to_string()
        ]
    );
    assert!(
        sync.render().contains("no longer merged into any task"),
        "{}",
        sync.render()
    );
    assert_eq!(
        manifest(dir.path())["required_tools"],
        serde_json::json!(["bash", "cargo"]),
        "nothing is removed from a manifest"
    );
    assert_eq!(
        manifest(dir.path())["tool_bundles"],
        serde_json::json!({ "lake": ["python3"] })
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
        "[POLYGON_API_KEY]",
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
    let keys: Vec<&str> = m["required_env_keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        keys.contains(&"OPENBB_API_URL"),
        "kept the earlier PRD's key: {keys:?}"
    );
    assert!(
        keys.contains(&"POLYGON_API_KEY"),
        "added this PRD's key: {keys:?}"
    );
    assert_eq!(m["required_tools"], serde_json::json!(["node"]));
}

/// Re-running a decomposition must not keep reporting changes it did not make.
#[test]
fn a_second_sync_is_a_no_op() {
    let dir = project(&[task(
        "TASK-X-001",
        "[cargo]",
        "[POLYGON_API_KEY]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("first");
    let again = sync_capabilities(dir.path(), &dir.path().join("tasks"), false).expect("second");
    assert!(again.added_env_keys.is_empty());
    assert!(!again.created);
}

#[test]
fn dry_run_reports_without_writing() {
    let dir = project(&[task(
        "TASK-X-001",
        "[]",
        "[POLYGON_API_KEY]",
        "## Focused Tests\n\n- `cargo test`\n",
    )]);
    let sync = sync_capabilities(dir.path(), &dir.path().join("tasks"), true).expect("sync");
    assert_eq!(sync.added_env_keys, vec!["POLYGON_API_KEY".to_string()]);
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
    assert!(
        format!("{error:#}").contains("refusing to overwrite"),
        "{error:#}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".archon/project.json")).expect("read"),
        "{ not json"
    );
}

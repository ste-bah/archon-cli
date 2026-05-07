use super::*;

#[test]
fn get_agent_memory_dir_user_scope() {
    let cwd = Path::new("/tmp/project");
    let dir = get_agent_memory_dir("code-reviewer", &AgentMemoryScope::User, cwd);
    let home = dirs::home_dir().unwrap();
    assert_eq!(dir, home.join(".archon/agent-memory/code-reviewer"));
}

#[test]
fn get_agent_memory_dir_project_scope() {
    let cwd = Path::new("/tmp/project");
    let dir = get_agent_memory_dir("code-reviewer", &AgentMemoryScope::Project, cwd);
    assert_eq!(
        dir,
        PathBuf::from("/tmp/project/.archon/agent-memory/code-reviewer")
    );
}

#[test]
fn get_agent_memory_dir_local_scope() {
    let cwd = Path::new("/tmp/project");
    let dir = get_agent_memory_dir("code-reviewer", &AgentMemoryScope::Local, cwd);
    assert_eq!(
        dir,
        PathBuf::from("/tmp/project/.archon/agent-memory-local/code-reviewer")
    );
}

#[test]
fn get_agent_memory_dir_sanitizes_colons() {
    let cwd = Path::new("/tmp/project");
    let dir = get_agent_memory_dir("plugin:my-agent", &AgentMemoryScope::Project, cwd);
    assert_eq!(
        dir,
        PathBuf::from("/tmp/project/.archon/agent-memory/plugin-my-agent")
    );
}

#[tokio::test]
async fn ensure_memory_dir_creates_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nested/agent-memory/test");
    assert!(!dir.exists());
    ensure_memory_dir_exists(&dir).await;
    assert!(dir.exists());
}

#[tokio::test]
async fn ensure_memory_dir_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("agent-memory/test");
    ensure_memory_dir_exists(&dir).await;
    ensure_memory_dir_exists(&dir).await;
    assert!(dir.exists());
}

#[test]
fn truncate_empty_returns_empty() {
    assert_eq!(truncate_entrypoint_content("", 200, 25_000), "");
    assert_eq!(truncate_entrypoint_content("   \n  ", 200, 25_000), "");
}

#[test]
fn truncate_no_truncation_needed() {
    let content = "- [Agent role](role.md) — Senior developer\n- [Prefs](prefs.md) — Uses vim";
    let result = truncate_entrypoint_content(content, 200, 25_000);
    assert_eq!(result, content);
    assert!(!result.contains("WARNING"));
}

#[test]
fn truncate_line_limit_fires() {
    let content = (0..10)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = truncate_entrypoint_content(&content, 5, 25_000);
    assert!(result.contains("WARNING"));
    assert!(result.contains("10 lines"));
    let before_warning = result.split("\n\n> WARNING").next().unwrap();
    assert_eq!(before_warning.lines().count(), 5);
}

#[test]
fn truncate_byte_limit_fires() {
    let line = "x".repeat(100);
    let content = (0..5).map(|_| line.clone()).collect::<Vec<_>>().join("\n");
    let result = truncate_entrypoint_content(&content, 200, 200);
    assert!(result.contains("WARNING"));
    assert!(result.contains("bytes"));
}

#[test]
fn truncate_both_limits_fire() {
    let content = (0..300)
        .map(|i| format!("line {i} with some extra padding text"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = truncate_entrypoint_content(&content, 200, 500);
    assert!(result.contains("WARNING"));
    assert!(result.contains("lines and"));
    assert!(result.contains("bytes"));
}

#[test]
fn prompt_contains_all_required_sections() {
    let dir = Path::new("/tmp/.archon/agent-memory/test-agent");
    let prompt = build_full_memory_prompt(dir, "Project-scoped memories.", "- [Role](role.md)");

    assert!(prompt.contains("# auto memory"), "must have header");
    assert!(
        prompt.contains("/tmp/.archon/agent-memory/test-agent"),
        "must have dir path"
    );
    assert!(
        prompt.contains("Project-scoped memories."),
        "must have scope guidance"
    );
    assert!(prompt.contains("<name>user</name>"), "must have user type");
    assert!(
        prompt.contains("<name>feedback</name>"),
        "must have feedback type"
    );
    assert!(
        prompt.contains("<name>project</name>"),
        "must have project type"
    );
    assert!(
        prompt.contains("<name>reference</name>"),
        "must have reference type"
    );
    assert!(prompt.contains("What NOT to save"), "must have exclusions");
    assert!(prompt.contains("Step 1"), "must have step 1");
    assert!(prompt.contains("Step 2"), "must have step 2");
    assert!(prompt.contains("MEMORY.md"), "must reference MEMORY.md");
    assert!(prompt.contains("name:"), "must show frontmatter");
    assert!(prompt.contains("description:"), "must show frontmatter");
    assert!(prompt.contains("type:"), "must show frontmatter");
    assert!(prompt.contains("When to access"), "must have access rules");
    assert!(
        prompt.contains("Before recommending"),
        "must have verification rules"
    );
    assert!(
        prompt.contains("- [Role](role.md)"),
        "must include existing content"
    );
}

#[test]
fn prompt_empty_memory_shows_placeholder() {
    let dir = Path::new("/tmp/test");
    let prompt = build_full_memory_prompt(dir, "test", "");
    assert!(prompt.contains("(No memories yet)"));
}

#[test]
fn load_prompt_none_scope_returns_none() {
    let result = load_agent_memory_prompt("test-agent", None, Path::new("/tmp"));
    assert!(result.is_none(), "AC-101: None scope = no memory prompt");
}

#[tokio::test]
async fn load_prompt_with_scope_returns_some() {
    let tmp = tempfile::tempdir().unwrap();
    let result =
        load_agent_memory_prompt("test-agent", Some(&AgentMemoryScope::Project), tmp.path());
    assert!(result.is_some(), "should return prompt for valid scope");
    let prompt = result.unwrap();
    assert!(prompt.contains("# auto memory"));
    assert!(prompt.contains("test-agent"));
}

#[tokio::test]
async fn load_prompt_reads_existing_memory_md() {
    let tmp = tempfile::tempdir().unwrap();
    let memory_dir = tmp.path().join(".archon/agent-memory/test-agent");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "- [Pref](pref.md) — prefers short responses",
    )
    .unwrap();

    let prompt =
        load_agent_memory_prompt("test-agent", Some(&AgentMemoryScope::Project), tmp.path())
            .unwrap();
    assert!(
        prompt.contains("prefers short responses"),
        "must include existing MEMORY.md content"
    );
}
